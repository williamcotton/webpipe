use std::pin::Pin;
use std::future::Future;
use serde_json::Value;
use crate::error::WebPipeError;

/// Output from pipeline execution
#[derive(Debug, Clone)]
pub struct PipelineOutput {
    /// The current JSON body/state of the pipeline
    pub state: Value,
    /// The MIME type (e.g., "application/json", "text/html")
    pub content_type: String,
    /// The HTTP status code, if set
    pub status_code: Option<u16>,
}

impl PipelineOutput {
    pub fn new(state: Value) -> Self {
        Self {
            state,
            content_type: "application/json".to_string(),
            status_code: None,
        }
    }
}

/// Loop control enum to manage flow within the pipeline loop
pub enum StepOutcome {
    /// Continue to the next step
    Continue,
    /// Stop execution and return the pipeline result immediately
    Return(PipelineOutput),
}

/// Represents different execution modes for pipeline steps
#[derive(Debug)]
pub enum ExecutionMode {
    /// Standard synchronous middleware execution
    Standard,
    /// Asynchronous execution - spawn task with given name
    Async(String),
    /// Join operation - wait for async tasks
    Join,
    /// Recursive pipeline execution
    Recursive,
}

/// Result from executing a single step
pub struct StepResult {
    pub value: Value,
    pub content_type: String,
    pub status_code: Option<u16>,
}

/// Public alias for the complex future type returned by pipeline execution functions.
pub type PipelineExecFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<(Value, String, Option<u16>), WebPipeError>> + Send + 'a,
    >,
>;

/// Cache policy configuration for middleware communication
#[derive(Clone, Debug, serde::Serialize)]
pub struct CachePolicy {
    pub enabled: bool,
    pub ttl: u64,
    pub key_template: Option<String>,
}

/// Log configuration for logging middleware
#[derive(Clone, Debug, serde::Serialize)]
pub struct LogConfig {
    pub level: String,
    pub include_body: bool,
    pub include_headers: bool,
}

/// Rate limit status for headers/logging
#[derive(Clone, Debug, serde::Serialize)]
pub struct RateLimitStatus {
    pub allowed: bool,
    pub current_count: u64,
    pub limit: u64,
    pub retry_after_secs: u64,
    /// The computed key used for rate limiting
    pub key: String,
    /// Optional semantic scope (e.g., "route", "global", "custom")
    pub scope: Option<String>,
}
