use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug)]
pub struct DebugMiddleware;

#[async_trait]
impl super::Middleware for DebugMiddleware {
    async fn execute(&self, config: &str, input: &Value, _env: &crate::executor::ExecutionEnv) -> Result<Value, WebPipeError> {
        let label = { let trimmed = config.trim(); if trimmed.is_empty() { "debug" } else { trimmed } };
        println!("{}", label);
        match serde_json::to_string_pretty(input) { Ok(pretty) => println!("{}", pretty), Err(_) => println!("{}", input), }
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
    async fn returns_input_and_prints_label() {
        let mw = DebugMiddleware;
        let input = serde_json::json!({"a":1});
        let env = dummy_env();
        let out = mw.execute("", &input, &env).await.unwrap();
        assert_eq!(out["a"], serde_json::json!(1));
        let out2 = mw.execute("mylabel", &input, &env).await.unwrap();
        assert_eq!(out2["a"], serde_json::json!(1));
    }
}

