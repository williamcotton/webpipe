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
    async fn execute(
        &self,
        _args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &crate::executor::ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<(), WebPipeError> {
        let input = &pipeline_ctx.state;
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

        // If caching is disabled, state unchanged
        if !enabled {
            return Ok(());
        }

        // Store cache policy in typed context
        ctx.metadata.cache_policy = Some(crate::executor::CachePolicy {
            enabled,
            ttl,
            key_template: parsed.key_tpl.clone(),
        });

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

        // Check cache for a hit using env.cache (global shared cache)
        if let Some(cached_val) = env.cache.get(&key) {
            // Extract stored content_type if available
            let (value, stored_content_type) = if let Some(obj) = cached_val.as_object() {
                if let (Some(v), Some(ct)) = (obj.get("_cache_value"), obj.get("_cache_content_type")) {
                    (v.clone(), ct.as_str().map(|s| s.to_string()))
                } else {
                    (cached_val.clone(), None)
                }
            } else {
                (cached_val.clone(), None)
            };

            // CACHE HIT: Set state to wrapper with stop signal
            let mut control = serde_json::json!({
                "_control": {
                    "stop": true,
                    "value": value
                }
            });

            if let Some(ct) = stored_content_type {
                control["_control"]["content_type"] = serde_json::json!(ct);
            }

            pipeline_ctx.state = control;
            return Ok(());
        }

        // CACHE MISS: Register deferred action to save result at pipeline end
        let store = env.cache.clone();
        let key_clone = key.clone();
        ctx.defer(move |final_result, content_type, _env_ref| {
            // This runs AFTER the pipeline finishes
            // Store both the value and content_type
            let cached_data = serde_json::json!({
                "_cache_value": final_result,
                "_cache_content_type": content_type
            });

            // Use "write-once" semantics to prevent race conditions
            if store.get(&key_clone).is_none() {
                store.put(key_clone, cached_data, Some(ttl));
            }
        });

        // Cache miss - state unchanged
        Ok(())
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

    struct StubInvoker;
    #[async_trait::async_trait]
    impl crate::executor::MiddlewareInvoker for StubInvoker {
        async fn call(
            &self,
            _name: &str,
            _args: &[String],
            _cfg: &str,
            _pipeline_ctx: &mut crate::runtime::PipelineContext,
            _env: &crate::executor::ExecutionEnv,
            _ctx: &mut crate::executor::RequestContext,
            _target_name: Option<&str>,
        ) -> Result<(), WebPipeError> {
            Ok(())
        }
    }
    fn dummy_env() -> crate::executor::ExecutionEnv {
        use crate::executor::ExecutionEnv;
        use crate::runtime::context::{CacheStore, RateLimitStore};
        let registry = Arc::new(crate::middleware::MiddlewareRegistry::empty());
        ExecutionEnv {
            variables: Arc::new(std::collections::HashMap::new()),
            named_pipelines: Arc::new(std::collections::HashMap::new()),
            invoker: Arc::new(StubInvoker),
            registry,
            environment: None,
            cache: CacheStore::new(8, 60),
            rate_limit: RateLimitStore::new(1000),
            #[cfg(feature = "debugger")]
            debugger: None,
        }
    }

    #[tokio::test]
    async fn cache_miss_registers_deferred_action() {
        crate::config::init_global(vec![]);

        let ctx = test_ctx();
        let mw = CacheMiddleware { ctx: ctx.clone() };
        let input = serde_json::json!({});
        let env = dummy_env();
        let mut req_ctx = crate::executor::RequestContext::new();

        // Execute with cache miss
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        mw.execute(&[], "ttl: 5, keyTemplate: test-key-123", &mut pipeline_ctx, &env, &mut req_ctx, None).await.unwrap();

        // Should return input unchanged (no metadata)
        assert_eq!(pipeline_ctx.state, input);

        // Verify cache is empty before deferred runs (env.cache is the one used by middleware)
        assert!(env.cache.get("test-key-123").is_none());

        // Run deferred actions with final result
        let final_result = serde_json::json!({"final": "data"});
        req_ctx.run_deferred(&final_result, "application/json", &env);

        // Verify cache was populated (env.cache is the one used by middleware)
        let cached = env.cache.get("test-key-123").unwrap();
        assert_eq!(cached["_cache_value"]["final"], serde_json::json!("data"));
        assert_eq!(cached["_cache_content_type"], serde_json::json!("application/json"));
    }

    #[tokio::test]
    async fn returns_stop_signal_on_cache_hit() {
        crate::config::init_global(vec![]);

        let ctx = test_ctx();
        let mw = CacheMiddleware { ctx };
        let input = serde_json::json!({});
        let env = dummy_env();

        // Pre-populate env.cache (which is what the middleware uses)
        env.cache.put(
            "test-key".to_string(),
            serde_json::json!({
                "_cache_value": {"cached": "data"},
                "_cache_content_type": "text/html"
            }),
            Some(60)
        );

        let mut req_ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        mw.execute(&[], "keyTemplate: test-key", &mut pipeline_ctx, &env, &mut req_ctx, None).await.unwrap();

        // Should return stop signal with cached value and content_type
        assert_eq!(pipeline_ctx.state["_control"]["stop"], serde_json::json!(true));
        assert_eq!(pipeline_ctx.state["_control"]["value"]["cached"], serde_json::json!("data"));
        assert_eq!(pipeline_ctx.state["_control"]["content_type"], serde_json::json!("text/html"));
    }

    #[tokio::test]
    async fn cache_disabled_passes_through() {
        crate::config::init_global(vec![]);

        let mw = CacheMiddleware { ctx: test_ctx() };
        let input = serde_json::json!({"original": "data"});
        let env = dummy_env();
        let mut req_ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        mw.execute(&[], "enabled: false", &mut pipeline_ctx, &env, &mut req_ctx, None).await.unwrap();

        // Should pass through original data unchanged
        assert_eq!(pipeline_ctx.state["original"], serde_json::json!("data"));
    }
}

