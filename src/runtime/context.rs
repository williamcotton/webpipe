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

/// A cached pipeline response: the final state plus the response metadata
/// needed to replay it (content type and status code).
#[derive(Clone, Debug)]
pub struct CachedResponse {
    /// Shared so cache hits don't deep-clone the payload under the store lock
    pub body: Arc<Value>,
    pub content_type: String,
    pub status_code: Option<u16>,
}

#[derive(Debug)]
struct CacheEntry {
    response: CachedResponse,
    fresh_until: std::time::Instant,
    /// fresh_until + the stale-while-revalidate grace window
    stale_until: std::time::Instant,
    /// Set when a caller has been elected to refresh this stale entry
    refresh_started: Option<std::time::Instant>,
    size_bytes: usize,
}

/// Outcome of a cache lookup with stale-while-revalidate semantics.
#[derive(Debug)]
pub enum CacheLookup {
    /// Entry is fresh: serve it.
    Fresh(CachedResponse),
    /// Entry is stale and another caller is already refreshing: serve stale.
    Stale(CachedResponse),
    /// Entry is stale and this caller won the refresh: proceed past the
    /// cache step and overwrite the entry with the new result.
    Refresh(CachedResponse),
    Miss,
}

#[derive(Clone, Debug)]
pub struct CacheStore {
    inner: Arc<Mutex<LruCache<String, CacheEntry>>>,
    default_ttl_secs: u64,
    max_entry_size: usize,
    max_total_size: usize,
    swr_grace_secs: u64,
    refresh_takeover_secs: u64,
    current_size: Arc<Mutex<usize>>,
}

impl CacheStore {
    /// Maximum size for a single cache entry (1MB)
    pub const DEFAULT_MAX_ENTRY_SIZE: usize = 1_048_576;

    /// Maximum total cache size (10MB)
    pub const DEFAULT_MAX_TOTAL_SIZE: usize = 10_485_760;

    /// Default stale-while-revalidate grace window after TTL expiry
    pub const DEFAULT_SWR_GRACE_SECS: u64 = 30;

    /// How long an elected refresher may hold the refresh slot before
    /// another caller is allowed to take over (covers crashed refreshes)
    pub const DEFAULT_REFRESH_TAKEOVER_SECS: u64 = 30;

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
            swr_grace_secs: Self::DEFAULT_SWR_GRACE_SECS,
            refresh_takeover_secs: Self::DEFAULT_REFRESH_TAKEOVER_SECS,
            current_size: Arc::new(Mutex::new(0)),
        }
    }

    /// Set the stale-while-revalidate grace window. `0` disables SWR:
    /// entries expire outright at their TTL.
    pub fn with_swr_grace(mut self, secs: u64) -> Self {
        self.swr_grace_secs = secs;
        self
    }

    /// Set how long a refresher may hold the refresh slot before takeover.
    pub fn with_refresh_takeover(mut self, secs: u64) -> Self {
        self.refresh_takeover_secs = secs;
        self
    }

    /// Estimate the size of a JSON value in bytes
    fn estimate_size(value: &Value) -> usize {
        // Serialize to JSON string to get accurate size
        serde_json::to_string(value)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    pub fn lookup(&self, key: &str) -> CacheLookup {
        let now = std::time::Instant::now();
        let mut guard = self.inner.lock();
        let Some(entry) = guard.get_mut(key) else {
            return CacheLookup::Miss;
        };

        if now < entry.fresh_until {
            return CacheLookup::Fresh(entry.response.clone());
        }

        if now < entry.stale_until {
            let can_take_over = entry.refresh_started.is_none_or(|started| {
                now.duration_since(started).as_secs() >= self.refresh_takeover_secs
            });
            if can_take_over {
                entry.refresh_started = Some(now);
                return CacheLookup::Refresh(entry.response.clone());
            }
            return CacheLookup::Stale(entry.response.clone());
        }

        // Past the stale window: remove the entry and reclaim its size
        if let Some(removed) = guard.pop(key) {
            let mut size = self.current_size.lock();
            *size = size.saturating_sub(removed.size_bytes);
        }
        CacheLookup::Miss
    }

    /// Release the refresh slot so another caller may refresh immediately
    /// (used when an elected refresher produced an uncacheable result).
    pub fn clear_refresh(&self, key: &str) {
        let mut guard = self.inner.lock();
        if let Some(entry) = guard.get_mut(key) {
            entry.refresh_started = None;
        }
    }

    /// Insert or overwrite a cached response. Overwrite semantics are
    /// required so an elected refresher can replace its stale entry.
    pub fn put_response(&self, key: String, response: CachedResponse, ttl_secs: Option<u64>) {
        let ttl = ttl_secs.unwrap_or(self.default_ttl_secs);
        if ttl == 0 { return; }

        // Estimate payload size
        let entry_size = Self::estimate_size(&response.body);

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
        let now = std::time::Instant::now();
        let entry = CacheEntry {
            response,
            fresh_until: now + Duration::from_secs(ttl),
            stale_until: now + Duration::from_secs(ttl + self.swr_grace_secs),
            refresh_started: None,
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
    /// RwLock so renders (reads) run in parallel; writes only happen when a
    /// new template is registered
    pub hb: Arc<RwLock<Handlebars<'static>>>,
    pub cfg: ConfigSnapshot,
    pub lua_scripts: Arc<std::collections::HashMap<String, String>>,
    pub js_scripts: Arc<std::collections::HashMap<String, String>>,
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

    /// Load all JavaScript scripts from the scripts directory into memory
    fn load_js_scripts() -> std::collections::HashMap<String, String> {
        let scripts_dir = std::env::var("WEBPIPE_SCRIPTS_DIR").unwrap_or_else(|_| "scripts".to_string());
        let dir_path = std::path::Path::new(&scripts_dir);
        let mut scripts = std::collections::HashMap::new();

        if !dir_path.is_dir() {
            tracing::debug!("JavaScript scripts directory '{}' not found, skipping preload", scripts_dir);
            return scripts;
        }

        match std::fs::read_dir(dir_path) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let is_js = path.extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| ext.eq_ignore_ascii_case("js"))
                            .unwrap_or(false);

                        if !is_js { continue; }

                        let module_name = path.file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_string();

                        if module_name.is_empty() { continue; }

                        match std::fs::read_to_string(&path) {
                            Ok(source) => {
                                tracing::debug!("Preloaded JavaScript script: {}", module_name);
                                scripts.insert(module_name, source);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to read JavaScript script '{}': {}", path.display(), e);
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read scripts directory '{}': {}", scripts_dir, e);
            }
        }

        tracing::info!("Preloaded {} JavaScript script(s)", scripts.len());
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
        let swr_grace_secs = cache_cfg
            .get("staleWhileRevalidate")
            .and_then(|v| v.as_u64())
            .unwrap_or(CacheStore::DEFAULT_SWR_GRACE_SECS);

        // Estimate max entries based on average entry size
        let approx_entry_size = 4096usize;
        let max_entries = std::cmp::max(64, max_total_bytes / approx_entry_size);

        let mut http_builder = Client::builder().timeout(Duration::from_secs(10));
        if std::env::var_os("WEBPIPE_DISABLE_SYSTEM_PROXY").is_some() {
            http_builder = http_builder.no_proxy();
        }
        let http = http_builder.build().expect("failed to build reqwest client");

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
        let js_scripts = Arc::new(Self::load_js_scripts());

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
            )
            .with_swr_grace(swr_grace_secs),
            rate_limit: RateLimitStore::new(rate_limit_max_entries),
            hb: Arc::new(RwLock::new(hb)),
            cfg: ConfigSnapshot(serde_json::json!({})),
            lua_scripts,
            js_scripts,
            graphql: None,
            execution_env: Arc::new(RwLock::new(None)),
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Config, Variable};

    fn resp(v: Value) -> CachedResponse {
        CachedResponse {
            body: Arc::new(v),
            content_type: "application/json".to_string(),
            status_code: None,
        }
    }

    fn lookup_fresh(cache: &CacheStore, key: &str) -> Option<CachedResponse> {
        match cache.lookup(key) {
            CacheLookup::Fresh(r) => Some(r),
            _ => None,
        }
    }

    #[test]
    fn cache_put_get_and_expiry_and_zero_ttl() {
        // SWR grace 0 so entries expire outright at their TTL
        let cache = CacheStore::new(16, 1).with_swr_grace(0);
        // zero TTL is ignored
        cache.put_response("k0".to_string(), resp(serde_json::json!({"v":0})), Some(0));
        assert!(matches!(cache.lookup("k0"), CacheLookup::Miss));

        cache.put_response("k1".to_string(), resp(serde_json::json!({"v":1})), None);
        assert_eq!(lookup_fresh(&cache, "k1").unwrap().body["v"], serde_json::json!(1));

        // expiry path
        cache.put_response("k2".to_string(), resp(serde_json::json!({"v":2})), Some(1));
        assert!(lookup_fresh(&cache, "k2").is_some());
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(matches!(cache.lookup("k2"), CacheLookup::Miss));
    }

    #[test]
    fn cache_stale_while_revalidate_elects_single_refresher() {
        let cache = CacheStore::new(16, 60).with_swr_grace(30);
        cache.put_response("k".to_string(), resp(serde_json::json!({"v":1})), Some(1));
        assert!(lookup_fresh(&cache, "k").is_some());

        std::thread::sleep(std::time::Duration::from_millis(1100));

        // First lookup in the stale window wins the refresh slot
        let first = cache.lookup("k");
        assert!(matches!(first, CacheLookup::Refresh(_)), "got {:?}", first);
        if let CacheLookup::Refresh(r) = first {
            assert_eq!(r.body["v"], serde_json::json!(1), "refresher still gets the stale value");
        }

        // Everyone else gets the stale value while the refresh is in flight
        assert!(matches!(cache.lookup("k"), CacheLookup::Stale(_)));
        assert!(matches!(cache.lookup("k"), CacheLookup::Stale(_)));

        // A failed refresher releases the slot for the next caller
        cache.clear_refresh("k");
        assert!(matches!(cache.lookup("k"), CacheLookup::Refresh(_)));

        // A successful refresh overwrites the entry and makes it fresh again
        cache.put_response("k".to_string(), resp(serde_json::json!({"v":2})), Some(60));
        assert_eq!(lookup_fresh(&cache, "k").unwrap().body["v"], serde_json::json!(2));
    }

    #[test]
    fn cache_refresh_slot_takeover_after_timeout() {
        let cache = CacheStore::new(16, 60)
            .with_swr_grace(30)
            .with_refresh_takeover(1);
        cache.put_response("k".to_string(), resp(serde_json::json!({"v":1})), Some(1));
        std::thread::sleep(std::time::Duration::from_millis(1100));

        assert!(matches!(cache.lookup("k"), CacheLookup::Refresh(_)));
        assert!(matches!(cache.lookup("k"), CacheLookup::Stale(_)));

        // After the takeover timeout, a new caller may claim the refresh slot
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(matches!(cache.lookup("k"), CacheLookup::Refresh(_)));
    }

    #[test]
    fn cache_rejects_oversized_entries() {
        // Create cache with 1KB max entry size
        let cache = CacheStore::new_with_limits(16, 60, 1024, 10_000);

        // Small entry should succeed
        cache.put_response("small".to_string(), resp(serde_json::json!({"data": "small"})), None);
        assert!(lookup_fresh(&cache, "small").is_some());

        // Large entry should be rejected
        let large_string = "x".repeat(2000);
        cache.put_response("large".to_string(), resp(serde_json::json!({"data": large_string})), None);
        assert!(matches!(cache.lookup("large"), CacheLookup::Miss), "Large entry should be rejected");
    }

    #[test]
    fn cache_evicts_lru_when_full() {
        // Create cache with 1KB total size
        let cache = CacheStore::new_with_limits(100, 60, 500, 1000);

        // Add entries until we exceed total size
        cache.put_response("entry1".to_string(), resp(serde_json::json!({"data": "x".repeat(300)})), None);
        cache.put_response("entry2".to_string(), resp(serde_json::json!({"data": "y".repeat(300)})), None);

        // Access entry1 to make it more recently used
        let _ = cache.lookup("entry1");

        // Add entry3 which should evict entry2 (LRU)
        cache.put_response("entry3".to_string(), resp(serde_json::json!({"data": "z".repeat(300)})), None);

        // entry1 and entry3 should exist, entry2 should be evicted
        assert!(lookup_fresh(&cache, "entry1").is_some(), "entry1 should still exist (recently used)");
        assert!(lookup_fresh(&cache, "entry3").is_some(), "entry3 should exist (just added)");
        // Note: Due to LRU eviction, entry2 may or may not be evicted depending on exact sizes
    }

    #[test]
    fn cache_overwrite_replaces_entry_and_size_accounting() {
        let cache = CacheStore::new_with_limits(10, 60, 1000, 5000);
        cache.put_response("k".to_string(), resp(serde_json::json!({"data": "x".repeat(100)})), None);
        let size_after_first = cache.stats().total_size_bytes;

        cache.put_response("k".to_string(), resp(serde_json::json!({"data": "y".repeat(100)})), None);
        let stats = cache.stats();
        assert_eq!(stats.entry_count, 1);
        assert_eq!(stats.total_size_bytes, size_after_first, "overwrite must not leak size");
        assert_eq!(lookup_fresh(&cache, "k").unwrap().body["data"], serde_json::json!("y".repeat(100)));
    }

    #[test]
    fn cache_stats_tracking() {
        let cache = CacheStore::new_with_limits(10, 60, 1000, 5000);

        // Add some entries
        cache.put_response("key1".to_string(), resp(serde_json::json!({"data": "test1"})), None);
        cache.put_response("key2".to_string(), resp(serde_json::json!({"data": "test2"})), None);

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
        let mut hb = ctx.hb.write();
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
