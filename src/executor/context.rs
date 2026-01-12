use std::collections::HashMap;
use serde_json::Value;

use crate::executor::env::ExecutionEnv;
use crate::executor::tasks::AsyncTaskRegistry;
use crate::executor::types::{CachePolicy, LogConfig, RateLimitStatus};

#[derive(Default)]
pub struct RequestMetadata {
    pub start_time: Option<std::time::Instant>,
    pub cache_policy: Option<CachePolicy>,
    pub log_config: Option<LogConfig>,
    pub rate_limit_status: Option<RateLimitStatus>,
}

impl RequestMetadata {
    /// Convert metadata to a JSON Value for context injection
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "cache": self.cache_policy.as_ref().map(|cp| serde_json::json!({
                "enabled": cp.enabled,
                "ttl": cp.ttl,
                "key_template": cp.key_template,
            })),
            "log": self.log_config.as_ref().map(|lc| serde_json::json!({
                "level": lc.level,
                "include_body": lc.include_body,
                "include_headers": lc.include_headers,
            })),
            "rate_limit": self.rate_limit_status.as_ref().map(|rl| serde_json::json!({
                "allowed": rl.allowed,
                "remaining": rl.limit.saturating_sub(rl.current_count),
                "limit": rl.limit,
                "retry_after": rl.retry_after_secs,
                "key": rl.key,
                "scope": rl.scope,
            })),
        })
    }
}

/// Debug stepping modes for step-by-step execution
#[cfg(feature = "debugger")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    /// Stop at next step in same pipeline (don't enter nested pipelines)
    StepOver,
    /// Stop at next step (enter nested pipelines)
    StepIn,
    /// Stop when returning to parent pipeline
    StepOut,
}

/// Profiler for tracking execution timing and generating folded stack traces
#[derive(Clone, Debug, Default)]
pub struct Profiler {
    /// Current call stack (e.g., ["GET /api", "pipeline:auth", "pg"])
    pub stack: Vec<String>,
    /// Recorded samples: (folded_stack_string, duration_micros)
    pub samples: Vec<(String, u128)>,
}

impl Profiler {
    pub fn push(&mut self, name: &str) {
        self.stack.push(name.to_string());
    }

    pub fn pop(&mut self) {
        self.stack.pop();
    }

    pub fn record_sample(&mut self, duration_micros: u128) {
        if self.stack.is_empty() { return; }
        let stack_str = self.stack.join(";");
        self.samples.push((stack_str, duration_micros));
    }
}

/// Per-request mutable context (The Backpack)
pub struct RequestContext {
    /// Feature flags for this request (extracted from headers or auth, NOT from user JSON)
    /// Static/Sticky configuration (e.g., ENABLE_BETA, NEW_UI). Controlled via @flag / getFlag / setFlag.
    pub feature_flags: HashMap<String, bool>,

    /// Transient conditions for @when routing
    /// Dynamic/Transient request state (e.g., is_admin, json_request). Controlled via @when / setWhen.
    pub conditions: HashMap<String, bool>,

    /// Async task registry for @async steps
    pub async_registry: AsyncTaskRegistry,

    /// Deferred actions to run at pipeline completion
    pub deferred: Vec<Box<dyn FnOnce(&Value, &str, &ExecutionEnv) + Send>>,

    /// Typed metadata for middleware communication
    pub metadata: RequestMetadata,

    /// Log of calls to resolvers/middleware for assertion (spying)
    /// Key: "query.users", Value: List of argument objects passed to that resolver
    pub call_log: HashMap<String, Vec<Value>>,

    /// Profiler for tracking execution timing
    pub profiler: Profiler,

    /// Persistent request information (query, params, headers) that survives state transformations
    pub request: Value,

    /// Debug thread ID for DAP protocol (None in production)
    #[cfg(feature = "debugger")]
    pub debug_thread_id: Option<u64>,

    /// Debug stepping mode (StepOver pauses at next step)
    #[cfg(feature = "debugger")]
    pub debug_step_mode: Option<StepMode>,
}

impl RequestContext {
    pub fn new() -> Self {
        Self {
            feature_flags: HashMap::new(),
            conditions: HashMap::new(),
            async_registry: AsyncTaskRegistry::new(),
            deferred: Vec::new(),
            metadata: RequestMetadata::default(),
            call_log: HashMap::new(),
            profiler: Profiler::default(),
            request: Value::Null,
            #[cfg(feature = "debugger")]
            debug_thread_id: None,
            #[cfg(feature = "debugger")]
            debug_step_mode: None,
        }
    }

    /// Register a deferred action to run at pipeline completion
    pub fn defer<F>(&mut self, f: F)
    where
        F: FnOnce(&Value, &str, &ExecutionEnv) + Send + 'static,
    {
        self.deferred.push(Box::new(f));
    }

    /// Execute all deferred actions (called by server after pipeline completes)
    pub fn run_deferred(mut self, final_result: &Value, content_type: &str, env: &ExecutionEnv) {
        for action in self.deferred.drain(..) {
            action(final_result, content_type, env);
        }
    }

    /// Convert RequestContext to a JSON Value for middleware injection
    /// This provides read-only access to system metadata for user scripts
    pub fn to_value(&self, env: &ExecutionEnv) -> serde_json::Value {
        let mut context = serde_json::Map::new();

        // Add feature flags
        let flags: serde_json::Map<String, serde_json::Value> = self.feature_flags
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::Bool(*v)))
            .collect();
        context.insert("flags".to_string(), serde_json::Value::Object(flags));

        // Add conditions (transient request state for @when routing)
        let conditions: serde_json::Map<String, serde_json::Value> = self.conditions
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::Bool(*v)))
            .collect();
        context.insert("conditions".to_string(), serde_json::Value::Object(conditions));

        // Add persistent request info
        if !self.request.is_null() {
            context.insert("request".to_string(), self.request.clone());
        }

        // Add environment name
        if let Some(env_name) = &env.environment {
            context.insert("env".to_string(), serde_json::Value::String(env_name.clone()));
        }

        // Add metadata (cache, log, rate_limit)
        let metadata_json = self.metadata.to_value();
        if let Some(cache) = metadata_json.get("cache") {
            if !cache.is_null() {
                context.insert("cache".to_string(), cache.clone());
            }
        }
        if let Some(log) = metadata_json.get("log") {
            if !log.is_null() {
                context.insert("log".to_string(), log.clone());
            }
        }
        if let Some(rate_limit) = metadata_json.get("rate_limit") {
            if !rate_limit.is_null() {
                context.insert("rate_limit".to_string(), rate_limit.clone());
            }
        }

        serde_json::Value::Object(context)
    }
}

impl Default for RequestContext {
    fn default() -> Self {
        Self::new()
    }
}
