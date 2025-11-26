use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::Value;
use std::cell::RefCell;
use lru::LruCache;

// Thread-local cache for JQ programs since jq_rs::JqProgram is not Send + Sync
thread_local! {
    static JQ_PROGRAM_CACHE: RefCell<LruCache<String, jq_rs::JqProgram>> = RefCell::new(
        LruCache::new(std::num::NonZeroUsize::new(100).unwrap())
    );
}

#[derive(Debug)]
pub struct JqMiddleware;

impl JqMiddleware {
    pub fn new() -> Self { Self }

    fn execute_jq(&self, filter: &str, input_json: &str) -> Result<String, WebPipeError> {
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
    async fn execute(&self, config: &str, input: &Value, _env: &crate::executor::ExecutionEnv, _ctx: &mut crate::executor::RequestContext) -> Result<Value, WebPipeError> {
        let input_json = serde_json::to_string(input)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Input serialization error: {}", e)))?;
        let result_json = self.execute_jq(config, &input_json)?;
        serde_json::from_str(&result_json)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ result parse error: {}", e)))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::Middleware;

    struct StubInvoker;
    #[async_trait::async_trait]
    impl crate::executor::MiddlewareInvoker for StubInvoker {
        async fn call(&self, _name: &str, _cfg: &str, _input: &Value, _env: &crate::executor::ExecutionEnv, _ctx: &mut crate::executor::RequestContext) -> Result<Value, WebPipeError> {
            Ok(Value::Null)
        }
    }
    fn dummy_env() -> crate::executor::ExecutionEnv {
        use crate::executor::ExecutionEnv;
        use crate::runtime::context::{CacheStore, RateLimitStore};
        use std::sync::Arc;
        use std::collections::HashMap;

        ExecutionEnv {
            variables: Arc::new(vec![]),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
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
        let out1 = jq.execute(".a", &input, &env, &mut ctx).await.unwrap();
        assert_eq!(out1, serde_json::json!(1));
        // second run should hit cache path implicitly; just ensure same result
        let out2 = jq.execute(".a", &input, &env, &mut ctx).await.unwrap();
        assert_eq!(out2, serde_json::json!(1));
    }

    #[tokio::test]
    async fn jq_parse_error_surfaces() {
        let jq = JqMiddleware::new();
        let input = serde_json::json!({});
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let err = jq.execute(".[] | ", &input, &env, &mut ctx).await.err().unwrap();
        // Ensure it's wrapped as MiddlewareExecutionError string
        assert!(format!("{}", err).contains("JQ"));
    }
}

