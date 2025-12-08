use crate::error::WebPipeError;
use async_trait::async_trait;
use std::cell::RefCell;
use lru::LruCache;

// Thread-local cache for JQ programs since jq_rs::JqProgram is not Send + Sync
thread_local! {
    static JQ_PROGRAM_CACHE: RefCell<LruCache<String, jq_rs::JqProgram>> = RefCell::new(
        LruCache::new(std::num::NonZeroUsize::new(100).unwrap())
    );
}

/// Evaluate a JQ filter and check if the result is truthy.
///
/// This function is used by @guard tags for conditional step execution.
/// It leverages the thread-local JQ_PROGRAM_CACHE for optimal performance.
///
/// # Truthiness Rules
/// - `false` and `null` are falsy
/// - Everything else (numbers, strings, arrays, objects) is truthy
///
/// # Performance
/// - Cache Hit: O(1) program lookup + O(n) JQ execution
/// - Cache Miss: O(n) compilation + O(n) execution, then cached for future use
pub fn eval_bool(filter: &str, input: &serde_json::Value) -> Result<bool, WebPipeError> {
    // 1. Serialize Input (Serialization Tax)
    let input_json = serde_json::to_string(input)
        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Guard input error: {}", e)))?;

    // 2. Use the Thread-Local Cache
    JQ_PROGRAM_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();

        // A. Cache Hit
        if let Some(program) = cache.get_mut(filter) {
            let output = program.run(&input_json)
                .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Guard execution error: {}", e)))?;
            return parse_bool_output(&output);
        }

        // B. Cache Miss - Compile
        match jq_rs::compile(filter) {
            Ok(mut program) => {
                let output = program.run(&input_json)
                    .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Guard execution error: {}", e)))?;

                // Store in cache for future use
                cache.put(filter.to_string(), program);

                parse_bool_output(&output)
            }
            Err(e) => Err(WebPipeError::MiddlewareExecutionError(format!("Guard compilation error: {}", e))),
        }
    })
}

/// Helper to parse JQ output string to boolean
fn parse_bool_output(output: &str) -> Result<bool, WebPipeError> {
    // JQ returns a string like "false\n" or "null\n" or "{...}\n"
    let val: serde_json::Value = serde_json::from_str(output)
        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Guard result parse error: {}", e)))?;

    // Truthiness logic matching standard JQ
    Ok(match val {
        serde_json::Value::Bool(b) => b,
        serde_json::Value::Null => false,
        _ => true // objects, arrays, numbers, strings are truthy
    })
}

#[derive(Debug)]
pub struct JqMiddleware;

impl JqMiddleware {
    pub fn new() -> Self { Self }

    // Helper that always uses the cache
    fn execute_cached(&self, filter: &str, input_json: &str) -> Result<String, WebPipeError> {
        JQ_PROGRAM_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();

            if let Some(program) = cache.get_mut(filter) {
                return program
                    .run(input_json)
                    .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ execution error: {}", e)));
            }

            match jq_rs::compile(filter) {
                Ok(mut program) => {
                    let result = program
                        .run(input_json)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ execution error: {}", e)));
                    if result.is_ok() {
                        cache.put(filter.to_string(), program);
                    }
                    result
                }
                Err(e) => Err(WebPipeError::MiddlewareExecutionError(format!("JQ compilation error: {}", e))),
            }
        })
    }
}

impl Default for JqMiddleware {
    fn default() -> Self { Self::new() }
}

#[async_trait]
impl super::Middleware for JqMiddleware {
    async fn execute(
        &self,
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &crate::executor::ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
    ) -> Result<(), WebPipeError> {
        // 1. Create a combined input object containing both state and context
        // This keeps the data separate from the code (filter)
        let combined_input = serde_json::json!({
            "state": pipeline_ctx.state,
            "context": ctx.to_value(env)
        });

        // 2. Serialize the combined input
        let input_json = serde_json::to_string(&combined_input)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Input serialization error: {}", e)))?;

        // 3. Create a static wrapper filter
        // This string depends ONLY on the config, not the request data, so it can be cached!
        let final_filter = format!(".context as $context | .state | {}", config);

        // 4. Execute using the cache
        let result_json = self.execute_cached(&final_filter, &input_json)?;
        
        pipeline_ctx.state = serde_json::from_str(&result_json)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ result parse error: {}", e)))?;
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
            _cfg: &str,
            _pipeline_ctx: &mut crate::runtime::PipelineContext,
            _env: &crate::executor::ExecutionEnv,
            _ctx: &mut crate::executor::RequestContext,
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
            variables: Arc::new(vec![]),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
            registry,
            environment: None,
            cache: CacheStore::new(8, 60),
            rate_limit: RateLimitStore::new(1000),
        }
    }

    #[tokio::test]
    async fn jq_happy_path_and_cache_reuse() {
        let jq = JqMiddleware::new();
        let input = serde_json::json!({"a":1});
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        jq.execute(".a", &mut pipeline_ctx, &env, &mut ctx).await.unwrap();
        assert_eq!(pipeline_ctx.state, serde_json::json!(1));
        // second run should hit cache path implicitly; just ensure same result
        let mut pipeline_ctx2 = crate::runtime::PipelineContext::new(input.clone());
        jq.execute(".a", &mut pipeline_ctx2, &env, &mut ctx).await.unwrap();
        assert_eq!(pipeline_ctx2.state, serde_json::json!(1));
    }

    #[tokio::test]
    async fn jq_parse_error_surfaces() {
        let jq = JqMiddleware::new();
        let input = serde_json::json!({});
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let err = jq.execute(".[] | ", &mut pipeline_ctx, &env, &mut ctx).await.err().unwrap();
        // Ensure it's wrapped as MiddlewareExecutionError string
        assert!(format!("{}", err).contains("JQ"));
    }
}

