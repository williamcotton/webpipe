use crate::error::WebPipeError;
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

        let parsed = parse_cfg(config);
        let mut out = input.clone();
        if let Some(obj) = out.as_object_mut() {
            let meta_entry = obj.entry("_metadata").or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !meta_entry.is_object() { *meta_entry = Value::Object(serde_json::Map::new()); }
            if let Some(meta) = meta_entry.as_object_mut() {
                let cache_entry = meta.entry("cache").or_insert_with(|| Value::Object(serde_json::Map::new()));
                if !cache_entry.is_object() { *cache_entry = Value::Object(serde_json::Map::new()); }
                if let Some(cache_map) = cache_entry.as_object_mut() {
                    if let Some(b) = parsed.enabled { cache_map.insert("enabled".to_string(), Value::Bool(b)); }
                    if let Some(ttl) = parsed.ttl { cache_map.insert("ttl".to_string(), Value::Number(serde_json::Number::from(ttl))); }
                    if let Some(tpl) = parsed.key_tpl { cache_map.insert("keyTemplate".to_string(), Value::String(tpl)); }
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
        let mw = CacheMiddleware;
        let input = serde_json::json!({});
        let out = mw.execute("enabled: true, ttl: 5, keyTemplate: id-{params.id}", &input).await.unwrap();
        assert_eq!(out["_metadata"]["cache"]["enabled"], serde_json::json!(true));
        assert_eq!(out["_metadata"]["cache"]["ttl"], serde_json::json!(5));
        assert_eq!(out["_metadata"]["cache"]["keyTemplate"], serde_json::json!("id-{params.id}"));
    }
}

