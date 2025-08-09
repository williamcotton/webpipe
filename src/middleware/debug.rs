use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug)]
pub struct DebugMiddleware;

#[async_trait]
impl super::Middleware for DebugMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        let label = { let trimmed = config.trim(); if trimmed.is_empty() { "debug" } else { trimmed } };
        println!("{}", label);
        match serde_json::to_string_pretty(input) { Ok(pretty) => println!("{}", pretty), Err(_) => println!("{}", input), }
        Ok(input.clone())
    }
}


