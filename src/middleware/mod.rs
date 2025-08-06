use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

#[async_trait]
pub trait Middleware: Send + Sync {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError>;
}

pub struct MiddlewareRegistry {
    middlewares: HashMap<String, Box<dyn Middleware>>,
}

impl MiddlewareRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            middlewares: HashMap::new(),
        };
        
        // Register built-in middleware
        registry.register("jq", Box::new(JqMiddleware));
        registry.register("auth", Box::new(AuthMiddleware));
        registry.register("validate", Box::new(ValidateMiddleware));
        registry.register("pg", Box::new(PgMiddleware));
        registry.register("mustache", Box::new(MustacheMiddleware));
        registry.register("fetch", Box::new(FetchMiddleware));
        registry.register("cache", Box::new(CacheMiddleware));
        registry.register("lua", Box::new(LuaMiddleware));
        registry.register("log", Box::new(LogMiddleware));
        
        registry
    }

    pub fn register(&mut self, name: &str, middleware: Box<dyn Middleware>) {
        self.middlewares.insert(name.to_string(), middleware);
    }

    pub async fn execute(
        &self,
        name: &str,
        config: &str,
        input: &Value,
    ) -> Result<Value, WebPipeError> {
        let middleware = self.middlewares.get(name)
            .ok_or_else(|| WebPipeError::MiddlewareNotFound(name.to_string()))?;
        
        middleware.execute(config, input).await
    }
}

// Placeholder middleware implementations
pub struct JqMiddleware;
pub struct AuthMiddleware;
pub struct ValidateMiddleware;
pub struct PgMiddleware;
pub struct MustacheMiddleware;
pub struct FetchMiddleware;
pub struct CacheMiddleware;
pub struct LuaMiddleware;
pub struct LogMiddleware;

#[async_trait]
impl Middleware for JqMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "jq",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "auth",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for ValidateMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "validate",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for PgMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "pg",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for MustacheMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "mustache",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for FetchMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "fetch",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for CacheMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "cache",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for LuaMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "lua",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for LogMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        println!("LOG: {} -> {:?}", config, input);
        Ok(input.clone())
    }
}