use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug)]
pub struct RateLimitMiddleware {
    pub ctx: Arc<Context>,
}

/// Configuration for the rateLimit middleware
#[derive(Debug, Default)]
struct RateLimitConfig {
    /// Template for the rate limit key, e.g. "ip-{ip}-route-{method}-{path}"
    key_template: Option<String>,
    /// Maximum requests allowed per window
    limit: Option<u64>,
    /// Window duration in seconds
    window_secs: Option<u64>,
    /// Extra burst capacity
    burst: Option<u64>,
    /// Semantic scope hint (route, global, custom)
    scope: Option<String>,
    /// Whether rate limiting is enabled (default: true)
    enabled: Option<bool>,
}

/// Parse a duration string like "60s", "1m", "5m", "1h" into seconds
fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Try to parse as plain number (seconds)
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }

    // Parse duration with unit suffix
    if let Some(num_str) = s.strip_suffix("ms") {
        let num: u64 = num_str.trim().parse().ok()?;
        return Some(num / 1000); // Convert milliseconds to seconds (minimum 0)
    }
    if let Some(num_str) = s.strip_suffix('s') {
        let num: u64 = num_str.trim().parse().ok()?;
        return Some(num);
    }
    if let Some(num_str) = s.strip_suffix('m') {
        let num: u64 = num_str.trim().parse().ok()?;
        return Some(num * 60);
    }
    if let Some(num_str) = s.strip_suffix('h') {
        let num: u64 = num_str.trim().parse().ok()?;
        return Some(num * 3600);
    }
    if let Some(num_str) = s.strip_suffix('d') {
        let num: u64 = num_str.trim().parse().ok()?;
        return Some(num * 86400);
    }

    None
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_config(cfg: &str) -> RateLimitConfig {
    let mut lc = RateLimitConfig::default();

    for part in cfg.replace('\n', ",").split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }

        if let Some((k, v)) = p.split_once(':') {
            let key = k.trim();
            let val = v.trim();

            match key {
                "keyTemplate" => lc.key_template = Some(val.to_string()),
                "limit" => lc.limit = val.parse::<u64>().ok(),
                "window" => lc.window_secs = parse_duration(val),
                "burst" => lc.burst = val.parse::<u64>().ok(),
                "scope" => lc.scope = Some(val.to_string()),
                "enabled" => lc.enabled = parse_bool(val),
                _ => {}
            }
        }
    }

    lc
}

/// Interpolate a template string with values from the input state.
/// Supports patterns like {ip}, {method}, {path}, {user.id}, {params.id}
fn interpolate_key(template: &str, input: &Value) -> String {
    let mut result = template.to_string();
    let mut start = 0;

    while let Some(open_idx) = result[start..].find('{') {
        let open_idx = start + open_idx;
        if let Some(close_idx) = result[open_idx..].find('}') {
            let close_idx = open_idx + close_idx;
            let var_name = &result[open_idx + 1..close_idx];

            // Resolve the variable from input
            let value = resolve_path(input, var_name);
            let replacement = value
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Null => "null".to_string(),
                    _ => v.to_string(),
                })
                .unwrap_or_else(|| "unknown".to_string());

            result.replace_range(open_idx..=close_idx, &replacement);
            start = open_idx + replacement.len();
        } else {
            break;
        }
    }

    result
}

/// Resolve a dot-separated path in a JSON value, e.g. "user.id" or "params.name"
fn resolve_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;

    for part in parts {
        match current {
            Value::Object(map) => {
                current = map.get(part)?;
            }
            Value::Array(arr) => {
                let idx: usize = part.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }

    Some(current)
}

#[async_trait]
impl super::Middleware for RateLimitMiddleware {
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
        let cfg = parse_config(config);

        // Check if rate limiting is enabled
        let enabled = cfg.enabled.unwrap_or(true);
        if !enabled {
            return Ok(());
        }

        // Validate required fields
        let key_template = cfg.key_template.ok_or_else(|| {
            WebPipeError::ConfigError("rateLimit: keyTemplate is required".to_string())
        })?;

        let limit = cfg.limit.ok_or_else(|| {
            WebPipeError::ConfigError("rateLimit: limit is required".to_string())
        })?;

        let window_secs = cfg.window_secs.ok_or_else(|| {
            WebPipeError::ConfigError("rateLimit: window is required".to_string())
        })?;

        // Ensure window is at least 1 second
        let window_secs = window_secs.max(1);

        // Interpolate the key template with values from input
        let key = interpolate_key(&key_template, input);

        // Check rate limit using env.rate_limit (global shared store)
        let (allowed, current_count, effective_limit, retry_after) =
            env.rate_limit.check_and_increment(&key, limit, window_secs, cfg.burst);

        // Store rate limit status in typed context (The Backpack)
        ctx.metadata.rate_limit_status = Some(crate::executor::RateLimitStatus {
            allowed,
            current_count,
            limit: effective_limit,
            retry_after_secs: retry_after,
            key: key.clone(),
            scope: cfg.scope,
        });

        if !allowed {
            return Err(WebPipeError::RateLimitExceeded(format!(
                "Rate limit exceeded for key '{}': {}/{} requests in {}s window. Retry after {}s",
                key, current_count, effective_limit, window_secs, retry_after
            )));
        }

        // Read-only middleware - state unchanged, rate limit info is in ctx.metadata.rate_limit_status
        Ok(())
    }
}

#[cfg(test)]
mod tests {
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
        use std::sync::Arc;
        use std::collections::HashMap;

        let registry = Arc::new(crate::middleware::MiddlewareRegistry::empty());
        ExecutionEnv {
            variables: Arc::new(HashMap::new()),
            named_pipelines: Arc::new(HashMap::new()),
            imports: Arc::new(std::collections::HashMap::new()),
            invoker: Arc::new(StubInvoker),
            registry,
            environment: None,
            cache: CacheStore::new(8, 60),
            rate_limit: RateLimitStore::new(1000),
            #[cfg(feature = "debugger")]
            debugger: None,
        }
    }
    use super::*;
    use crate::middleware::Middleware;
    use crate::runtime::context::{CacheStore, ConfigSnapshot, RateLimitStore};
    use parking_lot::RwLock;
    use serde_json::json;

    fn test_ctx() -> Arc<Context> {
        Arc::new(Context {
            pg: None,
            http: reqwest::Client::new(),
            cache: CacheStore::new(8, 60),
            rate_limit: RateLimitStore::new(1000),
            hb: Arc::new(parking_lot::Mutex::new(handlebars::Handlebars::new())),
            cfg: ConfigSnapshot(json!({})),
            lua_scripts: Arc::new(std::collections::HashMap::new()),
            js_scripts: Arc::new(std::collections::HashMap::new()),
            graphql: None,
            execution_env: Arc::new(RwLock::new(None)),
        })
    }

    #[test]
    fn parse_duration_works() {
        assert_eq!(parse_duration("60"), Some(60));
        assert_eq!(parse_duration("60s"), Some(60));
        assert_eq!(parse_duration("1m"), Some(60));
        assert_eq!(parse_duration("5m"), Some(300));
        assert_eq!(parse_duration("1h"), Some(3600));
        assert_eq!(parse_duration("1d"), Some(86400));
        assert_eq!(parse_duration("500ms"), Some(0)); // 500ms = 0s when truncated
        assert_eq!(parse_duration("2000ms"), Some(2));
        assert_eq!(parse_duration(""), None);
        assert_eq!(parse_duration("abc"), None);
    }

    #[test]
    fn parse_config_extracts_all_fields() {
        let cfg = parse_config(
            "keyTemplate: ip-{ip}, limit: 100, window: 60s, burst: 10, scope: route, enabled: true",
        );
        assert_eq!(cfg.key_template, Some("ip-{ip}".to_string()));
        assert_eq!(cfg.limit, Some(100));
        assert_eq!(cfg.window_secs, Some(60));
        assert_eq!(cfg.burst, Some(10));
        assert_eq!(cfg.scope, Some("route".to_string()));
        assert_eq!(cfg.enabled, Some(true));
    }

    #[test]
    fn parse_config_multiline() {
        let cfg = parse_config(
            r#"
            keyTemplate: user-{user.id}
            limit: 50
            window: 1m
            "#,
        );
        assert_eq!(cfg.key_template, Some("user-{user.id}".to_string()));
        assert_eq!(cfg.limit, Some(50));
        assert_eq!(cfg.window_secs, Some(60));
    }

    #[test]
    fn interpolate_key_simple() {
        let input = json!({
            "ip": "192.168.1.1",
            "method": "GET",
            "path": "/api/users"
        });
        let result = interpolate_key("ip-{ip}-route-{method}-{path}", &input);
        assert_eq!(result, "ip-192.168.1.1-route-GET-/api/users");
    }

    #[test]
    fn interpolate_key_nested() {
        let input = json!({
            "user": {
                "id": 123,
                "name": "Alice"
            },
            "params": {
                "id": "456"
            }
        });
        let result = interpolate_key("user-{user.id}-resource-{params.id}", &input);
        assert_eq!(result, "user-123-resource-456");
    }

    #[test]
    fn interpolate_key_missing_value() {
        let input = json!({
            "ip": "1.2.3.4"
        });
        let result = interpolate_key("user-{user.id}", &input);
        assert_eq!(result, "user-unknown");
    }

    #[tokio::test]
    async fn rate_limit_allows_within_limit() {
        crate::config::init_global(vec![]);

        let ctx = test_ctx();
        let mw = RateLimitMiddleware { ctx };

        let input = json!({
            "ip": "10.0.0.1",
            "method": "GET",
            "path": "/test"
        });

        // First request should be allowed
        let mut req_ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let result = mw
            .execute(&[], "keyTemplate: test-{ip}, limit: 5, window: 60s", &mut pipeline_ctx, &dummy_env(), &mut req_ctx, None)
            .await;
        assert!(result.is_ok());

        // Rate limit info is now in the typed context, not in JSON output
        let status = req_ctx.metadata.rate_limit_status.unwrap();
        assert!(status.allowed);
        assert_eq!(status.limit - status.current_count, 4); // remaining
        assert_eq!(status.limit, 5);
    }

    #[tokio::test]
    async fn rate_limit_blocks_when_exceeded() {
        crate::config::init_global(vec![]);

        let ctx = test_ctx();
        let mw = RateLimitMiddleware { ctx };

        let input = json!({
            "ip": "10.0.0.2"
        });

        // Make requests up to the limit
        let env = dummy_env();
        let mut req_ctx = crate::executor::RequestContext::new();
        for _ in 0..3 {
            let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
            let result = mw
                .execute(&[], "keyTemplate: block-test-{ip}, limit: 3, window: 60s", &mut pipeline_ctx, &env, &mut req_ctx, None)
                .await;
            assert!(result.is_ok());
        }

        // Next request should be blocked
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let result = mw
            .execute(&[], "keyTemplate: block-test-{ip}, limit: 3, window: 60s", &mut pipeline_ctx, &env, &mut req_ctx, None)
            .await;
        assert!(result.is_err());

        match result {
            Err(WebPipeError::RateLimitExceeded(msg)) => {
                assert!(msg.contains("Rate limit exceeded"));
            }
            _ => panic!("Expected RateLimitExceeded error"),
        }
    }

    #[tokio::test]
    async fn rate_limit_burst_allows_extra() {
        crate::config::init_global(vec![]);

        let ctx = test_ctx();
        let mw = RateLimitMiddleware { ctx };

        let input = json!({ "ip": "10.0.0.3" });
        let env = dummy_env();
        let mut req_ctx = crate::executor::RequestContext::new();

        // With limit: 2 and burst: 2, we should allow 4 requests
        for i in 0..4 {
            let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
            let result = mw
                .execute(
                    &[], "keyTemplate: burst-test-{ip}, limit: 2, window: 60s, burst: 2",
                    &mut pipeline_ctx,
                    &env,
                    &mut req_ctx,
                    None,
                )
                .await;
            assert!(result.is_ok(), "Request {} should be allowed", i + 1);
        }

        // 5th request should be blocked
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let result = mw
            .execute(
                &[], "keyTemplate: burst-test-{ip}, limit: 2, window: 60s, burst: 2",
                &mut pipeline_ctx,
                &env,
                &mut req_ctx,
                None,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rate_limit_disabled_passes_through() {
        crate::config::init_global(vec![]);

        let ctx = test_ctx();
        let mw = RateLimitMiddleware { ctx };

        let input = json!({ "ip": "10.0.0.4" });

        // Even without valid config, disabled should pass through
        let mut req_ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let result = mw.execute(&[], "enabled: false", &mut pipeline_ctx, &dummy_env(), &mut req_ctx, None).await;
        assert!(result.is_ok());
        assert_eq!(pipeline_ctx.state, input);
    }

    #[tokio::test]
    async fn rate_limit_missing_config_errors() {
        crate::config::init_global(vec![]);

        let ctx = test_ctx();
        let mw = RateLimitMiddleware { ctx };

        let input = json!({});
        let env = dummy_env();
        let mut req_ctx = crate::executor::RequestContext::new();

        // Missing keyTemplate
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let result = mw.execute(&[], "limit: 10, window: 60s", &mut pipeline_ctx, &env, &mut req_ctx, None).await;
        assert!(result.is_err());
        match result {
            Err(WebPipeError::ConfigError(msg)) => {
                assert!(msg.contains("keyTemplate"));
            }
            _ => panic!("Expected ConfigError"),
        }

        // Missing limit
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let result = mw.execute(&[], "keyTemplate: test, window: 60s", &mut pipeline_ctx, &env, &mut req_ctx, None).await;
        assert!(result.is_err());

        // Missing window
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let result = mw.execute(&[], "keyTemplate: test, limit: 10", &mut pipeline_ctx, &env, &mut req_ctx, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn rate_limit_adds_scope_to_context() {
        crate::config::init_global(vec![]);

        let ctx = test_ctx();
        let mw = RateLimitMiddleware { ctx };

        let input = json!({ "ip": "10.0.0.5" });

        let mut req_ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let result = mw
            .execute(
                &[], "keyTemplate: scope-test-{ip}, limit: 10, window: 60s, scope: route",
                &mut pipeline_ctx,
                &dummy_env(),
                &mut req_ctx,
                None,
            )
            .await;
        assert!(result.is_ok());

        // Scope is now in the typed context
        let status = req_ctx.metadata.rate_limit_status.unwrap();
        assert_eq!(status.scope, Some("route".to_string()));
        assert_eq!(status.key, "scope-test-10.0.0.5");
    }
}

