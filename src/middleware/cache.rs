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
    ) -> Result<super::MiddlewareOutput, WebPipeError> {
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
            return Ok(super::MiddlewareOutput::default());
        }

        // Store cache policy in typed context
        ctx.metadata.cache_policy = Some(crate::executor::CachePolicy {
            enabled,
            ttl,
            key_template: parsed.key_tpl.clone(),
        });

        // Generate cache key. keyTemplate keys are deliberately global (a
        // shared namespace) so multiple routes can share one entry; default
        // keys are namespaced by step identity so distinct cache steps with
        // identical configs never collide.
        let key = if let Some(tpl) = &parsed.key_tpl {
            render_key_template(tpl, input)
        } else {
            // Generate a hash-based key from the step identity, the config,
            // and relevant input parts
            let mut hasher = DefaultHasher::new();
            if let Some(step_id) = &pipeline_ctx.current_step_id {
                step_id.hash(&mut hasher);
            }
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

        // Look up with stale-while-revalidate semantics (env.cache is the
        // global shared cache)
        match env.cache.lookup(&key) {
            crate::runtime::CacheLookup::Fresh(resp) | crate::runtime::CacheLookup::Stale(resp) => {
                // CACHE HIT (fresh, or stale while another caller refreshes):
                // signal the executor to short-circuit this pipeline run with
                // the cached response.
                pipeline_ctx.cache_hit = Some(resp);
                Ok(super::MiddlewareOutput::default())
            }
            crate::runtime::CacheLookup::Refresh(_) => {
                // STALE, ELECTED REFRESHER: proceed past the cache step and
                // overwrite the entry with this pipeline run's final state.
                pipeline_ctx.pending_cache_saves.push(crate::runtime::PendingCacheSave {
                    key,
                    ttl,
                    store: env.cache.clone(),
                    refreshing: true,
                });
                Ok(super::MiddlewareOutput::default())
            }
            crate::runtime::CacheLookup::Miss => {
                // CACHE MISS: Register a pending save executed with the final
                // state of the pipeline this step runs in (not the enclosing
                // request), so the cached value matches what a cache hit
                // short-circuits to.
                pipeline_ctx.pending_cache_saves.push(crate::runtime::PendingCacheSave {
                    key,
                    ttl,
                    store: env.cache.clone(),
                    refreshing: false,
                });
                Ok(super::MiddlewareOutput::default())
            }
        }
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
            hb: Arc::new(parking_lot::RwLock::new(handlebars::Handlebars::new())),
            cfg: ConfigSnapshot(serde_json::json!({})),
            lua_scripts: Arc::new(std::collections::HashMap::new()),
            js_scripts: Arc::new(std::collections::HashMap::new()),
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
        ) -> Result<crate::middleware::MiddlewareOutput, WebPipeError> {
            Ok(crate::middleware::MiddlewareOutput::default())
        }
    }
    fn dummy_env() -> crate::executor::ExecutionEnv {
        use crate::executor::ExecutionEnv;
        use crate::runtime::context::{CacheStore, RateLimitStore};
        let registry = Arc::new(crate::middleware::MiddlewareRegistry::empty());
        ExecutionEnv {
            variables: Arc::new(std::collections::HashMap::new()),
            named_pipelines: Arc::new(std::collections::HashMap::new()),
            imports: Arc::new(std::collections::HashMap::new()),
            invoker: Arc::new(StubInvoker),
            registry,
            environment: None,
            cache: CacheStore::new(8, 60),
            rate_limit: RateLimitStore::new(1000),
            module_registry: Arc::new(crate::executor::ModuleRegistry::new()),
            debugger: None,
        }
    }

    fn lookup_fresh(cache: &crate::runtime::context::CacheStore, key: &str) -> Option<crate::runtime::CachedResponse> {
        match cache.lookup(key) {
            crate::runtime::CacheLookup::Fresh(r) => Some(r),
            _ => None,
        }
    }

    #[tokio::test]
    async fn cache_miss_registers_pending_save() {
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

        // Verify a pending save was registered but the cache is still empty
        assert_eq!(pipeline_ctx.pending_cache_saves.len(), 1);
        assert_eq!(pipeline_ctx.pending_cache_saves[0].key, "test-key-123");
        assert!(!pipeline_ctx.pending_cache_saves[0].refreshing);
        assert!(matches!(env.cache.lookup("test-key-123"), crate::runtime::CacheLookup::Miss));

        // Run pending saves with the pipeline's final state
        let final_result = serde_json::json!({"final": "data"});
        crate::executor::run_pending_cache_saves(
            std::mem::take(&mut pipeline_ctx.pending_cache_saves),
            &final_result,
            "application/json",
            Some(200),
        );

        // Verify cache was populated (env.cache is the one used by middleware)
        let cached = lookup_fresh(&env.cache, "test-key-123").unwrap();
        assert_eq!(cached.body["final"], serde_json::json!("data"));
        assert_eq!(cached.content_type, "application/json");
        assert_eq!(cached.status_code, Some(200));
    }

    #[tokio::test]
    async fn pending_save_skips_error_envelopes() {
        crate::config::init_global(vec![]);

        let mw = CacheMiddleware { ctx: test_ctx() };
        let env = dummy_env();
        let mut req_ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({}));
        mw.execute(&[], "ttl: 5, keyTemplate: err-key", &mut pipeline_ctx, &env, &mut req_ctx, None).await.unwrap();

        let error_result = serde_json::json!({
            "errors": [{ "type": "notFound", "message": "missing" }]
        });
        crate::executor::run_pending_cache_saves(
            std::mem::take(&mut pipeline_ctx.pending_cache_saves),
            &error_result,
            "application/json",
            None,
        );

        assert!(matches!(env.cache.lookup("err-key"), crate::runtime::CacheLookup::Miss));
    }

    #[tokio::test]
    async fn pending_save_skips_5xx_responses() {
        crate::config::init_global(vec![]);

        let mw = CacheMiddleware { ctx: test_ctx() };
        let env = dummy_env();
        let mut req_ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({}));
        mw.execute(&[], "ttl: 5, keyTemplate: err-500", &mut pipeline_ctx, &env, &mut req_ctx, None).await.unwrap();

        crate::executor::run_pending_cache_saves(
            std::mem::take(&mut pipeline_ctx.pending_cache_saves),
            &serde_json::json!("<html>error page</html>"),
            "text/html",
            Some(500),
        );

        assert!(matches!(env.cache.lookup("err-500"), crate::runtime::CacheLookup::Miss));
    }

    #[tokio::test]
    async fn signals_cache_hit_with_stored_response() {
        crate::config::init_global(vec![]);

        let ctx = test_ctx();
        let mw = CacheMiddleware { ctx };
        let input = serde_json::json!({});
        let env = dummy_env();

        // Pre-populate env.cache (which is what the middleware uses)
        env.cache.put_response(
            "test-key".to_string(),
            crate::runtime::CachedResponse {
                body: Arc::new(serde_json::json!({"cached": "data"})),
                content_type: "text/html".to_string(),
                status_code: Some(404),
            },
            Some(60),
        );

        let mut req_ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        mw.execute(&[], "keyTemplate: test-key", &mut pipeline_ctx, &env, &mut req_ctx, None).await.unwrap();

        // Should signal a typed hit and leave state untouched
        assert_eq!(pipeline_ctx.state, input, "state must not carry the cached value");
        let hit = pipeline_ctx.cache_hit.as_ref().expect("cache hit signalled");
        assert_eq!(hit.body["cached"], serde_json::json!("data"));
        assert_eq!(hit.content_type, "text/html");
        assert_eq!(hit.status_code, Some(404));
    }

    #[tokio::test]
    async fn stale_entry_elects_single_refresher_and_serves_stale() {
        crate::config::init_global(vec![]);

        let mw = CacheMiddleware { ctx: test_ctx() };
        let env = dummy_env();
        env.cache.put_response(
            "swr-key".to_string(),
            crate::runtime::CachedResponse {
                body: Arc::new(serde_json::json!({"v": 1})),
                content_type: "application/json".to_string(),
                status_code: Some(200),
            },
            Some(1),
        );
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        // First caller in the stale window proceeds as the refresher
        let mut req_ctx = crate::executor::RequestContext::new();
        let mut refresher_ctx = crate::runtime::PipelineContext::new(serde_json::json!({}));
        mw.execute(&[], "ttl: 1, keyTemplate: swr-key", &mut refresher_ctx, &env, &mut req_ctx, None).await.unwrap();
        assert!(refresher_ctx.cache_hit.is_none(), "refresher must proceed past the cache step");
        assert_eq!(refresher_ctx.pending_cache_saves.len(), 1);
        assert!(refresher_ctx.pending_cache_saves[0].refreshing);

        // Concurrent callers get the stale value
        let mut stale_ctx = crate::runtime::PipelineContext::new(serde_json::json!({}));
        mw.execute(&[], "ttl: 1, keyTemplate: swr-key", &mut stale_ctx, &env, &mut req_ctx, None).await.unwrap();
        let hit = stale_ctx.cache_hit.as_ref().expect("stale value served");
        assert_eq!(hit.body["v"], serde_json::json!(1));

        // The refresher's save overwrites the stale entry
        crate::executor::run_pending_cache_saves(
            std::mem::take(&mut refresher_ctx.pending_cache_saves),
            &serde_json::json!({"v": 2}),
            "application/json",
            Some(200),
        );
        let refreshed = lookup_fresh(&env.cache, "swr-key").unwrap();
        assert_eq!(refreshed.body["v"], serde_json::json!(2));
    }

    #[tokio::test]
    async fn identical_configs_get_distinct_default_keys_per_step() {
        crate::config::init_global(vec![]);

        let mw = CacheMiddleware { ctx: test_ctx() };
        let env = dummy_env();
        let mut req_ctx = crate::executor::RequestContext::new();
        // Same config and same state, but two different step identities —
        // this is the route-cache vs nested-pipeline-cache collision
        let input = serde_json::json!({"method": "GET", "path": "/", "query": {}, "params": {}});

        let mut ctx_a = crate::runtime::PipelineContext::new(input.clone());
        ctx_a.current_step_id = Some("app.wp:10:3#0".to_string());
        mw.execute(&[], "ttl: 60", &mut ctx_a, &env, &mut req_ctx, None).await.unwrap();

        let mut ctx_b = crate::runtime::PipelineContext::new(input);
        ctx_b.current_step_id = Some("app.wp:42:3#2".to_string());
        mw.execute(&[], "ttl: 60", &mut ctx_b, &env, &mut req_ctx, None).await.unwrap();

        assert_eq!(ctx_a.pending_cache_saves.len(), 1);
        assert_eq!(ctx_b.pending_cache_saves.len(), 1);
        assert_ne!(
            ctx_a.pending_cache_saves[0].key, ctx_b.pending_cache_saves[0].key,
            "distinct steps must never share a default cache key"
        );
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

