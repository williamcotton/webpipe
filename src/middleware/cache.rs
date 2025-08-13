use crate::error::WebPipeError;
use crate::config;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug)]
pub struct CacheMiddleware;

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
        let mut out = input.clone();
        if let Some(obj) = out.as_object_mut() {
            let meta_entry = obj.entry("_metadata").or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !meta_entry.is_object() { *meta_entry = Value::Object(serde_json::Map::new()); }
            if let Some(meta) = meta_entry.as_object_mut() {
                let cache_entry = meta.entry("cache").or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !cache_entry.is_object() { *cache_entry = Value::Object(serde_json::Map::new()); }
                if let Some(cache_map) = cache_entry.as_object_mut() {
                    // Use step config if provided, otherwise fall back to global config
                    let enabled = parsed.enabled.unwrap_or_else(|| 
                        global_cache_config.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true)
                    );
                    let ttl = parsed.ttl.unwrap_or_else(|| 
                        global_cache_config.get("defaultTtl").and_then(|v| v.as_u64()).unwrap_or(60)
                    );
                    
                    cache_map.insert("enabled".to_string(), Value::Bool(enabled));
                    cache_map.insert("ttl".to_string(), Value::Number(serde_json::Number::from(ttl)));
                    if let Some(tpl) = parsed.key_tpl { 
                        cache_map.insert("keyTemplate".to_string(), Value::String(tpl)); 
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

    #[tokio::test]
    async fn merges_cache_flags_into_metadata() {
        // Initialize global config for test
        crate::config::init_global(vec![]);
        
        let mw = CacheMiddleware;
        let input = serde_json::json!({});
        let out = mw.execute("enabled: true, ttl: 5, keyTemplate: id-{params.id}", &input).await.unwrap();
        assert_eq!(out["_metadata"]["cache"]["enabled"], serde_json::json!(true));
        assert_eq!(out["_metadata"]["cache"]["ttl"], serde_json::json!(5));
        assert_eq!(out["_metadata"]["cache"]["keyTemplate"], serde_json::json!("id-{params.id}"));
    }
}

