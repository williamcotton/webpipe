use crate::error::WebPipeError;
use async_trait::async_trait;

/// Evaluate a JQ filter and check if the result is truthy.
///
/// This function is used by @guard tags for conditional step execution.
/// It leverages the shared runtime JQ cache for optimal performance.
///
/// # Truthiness Rules
/// - `false` and `null` are falsy
/// - Everything else (numbers, strings, arrays, objects) is truthy
///
/// # Performance
/// - Cache Hit: O(1) program lookup + O(n) JQ execution
/// - Cache Miss: O(n) compilation + O(n) execution, then cached for future use
pub fn eval_bool(filter: &str, input: &serde_json::Value) -> Result<bool, WebPipeError> {
    // Use the shared runtime to evaluate the expression
    let result = crate::runtime::jq::evaluate(filter, input)?;

    // Apply truthiness logic matching standard JQ
    Ok(match result {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Null => false,
        _ => true // objects, arrays, numbers, strings are truthy
    })
}

#[derive(Debug)]
pub struct JqMiddleware;

impl JqMiddleware {
    pub fn new() -> Self { Self }
}

impl Default for JqMiddleware {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl super::Middleware for JqMiddleware {
    async fn execute(
        &self,
        _args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &crate::executor::ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<(), WebPipeError> {
        // 1. Create a combined input object containing both state and context
        // This keeps the data separate from the code (filter)
        let combined_input = serde_json::json!({
            "state": pipeline_ctx.state,
            "context": ctx.to_value(env)
        });

        // REMOVED: Step 2 (Serialization) is no longer necessary.
        // We now pass the 'combined_input' Value struct directly to the shared runtime.

        // 3. Create a static wrapper filter
        // This string depends ONLY on the config, not the request data, so it can be cached!
        let final_filter = format!(".context as $context | .state | {}", config);

        // 4. Execute using the shared runtime cache
        // We pass &combined_input (Value) and get back a Value directly
        let result_value = crate::runtime::jq::evaluate(&final_filter, &combined_input)?;

        // REMOVED: Step 5 (Deserialization) is no longer necessary.
        pipeline_ctx.state = result_value;
        Ok(())
    }

    fn behavior(&self) -> super::StateBehavior {
        super::StateBehavior::Transform
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::Middleware;

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

    #[tokio::test]
    async fn jq_happy_path_and_cache_reuse() {
        let jq = JqMiddleware::new();
        let input = serde_json::json!({"a":1});
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        jq.execute(&[], ".a", &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx.state, serde_json::json!(1));
        // second run should hit cache path implicitly; just ensure same result
        let mut pipeline_ctx2 = crate::runtime::PipelineContext::new(input.clone());
        jq.execute(&[], ".a", &mut pipeline_ctx2, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx2.state, serde_json::json!(1));
    }

    #[tokio::test]
    async fn jq_parse_error_surfaces() {
        let jq = JqMiddleware::new();
        let input = serde_json::json!({});
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let err = jq.execute(&[], ".[] | ", &mut pipeline_ctx, &env, &mut ctx, None).await.err().unwrap();
        // Ensure it's wrapped as MiddlewareExecutionError string
        assert!(format!("{}", err).contains("JQ"));
    }
}

