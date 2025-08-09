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
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        let input_json = serde_json::to_string(input)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Input serialization error: {}", e)))?;
        let result_json = self.execute_jq(config, &input_json)?;
        serde_json::from_str(&result_json)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ result parse error: {}", e)))
    }
}


