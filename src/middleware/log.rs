use crate::error::WebPipeError;
use crate::config;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde_json::Value;

// Monotonic epoch for high-resolution timing
static MONO_EPOCH: Lazy<std::time::Instant> = Lazy::new(std::time::Instant::now);

#[derive(Debug)]
pub struct LogMiddleware;

#[async_trait]
impl super::Middleware for LogMiddleware {
    async fn execute(&self, config: &str, input: &Value, _env: &crate::executor::ExecutionEnv) -> Result<Value, WebPipeError> {
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

        // Get global log config and merge with step-specific config  
        let cfg_mgr = config::global();
        let global_log_config = cfg_mgr.resolve_config_as_json("log").unwrap_or_else(|_| serde_json::json!({
            "enabled": true,
            "format": "json",
            "level": "debug",
            "includeBody": false,
            "includeHeaders": true,
            "maxBodySize": 1024,
            "timestamp": true
        }));

        let sc = parse_step_config(config);

        // Use step config if provided, otherwise fall back to global config
        let level = sc.level.unwrap_or_else(||
            global_log_config.get("level").and_then(|v| v.as_str()).unwrap_or("debug").to_string()
        );
        let include_body = sc.include_body.unwrap_or_else(||
            global_log_config.get("includeBody").and_then(|v| v.as_bool()).unwrap_or(false)
        );
        let include_headers = sc.include_headers.unwrap_or_else(||
            global_log_config.get("includeHeaders").and_then(|v| v.as_bool()).unwrap_or(true)
        );
        let enabled = sc.enabled.unwrap_or_else(||
            global_log_config.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true)
        );

        if !enabled {
            return Ok(input.clone());
        }

        // Capture start time
        let start_mono_us: u64 = MONO_EPOCH.elapsed().as_micros() as u64;

        // Register deferred action to log at pipeline end
        _env.defer(move |final_response| {
            let now_mono_us: u64 = MONO_EPOCH.elapsed().as_micros() as u64;
            let delta_us = now_mono_us.saturating_sub(start_mono_us);
            let duration_ms_f64 = (delta_us as f64) / 1000.0;

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
            entry.insert("level".to_string(), Value::String(level.clone()));
            entry.insert("duration_ms".to_string(), Value::Number(serde_json::Number::from_f64(duration_ms_f64).unwrap_or_else(|| serde_json::Number::from(0))));
            entry.insert("request".to_string(), Value::Object(req_obj));
            entry.insert("response".to_string(), Value::Object(resp_obj));
            if let Ok(line) = serde_json::to_string(&Value::Object(entry)) { println!("{}", line); }
        });

        Ok(input.clone())
    }

}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::Middleware;

    struct StubInvoker;
    #[async_trait::async_trait]
    impl crate::executor::MiddlewareInvoker for StubInvoker {
        async fn call(&self, _name: &str, _cfg: &str, _input: &Value, _env: &crate::executor::ExecutionEnv) -> Result<Value, WebPipeError> {
            Ok(Value::Null)
        }
    }
    fn dummy_env() -> crate::executor::ExecutionEnv {
        use crate::executor::{ExecutionEnv, AsyncTaskRegistry};
        use parking_lot::Mutex;
        use std::sync::Arc;
        use std::collections::HashMap;

        ExecutionEnv {
            variables: Arc::new(vec![]),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
            environment: None,
            async_registry: AsyncTaskRegistry::new(),
            flags: Arc::new(HashMap::new()),
            cache: None,
            deferred: Arc::new(Mutex::new(Vec::new())),
        }
    }

    #[tokio::test]
    async fn registers_deferred_logging_action() {
        // Initialize global config for test
        crate::config::init_global(vec![]);

        let mw = LogMiddleware;
        let input = serde_json::json!({
            "headers": {"x": "y"},
            "originalRequest": {"method": "GET", "params": {}, "query": {}},
        });
        let env = dummy_env();
        let out = mw.execute("level: info, includeHeaders: true, includeBody: false, enabled: true", &input, &env).await.unwrap();

        // Should return input unchanged
        assert_eq!(out, input);

        // Should have registered a deferred action
        assert_eq!(env.deferred.lock().len(), 1);

        // Run deferred to verify it doesn't panic
        env.run_deferred(&out);
    }
}

