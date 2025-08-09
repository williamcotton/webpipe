use std::sync::Arc;
use std::time::Duration;

use handlebars::Handlebars;
use lru::LruCache;
use parking_lot::Mutex;
use reqwest::Client;
use serde_json::Value;
use sqlx::{postgres::PgPoolOptions, PgPool};

use crate::config;
use crate::ast::Variable;

#[derive(Clone, Debug)]
pub struct CacheEntry {
    pub payload: Value,
    pub expires_at: std::time::Instant,
}

#[derive(Clone, Debug)]
pub struct CacheStore {
    inner: Arc<Mutex<LruCache<String, CacheEntry>>>,
    default_ttl_secs: u64,
}

impl CacheStore {
    pub fn new(max_entries: usize, default_ttl_secs: u64) -> Self {
        let capacity = std::num::NonZeroUsize::new(max_entries)
            .unwrap_or_else(|| std::num::NonZeroUsize::new(1024).unwrap());
        Self {
            inner: Arc::new(Mutex::new(LruCache::new(capacity))),
            default_ttl_secs,
        }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        let mut guard = self.inner.lock();
        if let Some(entry) = guard.get(key) {
            if std::time::Instant::now() < entry.expires_at {
                return Some(entry.payload.clone());
            }
        }
        None
    }

    pub fn put(&self, key: String, payload: Value, ttl_secs: Option<u64>) {
        let ttl = ttl_secs.unwrap_or(self.default_ttl_secs);
        if ttl == 0 { return; }
        let mut guard = self.inner.lock();
        guard.put(
            key,
            CacheEntry { payload, expires_at: std::time::Instant::now() + Duration::from_secs(ttl) },
        );
    }

    pub fn default_ttl(&self) -> u64 { self.default_ttl_secs }
}

#[derive(Clone, Debug)]
pub struct ConfigSnapshot(pub serde_json::Value);

#[derive(Clone)]
pub struct Context {
    pub pg: Option<PgPool>,
    pub http: Client,
    pub cache: CacheStore,
    pub hb: Arc<Mutex<Handlebars<'static>>>,
    pub cfg: ConfigSnapshot,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("pg", &self.pg.is_some())
            .field("cache", &"CacheStore")
            .field("cfg", &"ConfigSnapshot")
            .finish()
    }
}

impl Context {
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
                "maxCacheSize": 10_485_760
            }));
        let default_ttl_secs = cache_cfg.get("defaultTtl").and_then(|v| v.as_u64()).unwrap_or(60);
        let max_bytes = cache_cfg.get("maxCacheSize").and_then(|v| v.as_u64()).unwrap_or(10_485_760);
        let approx_entry_size = 4096u64;
        let max_entries = std::cmp::max(64, (max_bytes / approx_entry_size) as usize);

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

        Ok(Self {
            pg,
            http,
            cache: CacheStore::new(max_entries, default_ttl_secs),
            hb: Arc::new(Mutex::new(hb)),
            cfg: ConfigSnapshot(serde_json::json!({})),
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;

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
}

