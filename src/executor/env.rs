use std::{collections::HashMap, path::PathBuf, sync::Arc};
use async_trait::async_trait;
use serde_json::Value;

use crate::{
    ast::{Pipeline, Variable},
    error::WebPipeError,
    middleware::MiddlewareRegistry,
    runtime::context::{CacheStore, RateLimitStore},
};

use super::context::RequestContext;
use super::modules::{ModuleId, ModuleRegistry};

#[async_trait]
pub trait MiddlewareInvoker: Send + Sync {
    async fn call(
        &self,
        name: &str,
        args: &[String],
        cfg: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &ExecutionEnv,
        ctx: &mut RequestContext,
        target_name: Option<&str>,
    ) -> Result<(), WebPipeError>;

    /// Look up a mock value for a specific target (e.g. "query.users")
    /// Returns None for RealInvoker, Some(mock_value) for MockingInvoker in tests
    fn get_mock(&self, _target: &str) -> Option<Value> {
        None // Default implementation returns None
    }
}

/// Global execution environment (The Toolbelt)
/// Immutable, shared across all requests
#[derive(Clone)]
pub struct ExecutionEnv {
    /// Variables for resolution and auto-naming
    /// Key: (module_id, var_type, name) where module_id is None for local symbols
    pub variables: Arc<HashMap<(Option<ModuleId>, String, String), Variable>>,

    /// Named pipelines registry
    /// Key: (module_id, name) where module_id is None for local symbols
    pub named_pipelines: Arc<HashMap<(Option<ModuleId>, String), Arc<Pipeline>>>,

    /// Module registry for context-aware resolution
    pub module_registry: Arc<ModuleRegistry>,

    /// Import map: alias -> resolved file path (DEPRECATED - kept for backward compat during transition)
    pub imports: Arc<HashMap<String, PathBuf>>,

    /// Middleware invoker
    pub invoker: Arc<dyn MiddlewareInvoker>,

    /// Middleware registry (for accessing behavior metadata)
    pub registry: Arc<MiddlewareRegistry>,

    /// Environment name (e.g. "production", "development", "staging")
    pub environment: Option<String>,

    /// Global cache store (shared across requests)
    pub cache: CacheStore,

    /// Global rate limit store (shared across requests)
    pub rate_limit: RateLimitStore,

    /// Optional debugger hook (None = zero cost in production)
    /// When enabled, before_step/after_step are called for each pipeline step
    #[cfg(feature = "debugger")]
    pub debugger: Option<std::sync::Arc<dyn crate::debugger::DebuggerHook>>,
}

#[derive(Clone)]
pub struct RealInvoker {
    registry: Arc<MiddlewareRegistry>,
}

impl RealInvoker {
    pub fn new(registry: Arc<MiddlewareRegistry>) -> Self { Self { registry } }
}

#[async_trait]
impl MiddlewareInvoker for RealInvoker {
    async fn call(
        &self,
        name: &str,
        args: &[String],
        cfg: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &ExecutionEnv,
        ctx: &mut RequestContext,
        target_name: Option<&str>,
    ) -> Result<(), WebPipeError> {
        self.registry.execute(name, args, cfg, pipeline_ctx, env, ctx, target_name).await
    }
}
