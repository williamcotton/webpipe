//! Step execution logic for pipeline steps.
//!
//! This module extracts the step execution logic from the main executor module
//! to reduce complexity and improve testability. It provides:
//!
//! - `StepContext`: A context object that bundles execution state
//! - `RegularStepExecutor`: Handles execution of regular pipeline steps
//! - Control flow handlers: `handle_if_step`, `handle_dispatch_step`, etc.

use serde_json::Value;

use crate::{
    ast::{Pipeline, PipelineStep, SourceLocation, TagExpr},
    error::WebPipeError,
    runtime::json_path,
};

use super::{
    context::RequestContext,
    env::ExecutionEnv,
    resolver::{determine_target_name, extract_error_type, resolve_config_and_autoname, select_branch},
    tags::{check_tag_expr, get_async_from_tag_expr, should_execute_step},
    types::{ExecutionMode, PipelineOutput, StepOutcome, StepResult},
};

#[cfg(feature = "debugger")]
use super::context::StepMode;

// ==================================================================================
// StepContext - Bundles execution state to reduce argument bloat
// ==================================================================================

/// Bundles the mutable state required for step execution.
///
/// This reduces the number of arguments passed to step execution functions
/// from 10+ to a single context object, making the code more readable and
/// easier to maintain.
pub struct StepContext<'a> {
    /// Global execution environment (immutable)
    pub env: &'a ExecutionEnv,
    /// Per-request mutable context (feature flags, async registry, profiler)
    pub req_ctx: &'a mut RequestContext,
    /// Pipeline-level mutable state (JSON state being transformed)
    pub pipe_ctx: &'a mut crate::runtime::PipelineContext,
    /// Output tracking (content_type, status_code)
    pub output: &'a mut PipelineOutput,
    /// Current step index in the pipeline
    pub step_index: usize,
    /// Reference to all steps in the pipeline (for lookahead checks)
    pub all_steps: &'a [PipelineStep],
}

impl<'a> StepContext<'a> {
    /// Check if this is effectively the last step that will execute.
    /// Accounts for remaining steps being skipped due to feature flags, etc.
    pub fn is_last_step(&self) -> bool {
        is_effectively_last_step(
            self.step_index,
            self.all_steps,
            self.env,
            self.req_ctx,
            &self.pipe_ctx.state,
        )
    }

    /// Take ownership of the current state, leaving Null in its place.
    /// Used by branch handlers (If, Dispatch, Result) to pass state to sub-pipelines.
    pub fn take_state(&mut self) -> Value {
        std::mem::take(&mut self.pipe_ctx.state)
    }
}

// ==================================================================================
// RegularStepExecutor - Handles regular pipeline steps
// ==================================================================================

/// Executor for regular pipeline steps (middleware calls).
///
/// This struct extracts the 8 responsibilities of regular step execution
/// into focused methods:
/// 1. Guard checks (should_execute_step)
/// 2. Execution mode detection
/// 3. Config resolution
/// 4. Core execution (Standard/Join/Recursive/Async)
/// 5. State update
/// 6. Metadata update
/// 7. Cache control check
/// 8. Cleanup
pub struct RegularStepExecutor<'a> {
    name: &'a str,
    args: &'a [String],
    config: &'a str,
    condition: &'a Option<TagExpr>,
    parsed_join_targets: Option<&'a Vec<String>>,
    location: &'a SourceLocation,
}

impl<'a> RegularStepExecutor<'a> {
    /// Create a new executor for a regular step.
    pub fn new(
        name: &'a str,
        args: &'a [String],
        config: &'a str,
        condition: &'a Option<TagExpr>,
        parsed_join_targets: Option<&'a Vec<String>>,
        location: &'a SourceLocation,
    ) -> Self {
        Self {
            name,
            args,
            config,
            condition,
            parsed_join_targets,
            location,
        }
    }

    /// Execute the regular step.
    pub async fn execute(&self, ctx: &mut StepContext<'_>) -> Result<StepOutcome, WebPipeError> {
        // 1. Guard Check
        if !should_execute_step(self.condition, ctx.env, ctx.req_ctx, &ctx.pipe_ctx.state) {
            return Ok(StepOutcome::Continue);
        }

        let is_last_step = ctx.is_last_step();

        // 2. Determine Execution Mode
        let mode = detect_execution_mode(self.name, self.condition);

        // 3. Async Fast Path (returns early, no state merge)
        if let ExecutionMode::Async(async_name) = mode {
            spawn_async_step(
                ctx.env,
                ctx.req_ctx,
                self.name,
                self.args,
                self.config,
                self.location,
                ctx.pipe_ctx.state.clone(),
                async_name,
            );
            return Ok(StepOutcome::Continue);
        }

        // 4. Config Resolution
        let (effective_config, effective_input, auto_named) = resolve_config_and_autoname(
            ctx.env,
            self.location,
            self.name,
            self.config,
            &ctx.pipe_ctx.state,
        );

        if auto_named {
            ctx.pipe_ctx.state = effective_input.clone();
        }

        let (target_name, should_cleanup) =
            determine_target_name(self.condition, &ctx.pipe_ctx.state, auto_named);

        // 5. Execute Core Logic
        let step_result = self
            .run_core_logic(mode, ctx, &effective_config, effective_input, target_name.as_deref())
            .await?;

        // 6. Apply State Changes
        self.apply_result_to_state(ctx, step_result.value, target_name.as_deref(), is_last_step);

        // 7. Update Metadata
        self.update_metadata(ctx, step_result.content_type, step_result.status_code, is_last_step);

        // 8. Cache Control Check
        if let Some(outcome) = self.check_cache_stop(ctx) {
            return Ok(outcome);
        }

        // 9. Cleanup
        if should_cleanup {
            if let Some(obj) = ctx.pipe_ctx.state.as_object_mut() {
                obj.remove("resultName");
            }
        }

        Ok(StepOutcome::Continue)
    }

    /// Execute the core step logic based on execution mode.
    async fn run_core_logic(
        &self,
        mode: ExecutionMode,
        ctx: &mut StepContext<'_>,
        config: &str,
        input: Value,
        target_name: Option<&str>,
    ) -> Result<StepResult, WebPipeError> {
        match mode {
            ExecutionMode::Join => {
                handle_join(ctx.req_ctx, config, self.parsed_join_targets, input).await
            }
            ExecutionMode::Recursive => {
                handle_recursive_pipeline(ctx.env, ctx.req_ctx, self.args, config, self.location, input)
                    .await
            }
            ExecutionMode::Standard => {
                let mut temp_ctx = crate::runtime::PipelineContext::new(input);
                handle_standard_execution(
                    ctx.env,
                    ctx.req_ctx,
                    self.name,
                    self.args,
                    config,
                    &mut temp_ctx,
                    target_name,
                )
                .await
            }
            ExecutionMode::Async(_) => unreachable!("Async handled above"),
        }
    }

    /// Apply the step result to pipeline state.
    fn apply_result_to_state(
        &self,
        ctx: &mut StepContext,
        new_value: Value,
        target_name: Option<&str>,
        is_last_step: bool,
    ) {
        // Data-fetching middleware support result wrapping under .data.<target>
        let should_wrap = target_name.is_some() && matches!(self.name, "pg" | "fetch" | "graphql");

        if should_wrap {
            let target = target_name.unwrap();
            self.apply_data_wrap(ctx, new_value, target);
        } else {
            let behavior = ctx
                .env
                .registry
                .get_behavior(self.name)
                .unwrap_or(crate::middleware::StateBehavior::Merge);
            update_pipeline_state(ctx.pipe_ctx, new_value, behavior, is_last_step);
        }
    }

    /// Wrap result under .data.<target> for data-fetching middleware.
    fn apply_data_wrap(&self, ctx: &mut StepContext, value: Value, target: &str) {
        if let Some(obj) = ctx.pipe_ctx.state.as_object_mut() {
            let data_entry = obj
                .entry("data")
                .or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !data_entry.is_object() {
                *data_entry = Value::Object(serde_json::Map::new());
            }
            if let Some(data_obj) = data_entry.as_object_mut() {
                data_obj.insert(target.to_string(), value);
            }
        } else {
            ctx.pipe_ctx.state = serde_json::json!({
                "data": {
                    target: value
                }
            });
        }
    }

    /// Update output metadata (content_type, status_code).
    fn update_metadata(
        &self,
        ctx: &mut StepContext,
        content_type: String,
        status: Option<u16>,
        is_last_step: bool,
    ) {
        if content_type != "application/json" || is_last_step {
            if self.name == "pipeline" || is_last_step || self.name == "handlebars" || self.name == "gg"
            {
                ctx.output.content_type = content_type;
            }
        }

        if let Some(s) = status {
            ctx.output.status_code = Some(s);
        }
    }

    /// Check for cache stop signal.
    fn check_cache_stop(&self, ctx: &StepContext) -> Option<StepOutcome> {
        if let Some((cached_val, cached_ct)) = check_cache_control_signal(&ctx.pipe_ctx.state) {
            let final_ct = cached_ct.unwrap_or_else(|| ctx.output.content_type.clone());
            return Some(StepOutcome::Return(PipelineOutput {
                state: cached_val,
                content_type: final_ct,
                status_code: ctx.output.status_code,
            }));
        }
        None
    }
}

// ==================================================================================
// Control Flow Step Handlers
// ==================================================================================

/// Handle an If step - evaluate condition and execute appropriate branch.
pub async fn handle_if_step(
    condition: &Pipeline,
    then_branch: &Pipeline,
    else_branch: &Option<Pipeline>,
    ctx: &mut StepContext<'_>,
) -> Result<StepOutcome, WebPipeError> {
    // Run condition on cloned state
    let (cond_result, _, _) =
        super::execute_pipeline_internal(ctx.env, condition, ctx.pipe_ctx.state.clone(), ctx.req_ctx)
            .await?;

    let is_truthy = match cond_result {
        Value::Bool(b) => b,
        Value::Null => false,
        _ => true,
    };

    if is_truthy {
        execute_branch(then_branch, ctx).await?;
    } else if let Some(else_pipe) = else_branch {
        execute_branch(else_pipe, ctx).await?;
    }

    Ok(StepOutcome::Continue)
}

/// Handle a Dispatch step - match conditions and execute first matching branch.
pub async fn handle_dispatch_step(
    branches: &[crate::ast::DispatchBranch],
    default: &Option<Pipeline>,
    ctx: &mut StepContext<'_>,
) -> Result<StepOutcome, WebPipeError> {
    let mut matched_pipeline = default.as_ref();

    for branch in branches {
        if check_tag_expr(&branch.condition, ctx.env, ctx.req_ctx, &ctx.pipe_ctx.state) {
            matched_pipeline = Some(&branch.pipeline);
            break;
        }
    }

    if let Some(pipeline) = matched_pipeline {
        execute_branch(pipeline, ctx).await?;
    }

    Ok(StepOutcome::Continue)
}

/// Handle a Foreach step - iterate over array and execute pipeline for each item.
pub async fn handle_foreach_step(
    selector: &str,
    pipeline: &Pipeline,
    ctx: &mut StepContext<'_>,
) -> Result<StepOutcome, WebPipeError> {
    // Surgical extraction: take the array out of the state
    let items_opt = json_path::get_value_mut(&mut ctx.pipe_ctx.state, selector).map(|val| val.take());

    if let Some(Value::Array(items)) = items_opt {
        let mut results = Vec::with_capacity(items.len());

        for item in items {
            let (final_state, _, _) =
                super::execute_pipeline_internal(ctx.env, pipeline, item, ctx.req_ctx).await?;
            results.push(final_state);
        }

        // Implant results back at the selector path
        if let Some(slot) = json_path::get_value_mut(&mut ctx.pipe_ctx.state, selector) {
            *slot = Value::Array(results);
        }
    } else {
        return Err(WebPipeError::MiddlewareExecutionError(format!(
            "foreach: path '{}' is not an array or does not exist",
            selector
        )));
    }

    Ok(StepOutcome::Continue)
}

/// Handle a Result step - select branch based on error type and return.
pub async fn handle_result_step(
    branches: &[crate::ast::ResultBranch],
    ctx: &mut StepContext<'_>,
) -> Result<StepOutcome, WebPipeError> {
    let error_type = extract_error_type(&ctx.pipe_ctx.state);
    let selected = select_branch(branches, &error_type);

    if let Some(branch) = selected {
        let inherited_cookies = ctx.pipe_ctx.state.get("setCookies").cloned();
        let input_state = ctx.take_state();

        let (mut result, branch_ct, _status) =
            super::execute_pipeline_internal(ctx.env, &branch.pipeline, input_state, ctx.req_ctx)
                .await?;

        // Restore cookies if not present in result
        if inherited_cookies.is_some() {
            if let Some(obj) = result.as_object_mut() {
                if !obj.contains_key("setCookies") {
                    if let Some(c) = inherited_cookies {
                        obj.insert("setCookies".to_string(), c);
                    }
                }
            }
        }

        ctx.pipe_ctx.state = result.clone();

        let final_ct = if branch_ct != "application/json" {
            branch_ct
        } else {
            ctx.output.content_type.clone()
        };

        return Ok(StepOutcome::Return(PipelineOutput {
            state: result,
            content_type: final_ct,
            status_code: Some(branch.status_code),
        }));
    }

    Ok(StepOutcome::Return(PipelineOutput {
        state: ctx.pipe_ctx.state.clone(),
        content_type: ctx.output.content_type.clone(),
        status_code: ctx.output.status_code,
    }))
}

/// Execute a branch pipeline and update context state.
/// Common helper for If and Dispatch steps.
async fn execute_branch(pipeline: &Pipeline, ctx: &mut StepContext<'_>) -> Result<(), WebPipeError> {
    let input_state = ctx.take_state();
    let (res, ct, status) =
        super::execute_pipeline_internal(ctx.env, pipeline, input_state, ctx.req_ctx).await?;

    ctx.pipe_ctx.state = res;
    if let Some(s) = status {
        ctx.output.status_code = Some(s);
    }
    if ct != "application/json" {
        ctx.output.content_type = ct;
    }
    Ok(())
}

// ==================================================================================
// Utility Functions
// ==================================================================================

/// Determine the display name for a step (used in profiling and debugging).
pub fn determine_step_name(step: &PipelineStep) -> String {
    let base_name = match step {
        PipelineStep::Regular { name, config, .. } => {
            if name == "pipeline" {
                format!("pipeline:{}", config.trim())
            } else {
                name.clone()
            }
        }
        PipelineStep::If { .. } => "if".to_string(),
        PipelineStep::Result { .. } => "result".to_string(),
        PipelineStep::Dispatch { .. } => "dispatch".to_string(),
        PipelineStep::Foreach { .. } => "foreach".to_string(),
    };

    let location = step.location();
    if let Some(ref file_path) = location.file_path {
        let filename = std::path::Path::new(file_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(file_path);
        format!("{}@{}", base_name, filename)
    } else {
        base_name
    }
}

/// Handle debugger hooks before step execution.
#[cfg(feature = "debugger")]
pub async fn handle_debugger_before(
    ctx: &mut StepContext<'_>,
    step: &PipelineStep,
    step_name: &str,
) -> Result<(), WebPipeError> {
    if let Some(ref dbg) = ctx.env.debugger {
        let thread_id = ctx.req_ctx.debug_thread_id.unwrap_or(0);
        let location = step.location();
        let stack = ctx.req_ctx.profiler.stack.clone();
        let action = dbg
            .before_step(thread_id, step_name, location, &mut ctx.pipe_ctx.state, stack)
            .await?;

        match action {
            crate::debugger::StepAction::Continue => {
                ctx.req_ctx.debug_step_mode = None;
            }
            crate::debugger::StepAction::StepOver => {
                ctx.req_ctx.debug_step_mode = Some(StepMode::StepOver);
            }
            crate::debugger::StepAction::StepIn => {
                ctx.req_ctx.debug_step_mode = Some(StepMode::StepIn);
            }
            crate::debugger::StepAction::StepOut => {
                ctx.req_ctx.debug_step_mode = Some(StepMode::StepOut);
            }
        }
    }
    Ok(())
}

/// Handle debugger hooks after step execution.
#[cfg(feature = "debugger")]
pub async fn handle_debugger_after(ctx: &mut StepContext<'_>, step: &PipelineStep, step_name: &str) {
    if let Some(ref dbg) = ctx.env.debugger {
        let thread_id = ctx.req_ctx.debug_thread_id.unwrap_or(0);
        let location = step.location();
        let stack = ctx.req_ctx.profiler.stack.clone();
        dbg.after_step(thread_id, step_name, location, &ctx.pipe_ctx.state, stack)
            .await;
    }
}

// ==================================================================================
// Internal Helper Functions (re-exported from mod.rs or kept private)
// ==================================================================================

/// Check for cache control signal from cache middleware.
fn check_cache_control_signal(input: &Value) -> Option<(Value, Option<String>)> {
    input.get("_control").and_then(|c| {
        if c.get("stop").and_then(|b| b.as_bool()).unwrap_or(false) {
            let value = c.get("value").cloned()?;
            let content_type = c
                .get("content_type")
                .and_then(|ct| ct.as_str())
                .map(|s| s.to_string());
            Some((value, content_type))
        } else {
            None
        }
    })
}

/// Determine the execution mode for a step.
fn detect_execution_mode(name: &str, condition: &Option<TagExpr>) -> ExecutionMode {
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

/// Update pipeline state after a step execution.
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
                pipeline_ctx.replace_state(new_value);
            } else {
                let backups = pipeline_ctx.backup_system_keys();
                pipeline_ctx.replace_state(new_value);
                pipeline_ctx.restore_system_keys(backups);
            }
        }
        StateBehavior::Render => {
            pipeline_ctx.replace_state(new_value);
        }
        StateBehavior::ReadOnly => {
            // ReadOnly middleware don't modify state
        }
    }
}

/// Check if this is effectively the last step that will execute.
fn is_effectively_last_step(
    current_idx: usize,
    steps: &[PipelineStep],
    env: &ExecutionEnv,
    ctx: &RequestContext,
    state: &Value,
) -> bool {
    for remaining_step in steps.iter().skip(current_idx + 1) {
        match remaining_step {
            PipelineStep::Regular { condition, .. } => {
                if should_execute_step(condition, env, ctx, state) {
                    return false;
                }
            }
            PipelineStep::Result { .. }
            | PipelineStep::If { .. }
            | PipelineStep::Dispatch { .. }
            | PipelineStep::Foreach { .. } => {
                return false;
            }
        }
    }
    true
}

/// Handle join operation - wait for async tasks and merge results.
async fn handle_join(
    ctx: &mut RequestContext,
    config: &str,
    parsed_targets: Option<&Vec<String>>,
    input: Value,
) -> Result<StepResult, WebPipeError> {
    use super::tasks::parse_join_task_names;

    let task_names: Vec<String> = match parsed_targets {
        Some(targets) => targets.clone(),
        None => parse_join_task_names(config)?,
    };

    let mut async_results = serde_json::Map::new();
    for task_name in task_names {
        if let Some(handle) = ctx.async_registry.take(&task_name) {
            match handle.await {
                Ok(Ok((result, async_profiler))) => {
                    async_results.insert(task_name.clone(), result);

                    // Merge async task's profiler samples
                    for (async_stack, duration) in async_profiler.samples {
                        let prefixed_stack = if ctx.profiler.stack.is_empty() {
                            format!("async:{};{}", task_name, async_stack)
                        } else {
                            format!(
                                "{};async:{};{}",
                                ctx.profiler.stack.join(";"),
                                task_name,
                                async_stack
                            )
                        };
                        ctx.profiler.samples.push((prefixed_stack, duration));
                    }
                }
                Ok(Err(e)) => {
                    async_results.insert(
                        task_name,
                        serde_json::json!({
                            "error": e.to_string()
                        }),
                    );
                }
                Err(e) => {
                    async_results.insert(
                        task_name,
                        serde_json::json!({
                            "error": format!("Task panicked: {}", e)
                        }),
                    );
                }
            }
        }
    }

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

/// Spawn an async task without waiting for it.
fn spawn_async_step(
    env: &ExecutionEnv,
    ctx: &mut RequestContext,
    name: &str,
    args: &[String],
    config: &str,
    location: &SourceLocation,
    input: Value,
    async_name: String,
) {
    use super::resolver::parse_scoped_ref;

    let env_clone = env.clone();
    let name_clone = name.to_string();
    let args_clone = args.to_vec();
    let config_clone = config.to_string();
    let location_clone = location.clone();
    let input_snapshot = input;

    let handle = tokio::spawn(async move {
        let (effective_config, effective_input, _auto_named) = resolve_config_and_autoname(
            &env_clone,
            &location_clone,
            &name_clone,
            &config_clone,
            &input_snapshot,
        );

        let mut async_ctx = RequestContext::new();

        #[cfg(feature = "debugger")]
        if let Some(ref dbg) = env_clone.debugger {
            async_ctx.debug_thread_id = Some(dbg.allocate_thread_id());
        }

        if name_clone == "pipeline" {
            let pipeline_name = effective_config.trim();
            let (alias, name) = parse_scoped_ref(pipeline_name);

            let module_id_opt = if let Some(alias) = alias {
                if let Some(step_module_id) = location_clone.module_id {
                    if let Some(step_module) = env_clone.module_registry.get_module(step_module_id) {
                        step_module.import_map.get(&alias).copied()
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                location_clone.module_id
            };

            let key = (module_id_opt, name);
            if let Some(p) = env_clone.named_pipelines.get(&key) {
                let pipeline_input = if !args_clone.is_empty() {
                    let arg_value = crate::runtime::jq::evaluate(&args_clone[0], &effective_input)
                        .map_err(|e| {
                            WebPipeError::MiddlewareExecutionError(format!(
                                "pipeline argument evaluation failed: {}",
                                e
                            ))
                        })?;

                    if let (Some(input_obj), Some(arg_obj)) =
                        (effective_input.as_object(), arg_value.as_object())
                    {
                        let mut merged = input_obj.clone();
                        for (k, v) in arg_obj {
                            merged.insert(k.clone(), v.clone());
                        }
                        Value::Object(merged)
                    } else {
                        arg_value
                    }
                } else {
                    effective_input.clone()
                };

                let (val, _ct, _st, ctx) =
                    super::execute_pipeline(&env_clone, p, pipeline_input, async_ctx).await?;
                Ok((val, ctx.profiler))
            } else {
                Err(WebPipeError::PipelineNotFound(pipeline_name.to_string()))
            }
        } else {
            let mut pipeline_ctx = crate::runtime::PipelineContext::new(effective_input);
            env_clone
                .invoker
                .call(
                    &name_clone,
                    &args_clone,
                    &effective_config,
                    &mut pipeline_ctx,
                    &env_clone,
                    &mut async_ctx,
                    None,
                )
                .await?;
            Ok((pipeline_ctx.state, async_ctx.profiler))
        }
    });

    ctx.async_registry.register(async_name, handle);
}

/// Execute a named pipeline recursively.
fn handle_recursive_pipeline<'a>(
    env: &'a ExecutionEnv,
    ctx: &'a mut RequestContext,
    args: &'a [String],
    config: &'a str,
    location: &'a SourceLocation,
    input: Value,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<StepResult, WebPipeError>> + Send + 'a>>
{
    use super::resolver::parse_scoped_ref;

    Box::pin(async move {
        let pipeline_name = config.trim();
        let (alias, name) = parse_scoped_ref(pipeline_name);

        let module_id_opt = if let Some(alias) = alias {
            if let Some(step_module_id) = location.module_id {
                if let Some(step_module) = env.module_registry.get_module(step_module_id) {
                    step_module.import_map.get(&alias).copied()
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            location.module_id
        };

        let key = (module_id_opt, name);

        if let Some(pipeline) = env.named_pipelines.get(&key) {
            let pipeline_input = if !args.is_empty() {
                let arg_value = crate::runtime::jq::evaluate(&args[0], &input).map_err(|e| {
                    WebPipeError::MiddlewareExecutionError(format!(
                        "pipeline argument evaluation failed: {}",
                        e
                    ))
                })?;

                if let (Some(input_obj), Some(arg_obj)) = (input.as_object(), arg_value.as_object()) {
                    let mut merged = input_obj.clone();
                    for (k, v) in arg_obj {
                        merged.insert(k.clone(), v.clone());
                    }
                    Value::Object(merged)
                } else {
                    arg_value
                }
            } else {
                input
            };

            let (value, content_type, status_code) =
                super::execute_pipeline_internal(env, pipeline, pipeline_input, ctx).await?;
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

/// Execute standard middleware.
async fn handle_standard_execution(
    env: &ExecutionEnv,
    ctx: &mut RequestContext,
    name: &str,
    args: &[String],
    config: &str,
    pipeline_ctx: &mut crate::runtime::PipelineContext,
    target_name: Option<&str>,
) -> Result<StepResult, WebPipeError> {
    env.invoker
        .call(name, args, config, pipeline_ctx, env, ctx, target_name)
        .await?;

    let content_type = if name == "handlebars" {
        "text/html".to_string()
    } else if name == "gg" {
        if pipeline_ctx
            .state
            .as_str()
            .map_or(false, |s| s.trim().starts_with("<svg"))
        {
            "image/svg+xml".to_string()
        } else {
            "application/json".to_string()
        }
    } else {
        "application/json".to_string()
    };

    Ok(StepResult {
        value: pipeline_ctx.state.clone(),
        content_type,
        status_code: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_determine_step_name_regular() {
        let step = PipelineStep::Regular {
            name: "jq".to_string(),
            args: vec![],
            config: ".data".to_string(),
            config_type: crate::ast::ConfigType::Backtick,
            condition: None,
            parsed_join_targets: None,
            location: SourceLocation::default(),
        };
        assert_eq!(determine_step_name(&step), "jq");
    }

    #[test]
    fn test_determine_step_name_pipeline() {
        let step = PipelineStep::Regular {
            name: "pipeline".to_string(),
            args: vec![],
            config: "auth".to_string(),
            config_type: crate::ast::ConfigType::Identifier,
            condition: None,
            parsed_join_targets: None,
            location: SourceLocation::default(),
        };
        assert_eq!(determine_step_name(&step), "pipeline:auth");
    }

    #[test]
    fn test_determine_step_name_with_file() {
        let step = PipelineStep::Regular {
            name: "jq".to_string(),
            args: vec![],
            config: ".data".to_string(),
            config_type: crate::ast::ConfigType::Backtick,
            condition: None,
            parsed_join_targets: None,
            location: SourceLocation::with_file(1, 1, 0, "/path/to/main.wp".to_string()),
        };
        assert_eq!(determine_step_name(&step), "jq@main.wp");
    }

    #[test]
    fn test_determine_step_name_control_flow() {
        let if_step = PipelineStep::If {
            condition: Pipeline { steps: vec![] },
            then_branch: Pipeline { steps: vec![] },
            else_branch: None,
            location: SourceLocation::default(),
        };
        assert_eq!(determine_step_name(&if_step), "if");

        let foreach_step = PipelineStep::Foreach {
            selector: "data.items".to_string(),
            pipeline: Pipeline { steps: vec![] },
            location: SourceLocation::default(),
        };
        assert_eq!(determine_step_name(&foreach_step), "foreach");
    }

    #[test]
    fn test_detect_execution_mode() {
        assert!(matches!(
            detect_execution_mode("join", &None),
            ExecutionMode::Join
        ));
        assert!(matches!(
            detect_execution_mode("pipeline", &None),
            ExecutionMode::Recursive
        ));
        assert!(matches!(
            detect_execution_mode("jq", &None),
            ExecutionMode::Standard
        ));
    }

    #[test]
    fn test_check_cache_control_signal() {
        let no_control = serde_json::json!({"data": "test"});
        assert!(check_cache_control_signal(&no_control).is_none());

        let with_control = serde_json::json!({
            "_control": {
                "stop": true,
                "value": {"cached": "data"},
                "content_type": "text/html"
            }
        });
        let result = check_cache_control_signal(&with_control);
        assert!(result.is_some());
        let (value, ct) = result.unwrap();
        assert_eq!(value, serde_json::json!({"cached": "data"}));
        assert_eq!(ct, Some("text/html".to_string()));
    }
}
