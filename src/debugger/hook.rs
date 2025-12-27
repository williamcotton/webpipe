use async_trait::async_trait;
use serde_json::Value;
use crate::ast::SourceLocation;
use crate::error::WebPipeError;

/// Zero-cost debugger hook trait
///
/// When debugger is None in ExecutionEnv, the Option check
/// compiles to a simple null pointer check with branch prediction,
/// resulting in near-zero overhead in production builds.
#[async_trait]
pub trait DebuggerHook: Send + Sync {
    /// Allocate a unique thread ID for a new HTTP request
    fn allocate_thread_id(&self) -> u64;

    /// Called before executing a pipeline step
    ///
    /// This hook is called with:
    /// - thread_id: Unique ID for this HTTP request (from atomic counter)
    /// - step_name: Name of the step being executed (e.g., "jq", "pg", "fetch")
    /// - location: Source location of the step in the .wp file
    /// - state: Current pipeline state (JSON value) - MUTABLE to allow debugger updates
    /// - stack: Full execution stack (e.g., ["GET /api", "pipeline:auth", "pg"])
    ///
    /// Returns StepAction indicating how execution should proceed
    async fn before_step(
        &self,
        thread_id: u64,
        step_name: &str,
        location: &SourceLocation,
        state: &mut Value,
        stack: Vec<String>,
    ) -> Result<StepAction, WebPipeError>;

    /// Called after executing a pipeline step
    ///
    /// This hook is called after successful step execution with the
    /// same parameters as before_step plus the updated state.
    async fn after_step(
        &self,
        thread_id: u64,
        step_name: &str,
        location: &SourceLocation,
        state: &Value,
        stack: Vec<String>,
    );
}

/// Action to take after before_step hook
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepAction {
    /// Continue execution normally
    Continue,
    /// Step over: pause at next step in same pipeline
    StepOver,
    /// Step in: pause at next step (entering nested pipelines)
    StepIn,
    /// Step out: pause when returning to parent pipeline
    StepOut,
}