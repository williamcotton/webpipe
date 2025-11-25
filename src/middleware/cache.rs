use crate::error::WebPipeError;
use crate::config;
use crate::runtime::Context;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Debug)]
pub struct CacheMiddleware {
    pub ctx: Arc<Context>,
}

/// Look up a path in a JSON value and return as string
fn lookup_path_string(input: &Value, path: &str) -> String {
    let mut cur = input;
    for seg in path.split('.') {
        if seg.is_empty() { continue; }
        match cur {
            Value::Object(map) => { cur = map.get(seg).unwrap_or(&Value::Null); }
            _ => { cur = &Value::Null; }
        }
    }
    match cur {
        Value::Null => "".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Render a key template with placeholders like {path.to.value}
fn render_key_template(template: &str, input: &Value) -> String {
    let mut out = String::with_capacity(template.len());
    let mut i = 0;
    let chars: Vec<char> = template.chars().collect();
    while i < chars.len() {
        if chars[i] == '{' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '}' { j += 1; }
            if j < chars.len() && chars[j] == '}' {
                let key: String = chars[i+1..j].iter().collect();
                let val = lookup_path_string(input, key.trim());
                out.push_str(&val);
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[async_trait]
impl super::Middleware for CacheMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        #[derive(Default)]
        struct LocalCfg { enabled: Option<bool>, ttl: Option<u64>, key_tpl: Option<String> }
        fn parse_bool(s: &str) -> Option<bool> { match s.trim().to_ascii_lowercase().as_str() { "true" => Some(true), "false" => Some(false), _ => None } }
        fn parse_cfg(cfg: &str) -> LocalCfg {
            let mut lc = LocalCfg::default();
            for part in cfg.replace('\n', ",").split(',') {
                let p = part.trim(); if p.is_empty() { continue; }
                if let Some((k,v)) = p.split_once(':') {
                    let key = k.trim(); let val = v.trim();
                    match key {
                        "enabled" => lc.enabled = parse_bool(val),
                        "ttl" => lc.ttl = val.parse::<u64>().ok(),
                        "keyTemplate" => lc.key_tpl = Some(val.to_string()),
                        _ => {}
                    }
                }
            }
            lc
        }

        // Get global cache config and merge with step-specific config
        let cfg_mgr = config::global();
        let global_cache_config = cfg_mgr.resolve_config_as_json("cache").unwrap_or_else(|_| serde_json::json!({
            "enabled": true,
            "defaultTtl": 60,
            "maxCacheSize": 10485760
        }));

        let parsed = parse_cfg(config);
        
        // Determine effective settings
        let enabled = parsed.enabled.unwrap_or_else(|| 
            global_cache_config.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true)
        );
        let ttl = parsed.ttl.unwrap_or_else(|| 
            global_cache_config.get("defaultTtl").and_then(|v| v.as_u64()).unwrap_or(60)
        );

        // If caching is disabled, pass through with metadata for backward compatibility
        if !enabled {
            let mut out = input.clone();
            if let Some(obj) = out.as_object_mut() {
                let meta_entry = obj.entry("_metadata").or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !meta_entry.is_object() { *meta_entry = Value::Object(serde_json::Map::new()); }
                if let Some(meta) = meta_entry.as_object_mut() {
                    let cache_entry = meta.entry("cache").or_insert_with(|| Value::Object(serde_json::Map::new()));
                    if let Some(cache_map) = cache_entry.as_object_mut() {
                        cache_map.insert("enabled".to_string(), Value::Bool(false));
                    }
                }
            }
            return Ok(out);
        }

        // Generate cache key
        let key = if let Some(tpl) = &parsed.key_tpl {
            render_key_template(tpl, input)
        } else {
            // Generate a hash-based key from the config and relevant input parts
            let mut hasher = DefaultHasher::new();
            config.hash(&mut hasher);
            // Hash method and path if present (for route-level caching)
            if let Some(method) = input.get("method").and_then(|v| v.as_str()) {
                method.hash(&mut hasher);
            }
            if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                path.hash(&mut hasher);
            }
            // Hash query params if present
            if let Some(query) = input.get("query") {
                if let Ok(s) = serde_json::to_string(query) {
                    s.hash(&mut hasher);
                }
            }
            // Hash params if present
            if let Some(params) = input.get("params") {
                if let Ok(s) = serde_json::to_string(params) {
                    s.hash(&mut hasher);
                }
            }
            format!("pipeline:{:x}", hasher.finish())
        };

        // Check cache for a hit
        if let Some(cached_val) = self.ctx.cache.get(&key) {
            // CACHE HIT: Return wrapper with stop signal
            return Ok(serde_json::json!({
                "_control": {
                    "stop": true,
                    "value": cached_val
                }
            }));
        }

        // CACHE MISS: Return input with cache_control metadata for executor
        let mut out = input.clone();
        if let Some(obj) = out.as_object_mut() {
            let meta_entry = obj.entry("_metadata").or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !meta_entry.is_object() { *meta_entry = Value::Object(serde_json::Map::new()); }
            if let Some(meta) = meta_entry.as_object_mut() {
                // Add cache_control for the Executor to use at pipeline end
                meta.insert("cache_control".to_string(), serde_json::json!({
                    "key": key,
                    "ttl": ttl
                }));
                
                // Keep "cache" metadata for backward compatibility (e.g. fetch middleware)
                let cache_entry = meta.entry("cache").or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !cache_entry.is_object() { *cache_entry = Value::Object(serde_json::Map::new()); }
                if let Some(cache_map) = cache_entry.as_object_mut() {
                    cache_map.insert("enabled".to_string(), Value::Bool(enabled));
                    cache_map.insert("ttl".to_string(), Value::Number(serde_json::Number::from(ttl)));
                    if let Some(tpl) = &parsed.key_tpl { 
                        cache_map.insert("keyTemplate".to_string(), Value::String(tpl.clone())); 
                    }
                }
            }
        }
        Ok(out)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::Middleware;
    use crate::runtime::context::{CacheStore, ConfigSnapshot, RateLimitStore};
    use reqwest::Client;

    fn test_ctx() -> Arc<Context> {
        Arc::new(Context {
            pg: None,
            http: Client::new(),
            cache: CacheStore::new(64, 60),
            rate_limit: RateLimitStore::new(1000),
            hb: Arc::new(parking_lot::Mutex::new(handlebars::Handlebars::new())),
            cfg: ConfigSnapshot(serde_json::json!({})),
            lua_scripts: Arc::new(std::collections::HashMap::new()),
            graphql: None,
            execution_env: Arc::new(parking_lot::RwLock::new(None)),
        })
    }

    #[tokio::test]
    async fn merges_cache_flags_into_metadata() {
        // Initialize global config for test
        crate::config::init_global(vec![]);
        
        let mw = CacheMiddleware { ctx: test_ctx() };
        let input = serde_json::json!({});
        let out = mw.execute("enabled: true, ttl: 5, keyTemplate: id-{params.id}", &input).await.unwrap();
        assert_eq!(out["_metadata"]["cache"]["enabled"], serde_json::json!(true));
        assert_eq!(out["_metadata"]["cache"]["ttl"], serde_json::json!(5));
        assert_eq!(out["_metadata"]["cache"]["keyTemplate"], serde_json::json!("id-{params.id}"));
        // Should also have cache_control for executor
        assert!(out["_metadata"]["cache_control"]["key"].is_string());
        assert_eq!(out["_metadata"]["cache_control"]["ttl"], serde_json::json!(5));
    }

    #[tokio::test]
    async fn returns_stop_signal_on_cache_hit() {
        crate::config::init_global(vec![]);
        
        let ctx = test_ctx();
        // Pre-populate cache
        ctx.cache.put("test-key".to_string(), serde_json::json!({"cached": "data"}), Some(60));
        
        let mw = CacheMiddleware { ctx };
        let input = serde_json::json!({});
        let out = mw.execute("keyTemplate: test-key", &input).await.unwrap();
        
        // Should return stop signal with cached value
        assert_eq!(out["_control"]["stop"], serde_json::json!(true));
        assert_eq!(out["_control"]["value"]["cached"], serde_json::json!("data"));
    }

    #[tokio::test]
    async fn cache_disabled_passes_through() {
        crate::config::init_global(vec![]);
        
        let mw = CacheMiddleware { ctx: test_ctx() };
        let input = serde_json::json!({"original": "data"});
        let out = mw.execute("enabled: false", &input).await.unwrap();
        
        // Should pass through original data
        assert_eq!(out["original"], serde_json::json!("data"));
        // Should mark cache as disabled
        assert_eq!(out["_metadata"]["cache"]["enabled"], serde_json::json!(false));
        // Should NOT have cache_control (not tracking for caching)
        assert!(out["_metadata"].get("cache_control").is_none());
    }
}

