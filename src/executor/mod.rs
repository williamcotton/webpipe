use std::{collections::HashMap, pin::Pin, sync::Arc, future::Future};

use async_trait::async_trait;
use serde_json::Value;
use tokio::task::JoinHandle;
use parking_lot::Mutex;
use async_recursion::async_recursion;

use crate::{
    ast::{Pipeline, PipelineStep, Variable},
    error::WebPipeError,
    middleware::MiddlewareRegistry,
    runtime::context::{CacheStore, RateLimitStore},
    runtime::json_path,
};

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
enum StepOutcome {
    /// Continue to the next step
    Continue,
    /// Stop execution and return the pipeline result immediately
    Return(PipelineOutput),
}

/// Represents different execution modes for pipeline steps
#[derive(Debug)]
enum ExecutionMode {
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
struct StepResult {
    value: Value,
    content_type: String,
    status_code: Option<u16>,
}

/// Public alias for the complex future type returned by pipeline execution functions.
pub type PipelineExecFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<(Value, String, Option<u16>), WebPipeError>> + Send + 'a,
    >,
>;

#[async_trait]
pub trait MiddlewareInvoker: Send + Sync {
    async fn call(
        &self,
        name: &str,
        cfg: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &ExecutionEnv,
        ctx: &mut RequestContext,
    ) -> Result<(), WebPipeError>;

    /// Look up a mock value for a specific target (e.g. "query.users")
    /// Returns None for RealInvoker, Some(mock_value) for MockingInvoker in tests
    fn get_mock(&self, _target: &str) -> Option<Value> {
        None // Default implementation returns None
    }
}

/// Registry for async tasks spawned with @async tag
#[derive(Clone)]
pub struct AsyncTaskRegistry {
    tasks: Arc<Mutex<HashMap<String, JoinHandle<Result<Value, WebPipeError>>>>>,
}

impl AsyncTaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register(&self, name: String, handle: JoinHandle<Result<Value, WebPipeError>>) {
        self.tasks.lock().insert(name, handle);
    }

    pub fn take(&self, name: &str) -> Option<JoinHandle<Result<Value, WebPipeError>>> {
        self.tasks.lock().remove(name)
    }
}

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

/// Global execution environment (The Toolbelt)
/// Immutable, shared across all requests
#[derive(Clone)]
pub struct ExecutionEnv {
    /// Variables for resolution and auto-naming
    pub variables: Arc<Vec<Variable>>,

    /// Named pipelines registry
    pub named_pipelines: Arc<HashMap<String, Arc<Pipeline>>>,

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
        cfg: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &ExecutionEnv,
        ctx: &mut RequestContext,
    ) -> Result<(), WebPipeError> {
        self.registry.execute(name, cfg, pipeline_ctx, env, ctx).await
    }
}

fn resolve_config_and_autoname(
    variables: &[Variable],
    middleware_name: &str,
    step_config: &str,
    input: &Value,
) -> (String, Value, bool) {
    if let Some(var) = variables.iter().find(|v| v.var_type == middleware_name && v.name == step_config) {
        let resolved_config = var.value.clone();
        let mut new_input = input.clone();
        let mut auto_named = false;
        if let Some(obj) = new_input.as_object_mut() {
            if !obj.contains_key("resultName") {
                obj.insert("resultName".to_string(), Value::String(var.name.clone()));
                auto_named = true;
            }
        }
        return (resolved_config, new_input, auto_named);
    }
    (step_config.to_string(), input.clone(), false)
}

fn extract_error_type(input: &Value) -> Option<String> {
    if let Some(errors) = input.get("errors") {
        if let Some(errors_array) = errors.as_array() {
            if let Some(first_error) = errors_array.first() {
                if let Some(error_type) = first_error.get("type").and_then(|v| v.as_str()) {
                    return Some(error_type.to_string());
                }
            }
        }
    }
    None
}

fn should_execute_step(condition: &Option<crate::ast::TagExpr>, env: &ExecutionEnv, ctx: &RequestContext, input: &Value) -> bool {
    match condition {
        None => true, // No condition means always execute
        Some(expr) => check_tag_expr(expr, env, ctx, input),
    }
}

fn check_tag(tag: &crate::ast::Tag, env: &ExecutionEnv, ctx: &RequestContext, input: &Value) -> bool {
    match tag.name.as_str() {
        "env" => check_env_tag(tag, env),
        "async" => true, // async doesn't affect execution, just how it runs
        "flag" => check_flag_tag(tag, ctx),
        "when" => check_when_tag(tag, ctx),
        "guard" => check_guard_tag(tag, input, tag.negated),
        "needs" => true, // @needs is only for static analysis, doesn't affect runtime
        _ => true,       // unknown tags don't prevent execution
    }
}

/// Check @when tag against transient conditions in RequestContext
/// All arguments must be true (AND logic). If tag.negated is true, invert the result.
fn check_when_tag(tag: &crate::ast::Tag, ctx: &RequestContext) -> bool {
    if tag.args.is_empty() {
        return true; // Invalid @when() tag without args, don't prevent execution
    }

    // Check all condition arguments - all must be met for step to execute
    for condition_name in &tag.args {
        let is_met = ctx.conditions.get(condition_name.as_str())
            .copied()
            .unwrap_or(false); // Default: False (Fail Closed)

        let matches = if tag.negated {
            !is_met // @!when(admin) - execute if condition is NOT met
        } else {
            is_met  // @when(admin) - execute if condition IS met
        };

        // If any condition check fails, the step should not execute
        if !matches {
            return false;
        }
    }

    true
}

/// Evaluate a boolean tag expression (for dispatch routing)
/// Supports AND, OR operations with proper short-circuit evaluation
fn check_tag_expr(expr: &crate::ast::TagExpr, env: &ExecutionEnv, ctx: &RequestContext, input: &Value) -> bool {
    match expr {
        crate::ast::TagExpr::Tag(tag) => check_tag(tag, env, ctx, input),
        crate::ast::TagExpr::And(left, right) => {
            // Short-circuit: if left is false, don't evaluate right
            check_tag_expr(left, env, ctx, input) && check_tag_expr(right, env, ctx, input)
        }
        crate::ast::TagExpr::Or(left, right) => {
            // Short-circuit: if left is true, don't evaluate right
            check_tag_expr(left, env, ctx, input) || check_tag_expr(right, env, ctx, input)
        }
    }
}

fn check_flag_tag(tag: &crate::ast::Tag, ctx: &RequestContext) -> bool {
    if tag.args.is_empty() {
        return true; // Invalid @flag() tag without args, don't prevent execution
    }

    // Check all flag arguments - all must be enabled for step to execute
    for flag_name in &tag.args {
        let is_enabled = ctx.feature_flags.get(flag_name.as_str())
            .copied()
            .unwrap_or(false); // Default: False (Fail Closed)

        let matches = if tag.negated {
            !is_enabled // @!flag(beta) - execute if flag is NOT enabled
        } else {
            is_enabled  // @flag(beta) - execute if flag IS enabled
        };

        // If any flag check fails, the step should not execute
        if !matches {
            return false;
        }
    }

    true
}

fn check_env_tag(tag: &crate::ast::Tag, env: &ExecutionEnv) -> bool {
    if tag.args.len() != 1 {
        return true; // Invalid @env() tag, don't prevent execution
    }

    let required_env = &tag.args[0];

    match &env.environment {
        Some(current_env) => {
            let matches = current_env == required_env;
            if tag.negated {
                !matches // @!env(production) - execute if NOT production
            } else {
                matches  // @env(production) - execute if IS production
            }
        }
        None => {
            // No environment set: execute non-negated tags, skip negated ones
            !tag.negated
        }
    }
}

/// Check @guard tag - evaluates a JQ expression against the pipeline input
/// Returns true if the step should execute, false otherwise
///
/// # Examples
/// - `@guard(.user.role == "admin")` - execute only if user is admin
/// - `@!guard(.debug)` - execute only if debug is NOT truthy
fn check_guard_tag(tag: &crate::ast::Tag, input: &Value, negated: bool) -> bool {
    // 1. Extract JQ filter from args
    let filter = match tag.args.first() {
        Some(f) => f,
        None => {
            tracing::warn!("@guard tag without filter argument - defaulting to true");
            return true; // Guard without arg defaults to true
        }
    };

    // 2. Run JQ logic (uses thread-local cache for performance)
    match crate::middleware::jq_eval_bool(filter, input) {
        Ok(result) => {
            // Apply negation if present
            if negated {
                !result
            } else {
                result
            }
        }
        Err(e) => {
            // Log error and fail closed (don't execute) for safety
            tracing::warn!("Guard JQ error: {} - failing closed (skipping step)", e);
            false
        }
    }
}

/// Extract @async(name) from a TagExpr, walking the expression tree
fn get_async_from_tag_expr(expr: &crate::ast::TagExpr) -> Option<String> {
    match expr {
        crate::ast::TagExpr::Tag(tag) => {
            if tag.name == "async" && !tag.negated && tag.args.len() == 1 {
                Some(tag.args[0].clone())
            } else {
                None
            }
        }
        crate::ast::TagExpr::And(left, right) | crate::ast::TagExpr::Or(left, right) => {
            get_async_from_tag_expr(left).or_else(|| get_async_from_tag_expr(right))
        }
    }
}

/// Check for cache control signal from cache middleware.
/// Returns Some((cached_value, optional_content_type)) if cache hit occurred and pipeline should stop.
fn check_cache_control_signal(input: &Value) -> Option<(Value, Option<String>)> {
    input.get("_control")
        .and_then(|c| {
            if c.get("stop").and_then(|b| b.as_bool()).unwrap_or(false) {
                let value = c.get("value").cloned()?;
                let content_type = c.get("content_type").and_then(|ct| ct.as_str()).map(|s| s.to_string());
                Some((value, content_type))
            } else {
                None
            }
        })
}


fn parse_join_task_names(config: &str) -> Result<Vec<String>, WebPipeError> {
    let trimmed = config.trim();

    // Try parsing as JSON array first
    if trimmed.starts_with('[') {
        match serde_json::from_str::<Vec<String>>(trimmed) {
            Ok(names) => return Ok(names),
            Err(_) => {
                return Err(WebPipeError::ConfigError(
                    "Invalid JSON array for join config".to_string()
                ));
            }
        }
    }

    // Otherwise parse as comma-separated list
    let names: Vec<String> = trimmed
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if names.is_empty() {
        return Err(WebPipeError::ConfigError(
            "join config must specify at least one task name".to_string()
        ));
    }

    Ok(names)
}

fn select_branch<'a>(
    branches: &'a [crate::ast::ResultBranch],
    error_type: &Option<String>,
) -> Option<&'a crate::ast::ResultBranch> {
    use crate::ast::ResultBranchType;

    if let Some(err_type) = error_type {
        if let Some(branch) = branches.iter().find(|b| matches!(&b.branch_type, ResultBranchType::Custom(name) if name == err_type)) {
            return Some(branch);
        }
    } else if let Some(branch) = branches.iter().find(|b| matches!(b.branch_type, ResultBranchType::Ok)) {
        return Some(branch);
    }

    branches
        .iter()
        .find(|b| matches!(b.branch_type, ResultBranchType::Default))
}

/// Determine the execution mode for a step
fn detect_execution_mode(name: &str, condition: &Option<crate::ast::TagExpr>) -> ExecutionMode {
    // Check for async tag first
    if let Some(ref expr) = condition {
        if let Some(async_name) = get_async_from_tag_expr(expr) {
            return ExecutionMode::Async(async_name);
        }
    }

    // Check for special middleware types
    match name {
        "join" => ExecutionMode::Join,
        "pipeline" => ExecutionMode::Recursive,
        _ => ExecutionMode::Standard,
    }
}

/// Update pipeline state after a step execution
///
/// Handles the complex logic of whether to merge or replace state based on middleware behavior:
/// - Merge: Default behavior for most middleware (Backpack semantics)
/// - Transform: Replace state entirely, but preserve system context if not terminal
/// - Render: Replace state entirely (for final output like HTML/Text)
/// - ReadOnly: No state modification needed (handled by middleware)
fn update_pipeline_state(
    pipeline_ctx: &mut crate::runtime::PipelineContext,
    new_value: Value,
    behavior: crate::middleware::StateBehavior,
    is_last_step: bool,
) {
    use crate::middleware::StateBehavior;

    match behavior {
        StateBehavior::Merge => {
            pipeline_ctx.merge_state(new_value);
        }
        StateBehavior::Transform => {
            if is_last_step {
                // If it's the last step, just replace everything
                pipeline_ctx.replace_state(new_value);
            } else {
                // For non-terminal transforms, preserve runtime context keys
                let backups = pipeline_ctx.backup_system_keys();
                pipeline_ctx.replace_state(new_value);
                pipeline_ctx.restore_system_keys(backups);
            }
        }
        StateBehavior::Render => {
            // Render always replaces content entirely
            pipeline_ctx.replace_state(new_value);
        }
        StateBehavior::ReadOnly => {
            // ReadOnly middleware don't modify state through this path
            // (they may have side effects but don't change pipeline state)
            // In practice, they shouldn't call this function, but if they do, do nothing
        }
    }
}

/// Check if this is effectively the last step that will execute
/// (accounts for remaining steps being skipped due to feature flags, etc.)
fn is_effectively_last_step(
    current_idx: usize,
    steps: &[PipelineStep],
    env: &ExecutionEnv,
    ctx: &RequestContext,
    state: &Value,
) -> bool {
    // Check if any remaining steps will execute
    for remaining_step in steps.iter().skip(current_idx + 1) {
        match remaining_step {
            PipelineStep::Regular { condition, .. } => {
                // If this remaining step would execute, current is not the last
                if should_execute_step(condition, env, ctx, state) {
                    return false;
                }
            }
            PipelineStep::Result { .. } => {
                // Result branches always execute, so current is not last
                return false;
            }
            PipelineStep::If { .. } => {
                // If blocks always execute (condition is always evaluated), so current is not last
                return false;
            }
            PipelineStep::Dispatch { .. } => {
                // Dispatch blocks always execute (at least default case), so current is not last
                return false;
            }
            PipelineStep::Foreach { .. } => {
                // Foreach blocks always execute, so current is not last
                return false;
            }
        }
    }
    // No remaining steps will execute
    true
}

/// Handle join operation - wait for async tasks and merge results
/// Uses pre-parsed task names when available (compile-time optimization)
async fn handle_join(
    ctx: &mut RequestContext,
    config: &str,
    parsed_targets: Option<&Vec<String>>,
    input: Value,
) -> Result<StepResult, WebPipeError> {
    // Use pre-parsed targets if available, otherwise parse at runtime (fallback)
    let task_names: Vec<String> = match parsed_targets {
        Some(targets) => targets.clone(),
        None => parse_join_task_names(config)?,
    };

    // Wait for all async tasks
    let mut async_results = serde_json::Map::new();
    for task_name in task_names {
        if let Some(handle) = ctx.async_registry.take(&task_name) {
            match handle.await {
                Ok(Ok(result)) => {
                    async_results.insert(task_name, result);
                }
                Ok(Err(e)) => {
                    // Task failed - store error representation
                    async_results.insert(task_name, serde_json::json!({
                        "error": e.to_string()
                    }));
                }
                Err(e) => {
                    // Join error (task panicked)
                    async_results.insert(task_name, serde_json::json!({
                        "error": format!("Task panicked: {}", e)
                    }));
                }
            }
        }
    }

    // Merge results into input under .async key
    let mut result = input.clone();
    if let Some(obj) = result.as_object_mut() {
        obj.insert("async".to_string(), Value::Object(async_results));
    }

    Ok(StepResult {
        value: result,
        content_type: "application/json".to_string(),
        status_code: None,
    })
}

/// Spawn an async task without waiting for it
fn spawn_async_step(
    env: &ExecutionEnv,
    ctx: &mut RequestContext,
    name: &str,
    config: &str,
    input: Value,
    async_name: String,
) {
    let env_clone = env.clone();
    let name_clone = name.to_string();
    let config_clone = config.to_string();
    let input_snapshot = input;

    let handle = tokio::spawn(async move {
        let (effective_config, effective_input, _auto_named) =
            resolve_config_and_autoname(&env_clone.variables, &name_clone, &config_clone, &input_snapshot);

        // Create a new RequestContext for the async task
        let mut async_ctx = RequestContext::new();

        if name_clone == "pipeline" {
            let pipeline_name = effective_config.trim();
            if let Some(p) = env_clone.named_pipelines.get(pipeline_name) {
                let (val, _ct, _st, _) = execute_pipeline(&env_clone, p, effective_input.clone(), async_ctx).await?;
                Ok(val)
            } else {
                Err(WebPipeError::PipelineNotFound(pipeline_name.to_string()))
            }
        } else {
            // Create a PipelineContext for the async task
            let mut pipeline_ctx = crate::runtime::PipelineContext::new(effective_input);
            env_clone.invoker.call(&name_clone, &effective_config, &mut pipeline_ctx, &env_clone, &mut async_ctx).await?;
            Ok(pipeline_ctx.state)
        }
    });

    ctx.async_registry.register(async_name, handle);
}

/// Execute a named pipeline recursively
fn handle_recursive_pipeline<'a>(
    env: &'a ExecutionEnv,
    ctx: &'a mut RequestContext,
    config: &'a str,
    input: Value,
) -> Pin<Box<dyn Future<Output = Result<StepResult, WebPipeError>> + Send + 'a>> {
    Box::pin(async move {
        let pipeline_name = config.trim();
        if let Some(pipeline) = env.named_pipelines.get(pipeline_name) {
            // Reuse the same context for recursive pipeline execution
            let (value, content_type, status_code) = execute_pipeline_internal(env, pipeline, input, ctx).await?;
            Ok(StepResult {
                value,
                content_type,
                status_code,
            })
        } else {
            Err(WebPipeError::PipelineNotFound(pipeline_name.to_string()))
        }
    })
}

/// Execute standard middleware
async fn handle_standard_execution(
    env: &ExecutionEnv,
    ctx: &mut RequestContext,
    name: &str,
    config: &str,
    pipeline_ctx: &mut crate::runtime::PipelineContext,
) -> Result<StepResult, WebPipeError> {
    env.invoker.call(name, config, pipeline_ctx, env, ctx).await?;

    // Special case: handlebars sets HTML content type
    let content_type = if name == "handlebars" {
        "text/html".to_string()
    } else {
        "application/json".to_string()
    };

    Ok(StepResult {
        value: pipeline_ctx.state.clone(),
        content_type,
        status_code: None,
    })
}

/// Dispatch to the appropriate step handler
async fn execute_step<'a>(
    step: &'a PipelineStep,
    idx: usize,
    all_steps: &'a [PipelineStep],
    env: &'a ExecutionEnv,
    ctx: &'a mut RequestContext,
    pipeline_ctx: &mut crate::runtime::PipelineContext,
    current_output: &mut PipelineOutput,
) -> Result<StepOutcome, WebPipeError> {
    match step {
        PipelineStep::Regular { name, config, condition, parsed_join_targets, .. } => {
            execute_regular_step(
                name,
                config,
                condition,
                parsed_join_targets.as_ref(),
                idx,
                all_steps,
                env,
                ctx,
                pipeline_ctx,
                current_output,
            ).await
        }
        PipelineStep::Result { branches } => {
            // Handle result step - returns immediately
            let error_type = extract_error_type(&pipeline_ctx.state);
            let selected = select_branch(branches, &error_type);
            if let Some(branch) = selected {
                let inherited_cookies = pipeline_ctx.state.get("setCookies").cloned();
                let (mut result, branch_content_type, _status) = execute_pipeline_internal(
                    env,
                    &branch.pipeline,
                    pipeline_ctx.state.clone(),
                    ctx
                ).await?;
                if inherited_cookies.is_some() {
                    if let Some(obj) = result.as_object_mut() {
                        if !obj.contains_key("setCookies") {
                            if let Some(c) = inherited_cookies {
                                obj.insert("setCookies".to_string(), c);
                            }
                        }
                    }
                }
                let final_content_type = if branch_content_type != "application/json" {
                    branch_content_type
                } else {
                    current_output.content_type.clone()
                };
                Ok(StepOutcome::Return(PipelineOutput {
                    state: result,
                    content_type: final_content_type,
                    status_code: Some(branch.status_code),
                }))
            } else {
                Ok(StepOutcome::Return(PipelineOutput {
                    state: pipeline_ctx.state.clone(),
                    content_type: current_output.content_type.clone(),
                    status_code: current_output.status_code,
                }))
            }
        }
        PipelineStep::If { condition, then_branch, else_branch } => {
            // 1. Run Condition on Cloned State
            let (cond_result, _, _) = execute_pipeline_internal(
                env,
                condition,
                pipeline_ctx.state.clone(),
                ctx
            ).await?;

            // 2. Determine Truthiness
            let is_truthy = match cond_result {
                Value::Bool(b) => b,
                Value::Null => false,
                _ => true,
            };

            // 3. Execute Branch
            if is_truthy {
                // Run THEN branch with ORIGINAL state
                let (res, ct, status) = execute_pipeline_internal(
                    env,
                    then_branch,
                    pipeline_ctx.state.clone(),
                    ctx
                ).await?;

                // Update Context
                pipeline_ctx.state = res;
                if let Some(s) = status {
                    current_output.status_code = Some(s);
                }
                if ct != "application/json" {
                    current_output.content_type = ct;
                }
            } else if let Some(else_pipe) = else_branch {
                // Run ELSE branch with ORIGINAL state
                let (res, ct, status) = execute_pipeline_internal(
                    env,
                    else_pipe,
                    pipeline_ctx.state.clone(),
                    ctx
                ).await?;

                // Update Context
                pipeline_ctx.state = res;
                if let Some(s) = status {
                    current_output.status_code = Some(s);
                }
                if ct != "application/json" {
                    current_output.content_type = ct;
                }
            }
            // If false and no else: state remains unchanged (pass-through)

            Ok(StepOutcome::Continue)
        }
        PipelineStep::Dispatch { branches, default } => {
            // Iterate through branches and find the first matching condition
            let mut matched_pipeline = None;

            for branch in branches {
                // Check if this branch's condition expression matches
                if check_tag_expr(&branch.condition, env, ctx, &pipeline_ctx.state) {
                    matched_pipeline = Some(&branch.pipeline);
                    break;
                }
            }

            // If no case matched, use default
            if matched_pipeline.is_none() {
                matched_pipeline = default.as_ref();
            }

            // Execute the matched pipeline (or do nothing if no match and no default)
            if let Some(pipeline) = matched_pipeline {
                let (res, ct, status) = execute_pipeline_internal(
                    env,
                    pipeline,
                    pipeline_ctx.state.clone(),
                    ctx
                ).await?;

                // Update Context
                pipeline_ctx.state = res;
                if let Some(s) = status {
                    current_output.status_code = Some(s);
                }
                if ct != "application/json" {
                    current_output.content_type = ct;
                }
            }
            // If no match and no default: state remains unchanged (pass-through)

            Ok(StepOutcome::Continue)
        }
        PipelineStep::Foreach { selector, pipeline } => {
            // SURGICAL EXTRACTION PATTERN
            // 1. EXTRACTION
            // Use json_path helper to find the node and .take() it.
            // This leaves 'Null' in the tree at the selector path.
            let items_opt = json_path::get_value_mut(&mut pipeline_ctx.state, selector)
                .map(|val| val.take());

            if let Some(Value::Array(items)) = items_opt {
                let mut results = Vec::with_capacity(items.len());

                // 2. ITERATION
                for item in items {
                    // Execute the inner pipeline with the item as input.
                    // This creates isolated context for each iteration.
                    let (final_state, _, _) = execute_pipeline_internal(
                        env,
                        pipeline,
                        item,
                        ctx
                    ).await?;

                    results.push(final_state);
                }

                // 3. IMPLANTATION
                // Find the path again (since our mutable borrow ended) and
                // overwrite the 'Null' with our new Array of results.
                if let Some(slot) = json_path::get_value_mut(&mut pipeline_ctx.state, selector) {
                    *slot = Value::Array(results);
                }
            } else {
                // Path not found or not an array.
                // Fail fast so the user knows their path is wrong.
                return Err(WebPipeError::MiddlewareExecutionError(
                    format!("foreach: path '{}' is not an array or does not exist", selector)
                ));
            }

            Ok(StepOutcome::Continue)
        }
    }
}

/// Execute a regular pipeline step
async fn execute_regular_step(
    name: &str,
    config: &str,
    condition: &Option<crate::ast::TagExpr>,
    parsed_join_targets: Option<&Vec<String>>,
    idx: usize,
    all_steps: &[PipelineStep],
    env: &ExecutionEnv,
    ctx: &mut RequestContext,
    pipeline_ctx: &mut crate::runtime::PipelineContext,
    current_output: &mut PipelineOutput,
) -> Result<StepOutcome, WebPipeError> {
    // 1. Guard Checks: should_execute_step
    if !should_execute_step(condition, env, ctx, &pipeline_ctx.state) {
        return Ok(StepOutcome::Continue);
    }

    // Check if this is effectively the last step (no remaining steps will execute)
    let is_last_step = is_effectively_last_step(idx, all_steps, env, ctx, &pipeline_ctx.state);

    // 2. Determine Execution Mode
    let mode = detect_execution_mode(name, condition);

    // Handle async execution separately (no state merge needed)
    if let ExecutionMode::Async(async_name) = mode {
        spawn_async_step(env, ctx, name, config, pipeline_ctx.state.clone(), async_name);
        return Ok(StepOutcome::Continue);
    }

    // 3. Config Resolution: resolve_config_and_autoname. Handle resultName injection.
    let (effective_config, effective_input, auto_named) =
        resolve_config_and_autoname(&env.variables, name, config, &pipeline_ctx.state);

    // If auto-named, update the pipeline context state with resultName
    if auto_named {
        pipeline_ctx.state = effective_input.clone();
    }

    // 4. Execution Mode: Switch on Standard, Join, Recursive
    let step_result = match mode {
        ExecutionMode::Join => {
            // Use pre-parsed targets for hot path optimization
            handle_join(ctx, &effective_config, parsed_join_targets, effective_input.clone()).await?
        }
        ExecutionMode::Recursive => {
            handle_recursive_pipeline(env, ctx, &effective_config, effective_input.clone()).await?
        }
        ExecutionMode::Standard => {
            // Standard execution - pass mutable context
            let mut temp_ctx = crate::runtime::PipelineContext::new(pipeline_ctx.state.clone());
            handle_standard_execution(env, ctx, name, &effective_config, &mut temp_ctx).await?;
            StepResult {
                value: temp_ctx.state,
                content_type: if name == "handlebars" { "text/html".to_string() } else { "application/json".to_string() },
                status_code: None,
            }
        }
        ExecutionMode::Async(_) => unreachable!("Async handled above"),
    };

    // 5. State Update: Call update_pipeline_state with behavior from registry
    let behavior = env.registry.get_behavior(name).unwrap_or(crate::middleware::StateBehavior::Merge);
    update_pipeline_state(pipeline_ctx, step_result.value, behavior, is_last_step);

    // 6. Metadata Update: Update content_type and status_code in current_output
    if step_result.content_type != "application/json" || is_last_step {
        if name == "pipeline" || is_last_step || name == "handlebars" {
            current_output.content_type = step_result.content_type;
        }
    }
    if let Some(status) = step_result.status_code {
        current_output.status_code = Some(status);
    }

    // 7. Cache Check: Check for cache stop signals. If found, return StepOutcome::Return.
    if let Some((cached_val, cached_content_type)) = check_cache_control_signal(&pipeline_ctx.state) {
        let final_content_type = cached_content_type.unwrap_or_else(|| current_output.content_type.clone());
        return Ok(StepOutcome::Return(PipelineOutput {
            state: cached_val,
            content_type: final_content_type,
            status_code: current_output.status_code,
        }));
    }

    // 8. Cleanup: Remove resultName if auto-named
    if auto_named {
        if let Some(obj) = pipeline_ctx.state.as_object_mut() {
            obj.remove("resultName");
        }
    }

    Ok(StepOutcome::Continue)
}

/// Internal pipeline execution that accepts a mutable RequestContext
/// Public to allow GraphQL runtime to reuse the same context for call logging
#[async_recursion]
pub async fn execute_pipeline_internal<'a>(
    env: &'a ExecutionEnv,
    pipeline: &'a Pipeline,
    input: Value,
    ctx: &'a mut RequestContext,
) -> Result<(Value, String, Option<u16>), WebPipeError> {
    // Track content_type and status_code as we execute steps
    let mut content_type = "application/json".to_string();
    let mut status_code: Option<u16> = None;

    let mut pipeline_ctx = crate::runtime::PipelineContext::new(input);

    for (idx, step) in pipeline.steps.iter().enumerate() {
        // Create output tracker for this iteration
        let mut current_output = PipelineOutput {
            state: Value::Null, // Not used during step execution
            content_type: content_type.clone(),
            status_code,
        };

        let outcome = execute_step(
            step,
            idx,
            &pipeline.steps,
            env,
            ctx,
            &mut pipeline_ctx,
            &mut current_output,
        ).await?;

        // Update tracked metadata
        content_type = current_output.content_type;
        status_code = current_output.status_code;

        match outcome {
            StepOutcome::Continue => continue,
            StepOutcome::Return(output) => return Ok((output.state, output.content_type, output.status_code)),
        }
    }

    // Return final result with accumulated state and metadata
    Ok((pipeline_ctx.state, content_type, status_code))
}

/// Public entry point for pipeline execution
/// Executes pipeline and returns the context so deferred actions can be run
pub async fn execute_pipeline<'a>(
    env: &'a ExecutionEnv,
    pipeline: &'a Pipeline,
    input: Value,
    mut ctx: RequestContext,
) -> Result<(Value, String, Option<u16>, RequestContext), WebPipeError> {
    let result = execute_pipeline_internal(env, pipeline, input, &mut ctx).await?;
    Ok((result.0, result.1, result.2, ctx))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{PipelineStep, Pipeline, TagExpr, Tag};
    use std::sync::Arc;

    /// Helper to create a condition from a single tag
    fn single_tag(name: &str, negated: bool, args: Vec<&str>) -> Option<TagExpr> {
        Some(TagExpr::Tag(Tag {
            name: name.to_string(),
            negated,
            args: args.into_iter().map(|s| s.to_string()).collect(),
        }))
    }

    /// Helper to create an AND condition from two tags
    fn and_tags(tag1: TagExpr, tag2: TagExpr) -> Option<TagExpr> {
        Some(TagExpr::And(Box::new(tag1), Box::new(tag2)))
    }

    struct StubInvoker;
    #[async_trait]
    impl MiddlewareInvoker for StubInvoker {
        async fn call(
            &self,
            name: &str,
            cfg: &str,
            pipeline_ctx: &mut crate::runtime::PipelineContext,
            _env: &ExecutionEnv,
            _ctx: &mut RequestContext,
        ) -> Result<(), WebPipeError> {
            let input = &pipeline_ctx.state;
            match name {
                "handlebars" => {
                    pipeline_ctx.state = Value::String(format!("<p>{}</p>", cfg));
                }
                "echo" => {
                    // Try to parse config as JSON and merge with input
                    if let Ok(json_cfg) = serde_json::from_str::<Value>(cfg) {
                        // Merge config into input (config takes precedence)
                        let mut result = input.clone();
                        if let (Some(input_obj), Some(cfg_obj)) = (result.as_object_mut(), json_cfg.as_object()) {
                            for (k, v) in cfg_obj {
                                input_obj.insert(k.clone(), v.clone());
                            }
                        }
                        pipeline_ctx.state = result;
                    } else {
                        pipeline_ctx.state = serde_json::json!({"echo": cfg, "inputCopy": input });
                    }
                },
                _ => {
                    pipeline_ctx.state = serde_json::json!({"ok": true});
                }
            }
            Ok(())
        }
    }

    fn env_with_vars(vars: Vec<Variable>) -> ExecutionEnv {
        let registry = Arc::new(MiddlewareRegistry::empty());
        ExecutionEnv {
            variables: Arc::new(vars),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
            registry,
            environment: None,

            cache: CacheStore::new(8, 60),
            rate_limit: crate::runtime::context::RateLimitStore::new(1000),
        }
    }


    #[tokio::test]
    async fn result_branch_selection_and_status() {
        let env = env_with_vars(vec![]);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "echo".to_string(), config: "{}".to_string(), config_type: crate::ast::ConfigType::Quoted, condition: None, parsed_join_targets: None },
            PipelineStep::Result { branches: vec![
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Ok, status_code: 201, pipeline: Pipeline { steps: vec![] } },
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Default, status_code: 200, pipeline: Pipeline { steps: vec![] } },
            ]}
        ]};
        let (out, _ct, status, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.is_object());
        assert_eq!(status, Some(201));
    }

    #[tokio::test]
    async fn result_branch_custom_error_type_selected() {
        let env = env_with_vars(vec![]);
        let input = serde_json::json!({ "errors": [ { "type": "validationError", "message": "bad" } ] });
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Result { branches: vec![
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Custom("validationError".to_string()), status_code: 422, pipeline: Pipeline { steps: vec![] } },
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Default, status_code: 500, pipeline: Pipeline { steps: vec![] } },
            ]}
        ]};
        let (_out, _ct, status, _ctx) = execute_pipeline(&env, &pipeline, input, RequestContext::new()).await.unwrap();
        assert_eq!(status, Some(422));
    }

    #[tokio::test]
    async fn handlebars_sets_html_content_type() {
        let env = env_with_vars(vec![]);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "handlebars".to_string(), config: "Hello".to_string(), config_type: crate::ast::ConfigType::Quoted, condition: None, parsed_join_targets: None }
        ]};
        let (_out, ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert_eq!(ct, "text/html");
    }

    #[tokio::test]
    async fn variable_auto_naming_adds_and_removes_result_name() {
        let vars = vec![Variable { var_type: "echo".to_string(), name: "myVar".to_string(), value: "{}".to_string() }];
        let env = env_with_vars(vars);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "echo".to_string(), config: "myVar".to_string(), config_type: crate::ast::ConfigType::Identifier, condition: None, parsed_join_targets: None }
        ]};
        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.get("resultName").is_none());
        // Ensure output is an object and not empty
        assert!(out.is_object());
    }

    #[tokio::test]
    async fn env_tag_executes_step_when_environment_matches() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("production".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "test".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", false, vec!["production"]),
                parsed_join_targets: None,
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.is_object());
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("test"));
    }

    #[tokio::test]
    async fn env_tag_skips_step_when_environment_doesnt_match() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("development".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"message": "production only"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", false, vec!["production"]),
                parsed_join_targets: None,
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({"initial": "data"}), RequestContext::new()).await.unwrap();
        // Should return initial input unchanged since step was skipped
        assert_eq!(out.get("initial").and_then(|v| v.as_str()), Some("data"));
        assert!(out.get("message").is_none());
    }

    #[tokio::test]
    async fn negated_env_tag_skips_when_environment_matches() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("production".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"debug": true}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", true, vec!["production"]),
                parsed_join_targets: None,
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({"initial": "data"}), RequestContext::new()).await.unwrap();
        // Should skip the debug step in production
        assert!(out.get("debug").is_none());
        assert_eq!(out.get("initial").and_then(|v| v.as_str()), Some("data"));
    }

    #[tokio::test]
    async fn negated_env_tag_executes_when_environment_doesnt_match() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("development".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "dev".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", true, vec!["production"]),
                parsed_join_targets: None,
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        // Should execute in development (not production)
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("dev"));
    }

    #[tokio::test]
    async fn multiple_env_tags_all_must_match() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("production".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "executed".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: and_tags(
                    TagExpr::Tag(Tag { name: "env".to_string(), negated: false, args: vec!["production".to_string()] }),
                    TagExpr::Tag(Tag { name: "env".to_string(), negated: true, args: vec!["staging".to_string()] })
                ),
                parsed_join_targets: None,
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        // Should execute: env is production (✓) and env is not staging (✓)
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("executed"));
    }

    #[tokio::test]
    async fn no_environment_set_executes_non_negated_tags() {
        let env = env_with_vars(vec![]);
        // env.environment is None

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "noenv".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", false, vec!["production"]),
                parsed_join_targets: None,
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        // When no environment is set, non-negated tags execute
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("noenv"));
    }

    #[tokio::test]
    async fn flag_tag_skips_when_flag_disabled() {
        let env_no_flags = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "tagged".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("flag", false, vec!["beta"]),
                parsed_join_targets: None,
            }
        ]};

        // No flags in env - should fail closed (skip the step)
        let (out, _ct, _st, _ctx) = execute_pipeline(&env_no_flags, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.get("echo").is_none());

        // With flag enabled in RequestContext - should execute
        let mut ctx_with_flag = RequestContext::new();
        ctx_with_flag.feature_flags.insert("beta".to_string(), true);
        let (out2, _ct2, _st2, _ctx2) = execute_pipeline(&env_no_flags, &pipeline, serde_json::json!({}), ctx_with_flag).await.unwrap();
        assert_eq!(out2.get("echo").and_then(|v| v.as_str()), Some("tagged"));
    }

    #[tokio::test]
    async fn non_flag_tags_do_not_prevent_execution() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "tagged".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("needs", false, vec!["flags"]),
                parsed_join_targets: None,
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        // @needs tags are for static analysis only, don't prevent execution
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("tagged"));
    }

    #[tokio::test]
    async fn async_tag_spawns_task_without_blocking() {
        let env = env_with_vars(vec![]);

        // Pipeline with async step - should continue immediately
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"async": "data"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["task1"]),
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"sync": "data"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: None,
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        // The async step should not modify the state
        assert!(out.get("async").is_none());
        // The sync step should execute and modify state
        assert_eq!(out.get("sync").and_then(|v| v.as_str()), Some("data"));
    }

    #[tokio::test]
    async fn join_waits_for_async_tasks() {
        let env = env_with_vars(vec![]);

        // Pipeline with async step followed by join
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"result": "from-async"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["task1"]),
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: "task1".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: Some(vec!["task1".to_string()]),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        // Check that async results are available under .async.task1
        assert!(out.get("async").is_some());
        let async_obj = out.get("async").unwrap().as_object().unwrap();
        assert!(async_obj.contains_key("task1"));
        let task1_result = async_obj.get("task1").unwrap();
        assert_eq!(task1_result.get("result").and_then(|v| v.as_str()), Some("from-async"));
    }

    #[tokio::test]
    async fn join_multiple_async_tasks() {
        let env = env_with_vars(vec![]);

        // Pipeline with multiple async steps
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"data": "task1"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["task1"]),
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"data": "task2"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["task2"]),
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: "task1,task2".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: Some(vec!["task1".to_string(), "task2".to_string()]),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        // Check both tasks completed
        let async_obj = out.get("async").unwrap().as_object().unwrap();
        assert_eq!(async_obj.len(), 2);
        assert_eq!(async_obj.get("task1").unwrap().get("data").and_then(|v| v.as_str()), Some("task1"));
        assert_eq!(async_obj.get("task2").unwrap().get("data").and_then(|v| v.as_str()), Some("task2"));
    }

    #[tokio::test]
    async fn join_with_json_array_config() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"data": "a"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["a"]),
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: r#"["a"]"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: Some(vec!["a".to_string()]),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        let async_obj = out.get("async").unwrap().as_object().unwrap();
        assert_eq!(async_obj.get("a").unwrap().get("data").and_then(|v| v.as_str()), Some("a"));
    }

    #[tokio::test]
    async fn async_step_uses_state_snapshot() {
        let env = env_with_vars(vec![]);

        // Pipeline that modifies state, spawns async with that state, then modifies again
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"counter": 1}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: None,
            },
            // Async task should see counter: 1
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"asyncSaw": 0}"#.to_string(), // Will be replaced with actual counter value
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["snapshot"]),
                parsed_join_targets: None,
            },
            // Modify state after async spawn
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"counter": 2}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: "snapshot".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: Some(vec!["snapshot".to_string()]),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        // Main state should have counter: 2
        assert_eq!(out.get("counter").and_then(|v| v.as_u64()), Some(2));

        // Async task should have seen the snapshot with counter: 1
        let async_obj = out.get("async").unwrap().as_object().unwrap();
        let snapshot_result = async_obj.get("snapshot").unwrap();
        // The async echo will copy the input which had counter: 1
        assert!(snapshot_result.get("counter").is_some());
    }

    #[tokio::test]
    async fn when_tag_skips_when_condition_not_set() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "admin_only".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", false, vec!["is_admin"]),
                parsed_join_targets: None,
            }
        ]};

        // No conditions set - should fail closed (skip the step)
        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.get("echo").is_none());
    }

    #[tokio::test]
    async fn when_tag_executes_when_condition_set() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "admin_only".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", false, vec!["is_admin"]),
                parsed_join_targets: None,
            }
        ]};

        // With condition enabled in RequestContext - should execute
        let mut ctx_with_condition = RequestContext::new();
        ctx_with_condition.conditions.insert("is_admin".to_string(), true);
        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), ctx_with_condition).await.unwrap();
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("admin_only"));
    }

    #[tokio::test]
    async fn negated_when_tag_executes_when_condition_not_set() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "non_admin".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", true, vec!["is_admin"]),
                parsed_join_targets: None,
            }
        ]};

        // No conditions set - negated @!when(is_admin) should execute
        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("non_admin"));
    }

    #[tokio::test]
    async fn when_tag_multiple_conditions_all_must_be_true() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "admin_and_premium".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", false, vec!["is_admin", "is_premium"]),
                parsed_join_targets: None,
            }
        ]};

        // Only one condition set - should skip
        let mut ctx_partial = RequestContext::new();
        ctx_partial.conditions.insert("is_admin".to_string(), true);
        let (out1, _ct1, _st1, _ctx1) = execute_pipeline(&env, &pipeline, serde_json::json!({}), ctx_partial).await.unwrap();
        assert!(out1.get("echo").is_none());

        // Both conditions set - should execute
        let mut ctx_both = RequestContext::new();
        ctx_both.conditions.insert("is_admin".to_string(), true);
        ctx_both.conditions.insert("is_premium".to_string(), true);
        let (out2, _ct2, _st2, _ctx2) = execute_pipeline(&env, &pipeline, serde_json::json!({}), ctx_both).await.unwrap();
        assert_eq!(out2.get("echo").and_then(|v| v.as_str()), Some("admin_and_premium"));
    }

    #[tokio::test]
    async fn when_tag_isolation_from_flag_tag() {
        let env = env_with_vars(vec![]);

        // @flag(beta) step
        let flag_pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "flag_beta".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("flag", false, vec!["beta"]),
                parsed_join_targets: None,
            }
        ]};

        // @when(beta) step
        let when_pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: "when_beta".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", false, vec!["beta"]),
                parsed_join_targets: None,
            }
        ]};

        // Set beta in conditions only (not in flags)
        let mut ctx1 = RequestContext::new();
        ctx1.conditions.insert("beta".to_string(), true);

        // @flag(beta) should skip (beta is not in feature_flags)
        let (out1, _ct1, _st1, _ctx1) = execute_pipeline(&env, &flag_pipeline, serde_json::json!({}), ctx1).await.unwrap();
        assert!(out1.get("echo").is_none(), "flag tag should not match condition");

        // @when(beta) should execute (beta is in conditions)
        let mut ctx2 = RequestContext::new();
        ctx2.conditions.insert("beta".to_string(), true);
        let (out2, _ct2, _st2, _ctx2) = execute_pipeline(&env, &when_pipeline, serde_json::json!({}), ctx2).await.unwrap();
        assert_eq!(out2.get("echo").and_then(|v| v.as_str()), Some("when_beta"));
    }

    #[tokio::test]
    async fn request_context_to_value_includes_conditions() {
        let mut ctx = RequestContext::new();
        ctx.conditions.insert("is_admin".to_string(), true);
        ctx.conditions.insert("is_mobile".to_string(), false);

        let env = env_with_vars(vec![]);
        let context_value = ctx.to_value(&env);

        // Check that conditions are exposed
        let conditions = context_value.get("conditions").expect("conditions should be present");
        assert_eq!(conditions.get("is_admin").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(conditions.get("is_mobile").and_then(|v| v.as_bool()), Some(false));
    }
}

