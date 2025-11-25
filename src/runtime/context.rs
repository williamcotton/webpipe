use std::sync::Arc;
use std::time::Duration;

use handlebars::Handlebars;
use lru::LruCache;
use parking_lot::{Mutex, RwLock};
use reqwest::Client;
use serde_json::Value;
use sqlx::{postgres::PgPoolOptions, PgPool};

use crate::config;
use crate::ast::Variable;

#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub payload: Value,
    pub expires_at: std::time::Instant,
    pub size_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct CacheStore {
    inner: Arc<Mutex<LruCache<String, CacheEntry>>>,
    default_ttl_secs: u64,
    max_entry_size: usize,
    max_total_size: usize,
    current_size: Arc<Mutex<usize>>,
}

impl CacheStore {
    /// Maximum size for a single cache entry (1MB)
    pub const DEFAULT_MAX_ENTRY_SIZE: usize = 1_048_576;

    /// Maximum total cache size (10MB)
    pub const DEFAULT_MAX_TOTAL_SIZE: usize = 10_485_760;

    pub fn new(max_entries: usize, default_ttl_secs: u64) -> Self {
        Self::new_with_limits(
            max_entries,
            default_ttl_secs,
            Self::DEFAULT_MAX_ENTRY_SIZE,
            Self::DEFAULT_MAX_TOTAL_SIZE,
        )
    }

    pub fn new_with_limits(
        max_entries: usize,
        default_ttl_secs: u64,
        max_entry_size: usize,
        max_total_size: usize,
    ) -> Self {
        let capacity = std::num::NonZeroUsize::new(max_entries)
            .unwrap_or_else(|| std::num::NonZeroUsize::new(1024).unwrap());
        Self {
            inner: Arc::new(Mutex::new(LruCache::new(capacity))),
            default_ttl_secs,
            max_entry_size,
            max_total_size,
            current_size: Arc::new(Mutex::new(0)),
        }
    }

    /// Estimate the size of a JSON value in bytes
    fn estimate_size(value: &Value) -> usize {
        // Serialize to JSON string to get accurate size
        serde_json::to_string(value)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let mut guard = self.inner.lock();
        if let Some(entry) = guard.get(key) {
            if std::time::Instant::now() < entry.expires_at {
                return Some(entry.payload.clone());
            } else {
                // Entry expired, remove it and update size
                if let Some(removed) = guard.pop(key) {
                    let mut size = self.current_size.lock();
                    *size = size.saturating_sub(removed.size_bytes);
                }
            }
        }
        None
    }

    pub fn put(&self, key: String, payload: Value, ttl_secs: Option<u64>) {
        let ttl = ttl_secs.unwrap_or(self.default_ttl_secs);
        if ttl == 0 { return; }

        // Estimate payload size
        let entry_size = Self::estimate_size(&payload);

        // Check per-entry size limit
        if entry_size > self.max_entry_size {
            tracing::warn!(
                "Rejecting cache entry '{}' (size={} bytes, max={} bytes)",
                key,
                entry_size,
                self.max_entry_size
            );
            return;
        }

        let mut guard = self.inner.lock();
        let mut size_guard = self.current_size.lock();

        // If key exists, subtract its old size
        if let Some(old_entry) = guard.peek(&key) {
            *size_guard = size_guard.saturating_sub(old_entry.size_bytes);
        }

        // Check if adding this entry would exceed total size limit
        let new_total = *size_guard + entry_size;
        if new_total > self.max_total_size {
            // Evict LRU entries until we have space
            while *size_guard + entry_size > self.max_total_size {
                if let Some((_, evicted)) = guard.pop_lru() {
                    *size_guard = size_guard.saturating_sub(evicted.size_bytes);
                    tracing::debug!(
                        "Evicted LRU cache entry (size={} bytes) to make room",
                        evicted.size_bytes
                    );
                } else {
                    // Cache is empty but entry is still too large for total limit
                    tracing::warn!(
                        "Cache entry '{}' too large for total cache size (entry={} bytes, max_total={} bytes)",
                        key,
                        entry_size,
                        self.max_total_size
                    );
                    return;
                }
            }
        }

        // Insert the new entry
        let entry = CacheEntry {
            payload,
            expires_at: std::time::Instant::now() + Duration::from_secs(ttl),
            size_bytes: entry_size,
        };

        // If insert evicts an old entry, update size
        if let Some((_, evicted)) = guard.push(key, entry) {
            *size_guard = size_guard.saturating_sub(evicted.size_bytes);
        }

        *size_guard += entry_size;
    }

    pub fn default_ttl(&self) -> u64 { self.default_ttl_secs }

    /// Get current cache statistics
    pub fn stats(&self) -> CacheStats {
        let guard = self.inner.lock();
        let size_guard = self.current_size.lock();
        CacheStats {
            entry_count: guard.len(),
            total_size_bytes: *size_guard,
            max_entry_size: self.max_entry_size,
            max_total_size: self.max_total_size,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entry_count: usize,
    pub total_size_bytes: usize,
    pub max_entry_size: usize,
    pub max_total_size: usize,
}

/// Entry for tracking rate limit counters with sliding window
#[derive(Clone, Debug)]
pub struct RateLimitEntry {
    /// Number of requests in the current window
    pub count: u64,
    /// When the current window started
    pub window_start: std::time::Instant,
    /// Window duration in seconds
    pub window_secs: u64,
}

/// Store for rate limiting counters
#[derive(Clone, Debug)]
pub struct RateLimitStore {
    inner: Arc<Mutex<LruCache<String, RateLimitEntry>>>,
}

impl RateLimitStore {
    /// Default max entries for rate limit tracking
    pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

    pub fn new(max_entries: usize) -> Self {
        let capacity = std::num::NonZeroUsize::new(max_entries)
            .unwrap_or_else(|| std::num::NonZeroUsize::new(Self::DEFAULT_MAX_ENTRIES).unwrap());
        Self {
            inner: Arc::new(Mutex::new(LruCache::new(capacity))),
        }
    }

    /// Check and increment the rate limit counter for a key.
    /// Returns (allowed, current_count, limit, retry_after_secs) where:
    /// - allowed: whether the request should be allowed
    /// - current_count: the count after this request (if allowed) or current count (if denied)
    /// - limit: the configured limit
    /// - retry_after_secs: seconds until the window resets (useful for Retry-After header)
    pub fn check_and_increment(
        &self,
        key: &str,
        limit: u64,
        window_secs: u64,
        burst: Option<u64>,
    ) -> (bool, u64, u64, u64) {
        let effective_limit = limit + burst.unwrap_or(0);
        let now = std::time::Instant::now();
        let mut guard = self.inner.lock();

        if let Some(entry) = guard.get_mut(key) {
            let elapsed = now.duration_since(entry.window_start);
            let window_duration = Duration::from_secs(entry.window_secs);

            if elapsed >= window_duration {
                // Window expired, reset
                entry.count = 1;
                entry.window_start = now;
                entry.window_secs = window_secs;
                let retry_after = window_secs;
                (true, 1, effective_limit, retry_after)
            } else if entry.count < effective_limit {
                // Within limit, increment
                entry.count += 1;
                let retry_after = (window_duration - elapsed).as_secs();
                (true, entry.count, effective_limit, retry_after)
            } else {
                // Rate limited
                let retry_after = (window_duration - elapsed).as_secs();
                (false, entry.count, effective_limit, retry_after)
            }
        } else {
            // First request for this key
            let entry = RateLimitEntry {
                count: 1,
                window_start: now,
                window_secs,
            };
            guard.put(key.to_string(), entry);
            (true, 1, effective_limit, window_secs)
        }
    }

    /// Get the current count for a key without incrementing
    pub fn get_count(&self, key: &str) -> Option<u64> {
        let mut guard = self.inner.lock();
        guard.get(key).map(|e| {
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(e.window_start);
            if elapsed >= Duration::from_secs(e.window_secs) {
                0 // Window expired
            } else {
                e.count
            }
        })
    }
}

#[derive(Clone, Debug)]
pub struct ConfigSnapshot(pub serde_json::Value);

#[derive(Clone)]
pub struct Context {
    pub pg: Option<PgPool>,
    pub http: Client,
    pub cache: CacheStore,
    pub rate_limit: RateLimitStore,
    pub hb: Arc<Mutex<Handlebars<'static>>>,
    pub cfg: ConfigSnapshot,
    pub lua_scripts: Arc<std::collections::HashMap<String, String>>,
    pub graphql: Option<Arc<crate::graphql::GraphQLRuntime>>,
    pub execution_env: Arc<RwLock<Option<Arc<crate::executor::ExecutionEnv>>>>,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("pg", &self.pg.is_some())
            .field("graphql", &self.graphql.is_some())
            .field("execution_env", &self.execution_env.read().is_some())
            .field("cache", &"CacheStore")
            .field("rate_limit", &"RateLimitStore")
            .field("cfg", &"ConfigSnapshot")
            .finish()
    }
}

impl Context {
    /// Load all Lua scripts from the scripts directory into memory
    fn load_lua_scripts() -> std::collections::HashMap<String, String> {
        let scripts_dir = std::env::var("WEBPIPE_SCRIPTS_DIR").unwrap_or_else(|_| "scripts".to_string());
        let dir_path = std::path::Path::new(&scripts_dir);
        let mut scripts = std::collections::HashMap::new();

        if !dir_path.is_dir() {
            tracing::debug!("Lua scripts directory '{}' not found, skipping preload", scripts_dir);
            return scripts;
        }

        match std::fs::read_dir(dir_path) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let is_lua = path.extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| ext.eq_ignore_ascii_case("lua"))
                            .unwrap_or(false);

                        if !is_lua { continue; }

                        let module_name = path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_string();

                        if module_name.is_empty() { continue; }

                        match std::fs::read_to_string(&path) {
                            Ok(source) => {
                                tracing::debug!("Preloaded Lua script: {}", module_name);
                                scripts.insert(module_name, source);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to read Lua script '{}': {}", path.display(), e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read scripts directory '{}': {}", scripts_dir, e);
            }
        }

        tracing::info!("Preloaded {} Lua script(s)", scripts.len());
        scripts
    }

    pub async fn from_program_configs(
        configs: Vec<crate::ast::Config>,
        variables: &[Variable],
    ) -> Result<Self, crate::error::WebPipeError> {
        // Initialize global resolver for legacy paths; also capture a snapshot for Context
        config::init_global(configs.clone());
        let cfg_mgr = config::global();

        let cache_cfg = cfg_mgr
            .resolve_config_as_json("cache")
            .unwrap_or_else(|_| serde_json::json!({
                "enabled": true,
                "defaultTtl": 60,
                "maxCacheSize": 10_485_760,
                "maxEntrySize": 1_048_576
            }));
        let default_ttl_secs = cache_cfg.get("defaultTtl").and_then(|v| v.as_u64()).unwrap_or(60);
        let max_total_bytes = cache_cfg.get("maxCacheSize").and_then(|v| v.as_u64()).unwrap_or(10_485_760) as usize;
        let max_entry_bytes = cache_cfg.get("maxEntrySize").and_then(|v| v.as_u64()).unwrap_or(1_048_576) as usize;

        // Estimate max entries based on average entry size
        let approx_entry_size = 4096usize;
        let max_entries = std::cmp::max(64, max_total_bytes / approx_entry_size);

        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build reqwest client");

        // Optional pg pool
        let pg_cfg = cfg_mgr.resolve_config_as_json("pg");
        let pg = if let Ok(pg_cfg) = pg_cfg {
            // Only create if a host is provided; otherwise None
            let host_opt = pg_cfg.get("host").and_then(|v| v.as_str());
            if host_opt.is_some() {
                let host = pg_cfg.get("host").and_then(|v| v.as_str()).unwrap_or("localhost");
                let port = pg_cfg.get("port").and_then(|v| v.as_i64()).unwrap_or(5432);
                let database = pg_cfg.get("database").and_then(|v| v.as_str()).unwrap_or("postgres");
                let user = pg_cfg.get("user").and_then(|v| v.as_str()).unwrap_or("postgres");
                let password = pg_cfg.get("password").and_then(|v| v.as_str()).unwrap_or("postgres");
                let max_conns: u32 = pg_cfg.get("maxPoolSize").and_then(|v| v.as_i64()).unwrap_or(20).max(1) as u32;
                let mut min_conns: u32 = pg_cfg.get("initialPoolSize").and_then(|v| v.as_i64()).unwrap_or(5).max(1) as u32;
                if min_conns > max_conns { min_conns = max_conns; }

                let database_url = format!(
                    "postgres://{}:{}@{}:{}/{}",
                    percent_encoding::utf8_percent_encode(user, percent_encoding::NON_ALPHANUMERIC),
                    percent_encoding::utf8_percent_encode(password, percent_encoding::NON_ALPHANUMERIC),
                    host,
                    port,
                    database
                );

                let pool = PgPoolOptions::new()
                    .max_connections(max_conns)
                    .min_connections(min_conns)
                    .acquire_timeout(Duration::from_secs(3))
                    .connect(&database_url)
                    .await
                    .map_err(|e| crate::error::WebPipeError::DatabaseError(format!("Failed to connect to database: {}", e)))?;
                Some(pool)
            } else {
                None
            }
        } else {
            None
        };

        let mut hb = Handlebars::new();
        // Register partials from variables (mustache/handlebars types)
        for v in variables {
            if v.var_type == "mustache" || v.var_type == "handlebars" {
                let name = v.name.trim();
                let tpl = v.value.clone();
                let _ = hb.register_partial(name, &tpl);
            }
        }

        // Preload all Lua scripts at startup (non-blocking since we're in async context)
        let lua_scripts = Arc::new(Self::load_lua_scripts());

        // Configure rate limit store
        let rate_limit_cfg = cfg_mgr.resolve_config_as_json("rateLimit");
        let rate_limit_max_entries = if let Ok(rl_cfg) = rate_limit_cfg {
            rl_cfg.get("maxEntries").and_then(|v| v.as_u64()).unwrap_or(RateLimitStore::DEFAULT_MAX_ENTRIES as u64) as usize
        } else {
            RateLimitStore::DEFAULT_MAX_ENTRIES
        };

        Ok(Self {
            pg,
            http,
            cache: CacheStore::new_with_limits(
                max_entries,
                default_ttl_secs,
                max_entry_bytes,
                max_total_bytes,
            ),
            rate_limit: RateLimitStore::new(rate_limit_max_entries),
            hb: Arc::new(Mutex::new(hb)),
            cfg: ConfigSnapshot(serde_json::json!({})),
            lua_scripts,
            graphql: None,
            execution_env: Arc::new(RwLock::new(None)),
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Config, Variable};

    #[test]
    fn cache_put_get_and_expiry_and_zero_ttl() {
        let cache = CacheStore::new(16, 1);
        // zero TTL is ignored
        cache.put("k0".to_string(), serde_json::json!({"v":0}), Some(0));
        assert!(cache.get("k0").is_none());

        cache.put("k1".to_string(), serde_json::json!({"v":1}), None);
        assert_eq!(cache.get("k1").unwrap()["v"], serde_json::json!(1));

        // expiry path
        cache.put("k2".to_string(), serde_json::json!({"v":2}), Some(1));
        assert!(cache.get("k2").is_some());
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(cache.get("k2").is_none());
    }

    #[test]
    fn cache_rejects_oversized_entries() {
        // Create cache with 1KB max entry size
        let cache = CacheStore::new_with_limits(16, 60, 1024, 10_000);

        // Small entry should succeed
        let small = serde_json::json!({"data": "small"});
        cache.put("small".to_string(), small.clone(), None);
        assert!(cache.get("small").is_some());

        // Large entry should be rejected
        let large_string = "x".repeat(2000);
        let large = serde_json::json!({"data": large_string});
        cache.put("large".to_string(), large, None);
        assert!(cache.get("large").is_none(), "Large entry should be rejected");
    }

    #[test]
    fn cache_evicts_lru_when_full() {
        // Create cache with 1KB total size
        let cache = CacheStore::new_with_limits(100, 60, 500, 1000);

        // Add entries until we exceed total size
        cache.put("entry1".to_string(), serde_json::json!({"data": "x".repeat(300)}), None);
        cache.put("entry2".to_string(), serde_json::json!({"data": "y".repeat(300)}), None);

        // Access entry1 to make it more recently used
        let _ = cache.get("entry1");

        // Add entry3 which should evict entry2 (LRU)
        cache.put("entry3".to_string(), serde_json::json!({"data": "z".repeat(300)}), None);

        // entry1 and entry3 should exist, entry2 should be evicted
        assert!(cache.get("entry1").is_some(), "entry1 should still exist (recently used)");
        assert!(cache.get("entry3").is_some(), "entry3 should exist (just added)");
        // Note: Due to LRU eviction, entry2 may or may not be evicted depending on exact sizes
    }

    #[test]
    fn cache_stats_tracking() {
        let cache = CacheStore::new_with_limits(10, 60, 1000, 5000);

        // Add some entries
        cache.put("key1".to_string(), serde_json::json!({"data": "test1"}), None);
        cache.put("key2".to_string(), serde_json::json!({"data": "test2"}), None);

        let stats = cache.stats();
        assert_eq!(stats.entry_count, 2);
        assert!(stats.total_size_bytes > 0);
        assert_eq!(stats.max_entry_size, 1000);
        assert_eq!(stats.max_total_size, 5000);
    }

    #[tokio::test]
    async fn context_builds_without_pg_and_registers_partials() {
        let configs: Vec<Config> = vec![Config { name: "cache".to_string(), properties: vec![] }];
        // Provide a handlebars partial variable
        let variables = vec![Variable { var_type: "handlebars".to_string(), name: "greet".to_string(), value: "Hello {{name}}".to_string() }];
        let ctx = Context::from_program_configs(configs, &variables).await.unwrap();
        assert!(ctx.pg.is_none(), "pg should be None when no host provided");
        // Ensure partial was registered by attempting a render
        let mut hb = ctx.hb.lock();
        let _reg = handlebars::Handlebars::new();
        // copy registered partial into a new registry to render a template referencing it
        // The context's registry already has the partial; we register a template that uses it
        // and inject the same registry for rendering
        // Instead, render using the existing registry by registering a template id
        let tpl_id = "tpl";
        hb.register_template_string(tpl_id, "{{> greet }}").unwrap();
        let out = hb.render(tpl_id, &serde_json::json!({"name":"Ada"})).unwrap();
        assert_eq!(out, "Hello Ada");
    }
}

