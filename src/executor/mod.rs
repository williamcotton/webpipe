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
};

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
#[derive(Clone, Debug)]
pub struct CachePolicy {
    pub enabled: bool,
    pub ttl: u64,
    pub key_template: Option<String>,
}

/// Log configuration for logging middleware
#[derive(Clone, Debug)]
pub struct LogConfig {
    pub level: String,
    pub include_body: bool,
    pub include_headers: bool,
}

/// Rate limit status for headers/logging
#[derive(Clone, Debug)]
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

/// Per-request mutable context (The Backpack)
pub struct RequestContext {
    /// Feature flags for this request (extracted from headers or auth, NOT from user JSON)
    pub feature_flags: HashMap<String, bool>,

    /// Async task registry for @async steps
    pub async_registry: AsyncTaskRegistry,

    /// Deferred actions to run at pipeline completion
    pub deferred: Vec<Box<dyn FnOnce(&Value, &str, &ExecutionEnv) + Send>>,

    /// Typed metadata for middleware communication
    pub metadata: RequestMetadata,
}

impl RequestContext {
    pub fn new() -> Self {
        Self {
            feature_flags: HashMap::new(),
            async_registry: AsyncTaskRegistry::new(),
            deferred: Vec::new(),
            metadata: RequestMetadata::default(),
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

fn should_execute_step(tags: &[crate::ast::Tag], env: &ExecutionEnv, ctx: &RequestContext, input: &Value) -> bool {
    tags.iter().all(|tag| check_tag(tag, env, ctx, input))
}

fn check_tag(tag: &crate::ast::Tag, env: &ExecutionEnv, ctx: &RequestContext, _input: &Value) -> bool {
    match tag.name.as_str() {
        "env" => check_env_tag(tag, env),
        "async" => true, // async doesn't affect execution, just how it runs
        "flag" => check_flag_tag(tag, ctx),
        "needs" => true, // @needs is only for static analysis, doesn't affect runtime
        _ => true,       // unknown tags don't prevent execution
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

fn get_async_tag(tags: &[crate::ast::Tag]) -> Option<String> {
    tags.iter()
        .find(|tag| tag.name == "async" && !tag.negated && tag.args.len() == 1)
        .map(|tag| tag.args[0].clone())
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

#[async_recursion]
async fn handle_result_step<'a>(
    env: &'a ExecutionEnv,
    ctx: &'a mut RequestContext,
    branches: &'a [crate::ast::ResultBranch],
    input: Value,
    mut content_type: String,
) -> Result<(Value, String, Option<u16>), WebPipeError> {
    let error_type = extract_error_type(&input);
    let selected = select_branch(branches, &error_type);
    if let Some(branch) = selected {
        let inherited_cookies = input.get("setCookies").cloned();
        let (mut result, branch_content_type, _status) = execute_pipeline_internal(env, &branch.pipeline, input, ctx).await?;
        if inherited_cookies.is_some() {
            if let Some(obj) = result.as_object_mut() {
                if !obj.contains_key("setCookies") {
                    if let Some(c) = inherited_cookies { obj.insert("setCookies".to_string(), c); }
                }
            }
        }
        if branch_content_type != "application/json" { content_type = branch_content_type; }
        Ok((result, content_type, Some(branch.status_code)))
    } else {
        Ok((input, content_type, None))
    }
}

/// Determine the execution mode for a step
fn detect_execution_mode(name: &str, tags: &[crate::ast::Tag]) -> ExecutionMode {
    // Check for async tag first
    if let Some(async_name) = get_async_tag(tags) {
        return ExecutionMode::Async(async_name);
    }

    // Check for special middleware types
    match name {
        "join" => ExecutionMode::Join,
        "pipeline" => ExecutionMode::Recursive,
        _ => ExecutionMode::Standard,
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
            PipelineStep::Regular { tags, .. } => {
                // If this remaining step would execute, current is not the last
                if should_execute_step(tags, env, ctx, state) {
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

/// Internal pipeline execution that accepts a mutable RequestContext
#[async_recursion]
async fn execute_pipeline_internal<'a>(
    env: &'a ExecutionEnv,
    pipeline: &'a Pipeline,
    input: Value,
    ctx: &'a mut RequestContext,
) -> Result<(Value, String, Option<u16>), WebPipeError> {
    let mut content_type = "application/json".to_string();
    let mut status_code_out: Option<u16> = None;

    // Create PipelineContext with the initial state
    let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());

    for (idx, step) in pipeline.steps.iter().enumerate() {
        match step {
            PipelineStep::Regular { name, config, config_type: _, tags, parsed_join_targets } => {
                // 1. Check Guards (tags/flags)
                if !should_execute_step(tags, env, ctx, &pipeline_ctx.state) {
                    continue;
                }

                // Check if this is effectively the last step (no remaining steps will execute)
                let is_last_step = is_effectively_last_step(idx, &pipeline.steps, env, ctx, &pipeline_ctx.state);

                // 2. Determine Execution Mode
                let mode = detect_execution_mode(name, tags);

                // Handle async execution separately (no state merge needed)
                if let ExecutionMode::Async(async_name) = mode {
                    spawn_async_step(env, ctx, name, config, pipeline_ctx.state.clone(), async_name);
                    continue;
                }

                // Resolve configuration and handle auto-naming
                let (effective_config, effective_input, auto_named) =
                    resolve_config_and_autoname(&env.variables, name, config, &pipeline_ctx.state);

                // If auto-named, update the pipeline context state with resultName
                if auto_named {
                    pipeline_ctx.state = effective_input.clone();
                }

                // 3. Execute Strategy - delegate to specialized handlers
                let step_result = match mode {
                    ExecutionMode::Join => {
                        // Use pre-parsed targets for hot path optimization
                        handle_join(ctx, &effective_config, parsed_join_targets.as_ref(), effective_input.clone()).await?
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

                // 4. Update Pipeline State using centralized merge logic
                // Terminal transformers (jq, handlebars) and final steps replace entirely
                // Other middleware steps merge (Backpack semantics)
                let should_merge = !is_last_step && name != "handlebars" && name != "jq";
                if should_merge {
                    pipeline_ctx.merge_state(step_result.value);
                } else {
                    // For non-final jq steps, preserve runtime context keys
                    // These are needed for async/join, accumulated results, and request context
                    // Final steps should produce clean output without runtime metadata
                    const RUNTIME_KEYS: &[&str] = &[
                        "async", "data", "originalRequest",
                        "query", "params", "body", "headers", "cookies",
                        "method", "path", "ip", "content_type"
                    ];
                    
                    let backups: Vec<(String, Value)> = if name == "jq" && !is_last_step {
                        RUNTIME_KEYS.iter()
                            .filter_map(|&key| {
                                pipeline_ctx.state.get(key).map(|v| (key.to_string(), v.clone()))
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    
                    pipeline_ctx.replace_state(step_result.value);
                    
                    // Restore runtime keys if they existed and aren't in the new state
                    if !backups.is_empty() {
                        if let Some(obj) = pipeline_ctx.state.as_object_mut() {
                            for (key, val) in backups {
                                if !obj.contains_key(&key) {
                                    obj.insert(key, val);
                                }
                            }
                        }
                    }
                }

                // 5. Update content type if changed
                if step_result.content_type != "application/json" || is_last_step {
                    if name == "pipeline" || is_last_step || name == "handlebars" {
                        content_type = step_result.content_type;
                    }
                }
                if let Some(status) = step_result.status_code {
                    status_code_out = Some(status);
                }

                // 6. Handle Signals (Cache hits, etc)
                if let Some((cached_val, cached_content_type)) = check_cache_control_signal(&pipeline_ctx.state) {
                    let final_content_type = cached_content_type.unwrap_or(content_type);
                    return Ok((cached_val, final_content_type, status_code_out));
                }

                // 7. Clean up auto-naming
                if auto_named {
                    if let Some(obj) = pipeline_ctx.state.as_object_mut() {
                        obj.remove("resultName");
                    }
                }
            }
            PipelineStep::Result { branches } => {
                return handle_result_step(env, ctx, branches, pipeline_ctx.state, content_type).await;
            }
            PipelineStep::If { condition, then_branch, else_branch } => {
                // 1. Run Condition on Cloned State
                let (cond_result, _, _) = execute_pipeline_internal(
                    env,
                    condition,
                    pipeline_ctx.state.clone(), // <--- CLONE
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
                    if let Some(s) = status { status_code_out = Some(s); }
                    if ct != "application/json" { content_type = ct; }

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
                    if let Some(s) = status { status_code_out = Some(s); }
                    if ct != "application/json" { content_type = ct; }
                }
                // If false and no else: state remains unchanged (pass-through)
            }
        }
    }

    Ok((pipeline_ctx.state, content_type, status_code_out))
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
    use crate::ast::{PipelineStep, Pipeline};
    use std::sync::Arc;

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
        ExecutionEnv {
            variables: Arc::new(vars),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
            environment: None,
            
            cache: CacheStore::new(8, 60),
            rate_limit: crate::runtime::context::RateLimitStore::new(1000),
        }
    }


    #[tokio::test]
    async fn result_branch_selection_and_status() {
        let env = env_with_vars(vec![]);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "echo".to_string(), config: "{}".to_string(), config_type: crate::ast::ConfigType::Quoted, tags: vec![], parsed_join_targets: None },
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
            PipelineStep::Regular { name: "handlebars".to_string(), config: "Hello".to_string(), config_type: crate::ast::ConfigType::Quoted, tags: vec![], parsed_join_targets: None }
        ]};
        let (_out, ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert_eq!(ct, "text/html");
    }

    #[tokio::test]
    async fn variable_auto_naming_adds_and_removes_result_name() {
        let vars = vec![Variable { var_type: "echo".to_string(), name: "myVar".to_string(), value: "{}".to_string() }];
        let env = env_with_vars(vars);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "echo".to_string(), config: "myVar".to_string(), config_type: crate::ast::ConfigType::Identifier, tags: vec![], parsed_join_targets: None }
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: false, args: vec!["production".to_string()] }],
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: false, args: vec!["production".to_string()] }],
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: true, args: vec!["production".to_string()] }],
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: true, args: vec!["production".to_string()] }],
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
                tags: vec![
                    crate::ast::Tag { name: "env".to_string(), negated: false, args: vec!["production".to_string()] },
                    crate::ast::Tag { name: "env".to_string(), negated: true, args: vec!["staging".to_string()] }
                ],
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: false, args: vec!["production".to_string()] }],
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
                tags: vec![
                    crate::ast::Tag { name: "flag".to_string(), negated: false, args: vec!["beta".to_string()] }
                ],
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
                tags: vec![
                    crate::ast::Tag { name: "needs".to_string(), negated: false, args: vec!["flags".to_string()] }
                ],
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
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["task1".to_string()] }],
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"sync": "data"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![],
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
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["task1".to_string()] }],
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: "task1".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![],
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
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["task1".to_string()] }],
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"data": "task2"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["task2".to_string()] }],
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: "task1,task2".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![],
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
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["a".to_string()] }],
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: r#"["a"]"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![],
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
                tags: vec![],
                parsed_join_targets: None,
            },
            // Async task should see counter: 1
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"asyncSaw": 0}"#.to_string(), // Will be replaced with actual counter value
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["snapshot".to_string()] }],
                parsed_join_targets: None,
            },
            // Modify state after async spawn
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"counter": 2}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![],
                parsed_join_targets: None,
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: "snapshot".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![],
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
}

