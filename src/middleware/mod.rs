use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use serde_json::Value;
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

pub use jq::JqMiddleware;
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

#[async_trait]
pub trait Middleware: Send + Sync + std::fmt::Debug {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError>;
    async fn post_execute(&self, _final_response: &Value) -> Result<(), WebPipeError> { Ok(()) }
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

    pub fn register(&mut self, name: &str, middleware: Box<dyn Middleware>) {
        self.middlewares.insert(name.to_string(), middleware);
    }

    pub async fn execute(&self, name: &str, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        let middleware = self.middlewares.get(name)
            .ok_or_else(|| WebPipeError::MiddlewareNotFound(name.to_string()))?;
        middleware.execute(config, input).await
    }

    pub async fn post_execute_all(&self, final_response: &Value) {
        for mw in self.middlewares.values() { let _ = mw.post_execute(final_response).await; }
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

    #[tokio::test]
    async fn registry_executes_builtin_middleware() {
        let registry = MiddlewareRegistry::with_builtins(ctx_no_db());
        let out = registry.execute("jq", "{ ok: true }", &serde_json::json!({})).await.unwrap();
        assert_eq!(out["ok"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn registry_missing_middleware_errors() {
        let registry = MiddlewareRegistry::with_builtins(ctx_no_db());
        let err = registry.execute("nope", "", &serde_json::json!({})).await.unwrap_err();
        match err { WebPipeError::MiddlewareNotFound(_) => {}, other => panic!("unexpected: {:?}", other) }
    }
}


