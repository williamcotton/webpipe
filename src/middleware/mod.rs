use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

mod jq;
mod auth;
mod validate;
mod pg;
mod handlebars;
mod fetch;
mod cache;
mod lua;
mod log;
mod debug;
mod join;
mod graphql;
mod rate_limit;

pub use jq::{JqMiddleware, eval_bool as jq_eval_bool};
pub use auth::AuthMiddleware;
pub use validate::ValidateMiddleware;
pub use pg::PgMiddleware;
pub use handlebars::HandlebarsMiddleware;
pub use fetch::FetchMiddleware;
pub use cache::CacheMiddleware;
pub use lua::LuaMiddleware;
pub use log::LogMiddleware;
pub use debug::DebugMiddleware;
pub use join::JoinMiddleware;
pub use graphql::GraphQLMiddleware;
pub use rate_limit::RateLimitMiddleware;

/// Describes how a middleware interacts with the pipeline state
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StateBehavior {
    /// "Backpack" semantics: Merge object keys into the existing state.
    /// Used by: pg, fetch, auth, validate
    Merge,

    /// Transform semantics: Replace state entirely, but preserve system context (headers, params, etc) if not terminal.
    /// Used by: jq, lua
    Transform,

    /// Render/Replace semantics: Replace state entirely, intended for final output (HTML/Text).
    /// Used by: handlebars
    Render,

    /// Read-only: Does not modify state.
    /// Used by: log, debug, cache (mostly)
    ReadOnly,
}

#[async_trait]
pub trait Middleware: Send + Sync + std::fmt::Debug {
    /// Execute middleware, mutating pipeline_ctx.state in place
    /// This eliminates the need to clone and return a new Value
    ///
    /// # Arguments
    /// * `args` - Optional inline arguments evaluated from JQ expressions
    /// * `config` - The configuration string for this middleware step
    /// * `target_name` - Optional result target name from @result(name) tag or resultName variable
    async fn execute(
        &self,
        args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &crate::executor::ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
        target_name: Option<&str>,
    ) -> Result<(), WebPipeError>;

    /// Returns the state manipulation behavior of this middleware.
    /// Default implementation returns Merge (the most common behavior).
    fn behavior(&self) -> StateBehavior {
        StateBehavior::Merge
    }
}

#[derive(Debug)]
pub struct MiddlewareRegistry {
    middlewares: HashMap<String, Box<dyn Middleware>>,
}

impl Default for MiddlewareRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MiddlewareRegistry {
    pub fn with_builtins(ctx: Arc<Context>) -> Self {
        let mut registry = Self { middlewares: HashMap::new() };
        registry.register("jq", Box::new(JqMiddleware::new()));
        registry.register("auth", Box::new(AuthMiddleware { ctx: ctx.clone() }));
        registry.register("validate", Box::new(ValidateMiddleware));
        registry.register("pg", Box::new(PgMiddleware { ctx: ctx.clone() }));
        registry.register("handlebars", Box::new(HandlebarsMiddleware::new_with_ctx(ctx.clone())));
        registry.register("fetch", Box::new(FetchMiddleware { ctx: ctx.clone() }));
        registry.register("cache", Box::new(CacheMiddleware { ctx: ctx.clone() }));
        registry.register("lua", Box::new(LuaMiddleware::new(ctx.clone())));
        registry.register("log", Box::new(LogMiddleware));
        registry.register("debug", Box::new(DebugMiddleware));
        registry.register("join", Box::new(JoinMiddleware));
        registry.register("graphql", Box::new(GraphQLMiddleware::new(ctx.clone())));
        registry.register("rateLimit", Box::new(RateLimitMiddleware { ctx: ctx.clone() }));
        registry
    }

    pub fn new() -> Self {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        let ctx = rt
            .block_on(crate::runtime::Context::from_program_configs(vec![], &[]))
            .unwrap_or_else(|_| panic!("failed to create default Context"));
        Self::with_builtins(Arc::new(ctx))
    }

    /// Create an empty registry without any middleware (useful for tests)
    pub fn empty() -> Self {
        Self { middlewares: HashMap::new() }
    }

    pub fn register(&mut self, name: &str, middleware: Box<dyn Middleware>) {
        self.middlewares.insert(name.to_string(), middleware);
    }

    pub async fn execute(&self, name: &str, args: &[String], config: &str, pipeline_ctx: &mut crate::runtime::PipelineContext, env: &crate::executor::ExecutionEnv, ctx: &mut crate::executor::RequestContext, target_name: Option<&str>) -> Result<(), WebPipeError> {
        let middleware = self.middlewares.get(name)
            .ok_or_else(|| WebPipeError::MiddlewareNotFound(name.to_string()))?;
        middleware.execute(args, config, pipeline_ctx, env, ctx, target_name).await
    }

    pub fn get_behavior(&self, name: &str) -> Option<StateBehavior> {
        self.middlewares.get(name).map(|m| m.behavior())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Context;
    use crate::runtime::context::{CacheStore, ConfigSnapshot};
    use reqwest::Client;
    use ::handlebars::Handlebars;

    fn ctx_no_db() -> Arc<Context> {
        Arc::new(Context {
            pg: None,
            http: Client::new(),
            cache: CacheStore::new(8, 1),
            rate_limit: crate::runtime::context::RateLimitStore::new(1000),
            hb: std::sync::Arc::new(parking_lot::Mutex::new(Handlebars::new())),
            cfg: ConfigSnapshot(serde_json::json!({})),
            lua_scripts: std::sync::Arc::new(std::collections::HashMap::new()),
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
        use std::sync::Arc;
        use std::collections::HashMap;

        let registry = Arc::new(MiddlewareRegistry::empty());
        ExecutionEnv {
            variables: Arc::new(HashMap::new()),
            named_pipelines: Arc::new(HashMap::new()),
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
    async fn registry_executes_builtin_middleware() {
        let registry = MiddlewareRegistry::with_builtins(ctx_no_db());
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({}));
        registry.execute("jq", &[], "{ ok: true }", &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx.state["ok"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn registry_missing_middleware_errors() {
        let registry = MiddlewareRegistry::with_builtins(ctx_no_db());
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({}));
        let err = registry.execute("nope", &[], "", &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap_err();
        match err { WebPipeError::MiddlewareNotFound(_) => {}, other => panic!("unexpected: {:?}", other) }
    }
}


