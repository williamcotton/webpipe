use crate::error::WebPipeError;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::Value;

// Monotonic epoch for high-resolution timing
static MONO_EPOCH: Lazy<std::time::Instant> = Lazy::new(std::time::Instant::now);

#[derive(Debug)]
pub struct LogMiddleware;

#[async_trait]
impl super::Middleware for LogMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        #[derive(Default)]
        struct StepCfg { level: Option<String>, include_body: Option<bool>, include_headers: Option<bool>, enabled: Option<bool> }
        fn parse_bool(s: &str) -> Option<bool> { match s.trim().to_ascii_lowercase().as_str() { "true" => Some(true), "false" => Some(false), _ => None } }
        fn parse_step_config(cfg: &str) -> StepCfg {
            let mut sc = StepCfg::default(); let normalized = cfg.replace('\n', ",");
            for part in normalized.split(',') {
                let p = part.trim(); if p.is_empty() { continue; }
                if let Some((k, v)) = p.split_once(':') {
                    let key = k.trim(); let val = v.trim();
                    match key {
                        "level" => sc.level = Some(val.to_string()),
                        "includeBody" => sc.include_body = parse_bool(val),
                        "includeHeaders" => sc.include_headers = parse_bool(val),
                        "enabled" => sc.enabled = parse_bool(val),
                        _ => {}
                    }
                }
            }
            sc
        }

        let sc = parse_step_config(config);

        let mut out = input.clone();
        if let Some(obj) = out.as_object_mut() {
            let meta_entry = obj.entry("_metadata").or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !meta_entry.is_object() { *meta_entry = Value::Object(serde_json::Map::new()); }
            if let Some(meta) = meta_entry.as_object_mut() {
                let mut log_map = serde_json::Map::new();
                let start_ms = { use std::time::{SystemTime, UNIX_EPOCH}; let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default(); now.as_millis() as u64 };
                log_map.insert("startTimeMs".to_string(), Value::Number(serde_json::Number::from(start_ms)));
                let start_mono_us: u64 = MONO_EPOCH.elapsed().as_micros() as u64;
                log_map.insert("startMonoUs".to_string(), Value::Number(serde_json::Number::from(start_mono_us)));
                if let Some(level) = sc.level { log_map.insert("level".to_string(), Value::String(level)); }
                if let Some(b) = sc.include_body { log_map.insert("includeBody".to_string(), Value::Bool(b)); }
                if let Some(b) = sc.include_headers { log_map.insert("includeHeaders".to_string(), Value::Bool(b)); }
                if let Some(b) = sc.enabled { log_map.insert("enabled".to_string(), Value::Bool(b)); } else { log_map.insert("enabled".to_string(), Value::Bool(true)); }
                meta.insert("log".to_string(), Value::Object(log_map));
            }
        }
        Ok(out)
    }

    async fn post_execute(&self, final_response: &Value) -> Result<(), WebPipeError> {
        let meta = final_response.get("_metadata").and_then(|m| m.get("log")).and_then(|l| l.as_object()).ok_or_else(|| WebPipeError::InternalError("missing log metadata".to_string()))?;
        if let Some(Value::Bool(false)) = meta.get("enabled") { return Ok(()); }

        let include_body = meta.get("includeBody").and_then(|v| v.as_bool()).unwrap_or(false);
        let include_headers = meta.get("includeHeaders").and_then(|v| v.as_bool()).unwrap_or(true);
        let level = meta.get("level").and_then(|v| v.as_str()).unwrap_or("info");

        let start_mono_us = meta.get("startMonoUs").and_then(|v| v.as_u64()).unwrap_or(0);
        let duration_ms_f64 = if start_mono_us > 0 {
            let now_mono_us: u64 = MONO_EPOCH.elapsed().as_micros() as u64;
            let delta_us = now_mono_us.saturating_sub(start_mono_us);
            (delta_us as f64) / 1000.0
        } else {
            let start_ms = meta.get("startTimeMs").and_then(|v| v.as_u64()).unwrap_or(0);
            let now_ms = { use std::time::{SystemTime, UNIX_EPOCH}; let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default(); now.as_millis() as u64 };
            (now_ms.saturating_sub(start_ms)) as f64
        };

        let mut req_obj = serde_json::Map::new();
        if let Some(orig) = final_response.get("originalRequest").and_then(|v| v.as_object()) {
            if let Some(m) = orig.get("method").cloned() { req_obj.insert("method".to_string(), m); }
            if let Some(p) = orig.get("params").cloned() { req_obj.insert("params".to_string(), p); }
            if let Some(q) = orig.get("query").cloned() { req_obj.insert("query".to_string(), q); }
        }
        if include_headers { if let Some(h) = final_response.get("headers").cloned() { req_obj.insert("headers".to_string(), h); } }

        let mut resp_obj = serde_json::Map::new();
        if include_body {
            let mut clean = final_response.clone();
            if let Some(obj) = clean.as_object_mut() { obj.remove("_metadata"); obj.remove("originalRequest"); obj.remove("setCookies"); }
            resp_obj.insert("body".to_string(), clean);
        }

        let mut entry = serde_json::Map::new();
        entry.insert("level".to_string(), Value::String(level.to_string()));
        entry.insert("duration_ms".to_string(), Value::Number(serde_json::Number::from_f64(duration_ms_f64).unwrap_or_else(|| serde_json::Number::from(0))));
        entry.insert("request".to_string(), Value::Object(req_obj));
        entry.insert("response".to_string(), Value::Object(resp_obj));
        if let Ok(line) = serde_json::to_string(&Value::Object(entry)) { println!("{}", line); }

        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::Middleware;

    #[tokio::test]
    async fn pre_and_post_execution_metadata_and_duration() {
        let mw = LogMiddleware;
        let input = serde_json::json!({
            "headers": {"x": "y"},
            "originalRequest": {"method": "GET", "params": {}, "query": {}},
        });
        let out = mw.execute("level: info, includeHeaders: true, includeBody: false, enabled: true", &input).await.unwrap();
        assert!(out.get("_metadata").is_some());
        mw.post_execute(&out).await.unwrap();
    }
}

