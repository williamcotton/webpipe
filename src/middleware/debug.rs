use crate::error::WebPipeError;
use async_trait::async_trait;

#[derive(Debug)]
pub struct DebugMiddleware;

#[async_trait]
impl super::Middleware for DebugMiddleware {
    async fn execute(
        &self,
        _args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<super::MiddlewareOutput, WebPipeError> {
        let input = &pipeline_ctx.state;
        let label = { let trimmed = config.trim(); if trimmed.is_empty() { "debug" } else { trimmed } };
        println!("{}", label);
        match serde_json::to_string_pretty(input) { Ok(pretty) => println!("{}", pretty), Err(_) => println!("{}", input), }
        // Read-only middleware - state unchanged
        Ok(super::MiddlewareOutput::default())
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
        ) -> Result<crate::middleware::MiddlewareOutput, WebPipeError> {
            Ok(crate::middleware::MiddlewareOutput::default())
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
            module_registry: Arc::new(crate::executor::ModuleRegistry::new()),
            debugger: None,
        }
    }

    #[tokio::test]
    async fn returns_input_and_prints_label() {
        let mw = DebugMiddleware;
        let input = serde_json::json!({"a":1});
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        mw.execute(&[], "", &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx.state["a"], serde_json::json!(1));
        let mut pipeline_ctx2 = crate::runtime::PipelineContext::new(input.clone());
        mw.execute(&[], "mylabel", &mut pipeline_ctx2, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx2.state["a"], serde_json::json!(1));
    }
}

