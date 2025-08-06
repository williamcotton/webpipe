use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::cell::RefCell;
use lru::LruCache;

#[async_trait]
pub trait Middleware: Send + Sync + std::fmt::Debug {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError>;
}

#[derive(Debug)]
pub struct MiddlewareRegistry {
    middlewares: HashMap<String, Box<dyn Middleware>>,
}

impl MiddlewareRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            middlewares: HashMap::new(),
        };
        
        // Register built-in middleware
        registry.register("jq", Box::new(JqMiddleware::new()));
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

// Thread-local cache for JQ programs since JqProgram is not Send + Sync
thread_local! {
    static JQ_PROGRAM_CACHE: RefCell<LruCache<String, jq_rs::JqProgram>> = RefCell::new(
        LruCache::new(std::num::NonZeroUsize::new(100).unwrap())
    );
}

// Placeholder middleware implementations
#[derive(Debug)]
pub struct JqMiddleware;

impl JqMiddleware {
    pub fn new() -> Self {
        Self
    }
    
    fn execute_jq(&self, filter: &str, input_json: &str) -> Result<String, WebPipeError> {
        JQ_PROGRAM_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            
            // Check if we have a compiled program for this filter
            if let Some(program) = cache.get_mut(filter) {
                // Use the cached compiled program
                return program.run(input_json)
                    .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ execution error: {}", e)));
            }
            
            // Not in cache, compile a new program
            match jq_rs::compile(filter) {
                Ok(mut program) => {
                    let result = program.run(input_json)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ execution error: {}", e)));
                    
                    // If successful, cache the compiled program for future use
                    if result.is_ok() {
                        cache.put(filter.to_string(), program);
                    }
                    
                    result
                },
                Err(e) => Err(WebPipeError::MiddlewareExecutionError(format!("JQ compilation error: {}", e)))
            }
        })
    }
}

impl Default for JqMiddleware {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Debug)]
pub struct AuthMiddleware;
#[derive(Debug)]
pub struct ValidateMiddleware;
#[derive(Debug)]
pub struct PgMiddleware;
#[derive(Debug)]
pub struct MustacheMiddleware;
#[derive(Debug)]
pub struct FetchMiddleware;
#[derive(Debug)]
pub struct CacheMiddleware;
#[derive(Debug)]
pub struct LuaMiddleware;
#[derive(Debug)]
pub struct LogMiddleware;

#[async_trait]
impl Middleware for JqMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Convert input to JSON string for jq processing
        let input_json = serde_json::to_string(input)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Input serialization error: {}", e)))?;
        
        // Execute the jq program
        let result_json = self.execute_jq(config, &input_json)?;
        
        // Parse the result back to serde_json::Value
        serde_json::from_str(&result_json)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ result parse error: {}", e)))
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