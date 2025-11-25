use std::{collections::HashMap, pin::Pin, sync::Arc, future::Future};

use async_trait::async_trait;
use serde_json::Value;
use tokio::task::JoinHandle;
use parking_lot::Mutex;

use crate::{
    ast::{Pipeline, PipelineStep, Variable},
    error::WebPipeError,
    middleware::MiddlewareRegistry,
    runtime::context::CacheStore,
};

/// Public alias for the complex future type returned by pipeline execution functions.
pub type PipelineExecFuture<'a> = Pin<
    Box<
        dyn Future<Output = Result<(Value, String, Option<u16>), WebPipeError>> + Send + 'a,
    >,
>;

#[async_trait]
pub trait MiddlewareInvoker: Send + Sync {
    async fn call(&self, name: &str, cfg: &str, input: &Value) -> Result<Value, WebPipeError>;
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

#[derive(Clone)]
pub struct ExecutionEnv {
    pub variables: Arc<Vec<Variable>>, // for variable resolution and auto-naming
    pub named_pipelines: Arc<HashMap<String, Arc<Pipeline>>>,
    pub invoker: Arc<dyn MiddlewareInvoker>,
    pub environment: Option<String>, // e.g. "production", "development", "staging"
    pub async_registry: AsyncTaskRegistry, // registry for @async tasks
    pub flags: Arc<HashMap<String, bool>>, // feature flags for @flag() tag checking
    pub cache: Option<CacheStore>, // cache store for pipeline-level caching
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
    async fn call(&self, name: &str, cfg: &str, input: &Value) -> Result<Value, WebPipeError> {
        self.registry.execute(name, cfg, input).await
    }
}

fn merge_values_preserving_input(input: &Value, result: &Value) -> Value {
    match (input, result) {
        (Value::Object(base), Value::Object(patch)) => {
            let mut merged = base.clone();
            for (k, v) in patch {
                merged.insert(k.clone(), v.clone());
            }
            Value::Object(merged)
        }
        _ => result.clone(),
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

fn should_execute_step(tags: &[crate::ast::Tag], env: &ExecutionEnv, input: &Value) -> bool {
    tags.iter().all(|tag| check_tag(tag, env, input))
}

fn check_tag(tag: &crate::ast::Tag, env: &ExecutionEnv, _input: &Value) -> bool {
    match tag.name.as_str() {
        "env" => check_env_tag(tag, env),
        "async" => true, // async doesn't affect execution, just how it runs
        "flag" => check_flag_tag(tag, env),
        "needs" => true, // @needs is only for static analysis, doesn't affect runtime
        _ => true,       // unknown tags don't prevent execution
    }
}

fn check_flag_tag(tag: &crate::ast::Tag, env: &ExecutionEnv) -> bool {
    if tag.args.is_empty() {
        return true; // Invalid @flag() tag without args, don't prevent execution
    }

    // Check all flag arguments - all must be enabled for step to execute
    for flag_name in &tag.args {
        let is_enabled = env.flags.get(flag_name.as_str())
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
/// Returns Some(cached_value) if cache hit occurred and pipeline should stop.
fn check_cache_control_signal(input: &Value) -> Option<Value> {
    input.get("_control")
        .and_then(|c| {
            if c.get("stop").and_then(|b| b.as_bool()).unwrap_or(false) {
                c.get("value").cloned()
            } else {
                None
            }
        })
}

/// Extract pending cache metadata from cache middleware on miss.
/// Returns (cache_key, ttl) if cache control metadata is present.
fn extract_pending_cache_metadata(result: &Value) -> Option<(String, u64)> {
    result.get("_metadata")
        .and_then(|m| m.get("cache_control"))
        .and_then(|c| {
            let key = c.get("key").and_then(|v| v.as_str())?;
            let ttl = c.get("ttl").and_then(|v| v.as_u64())?;
            Some((key.to_string(), ttl))
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

async fn handle_result_step<'a>(
    env: &'a ExecutionEnv,
    branches: &'a [crate::ast::ResultBranch],
    input: Value,
    mut content_type: String,
) -> Result<(Value, String, Option<u16>), WebPipeError> {
    let error_type = extract_error_type(&input);
    let selected = select_branch(branches, &error_type);
    if let Some(branch) = selected {
        let inherited_cookies = input.get("setCookies").cloned();
        let (mut result, branch_content_type, _status) = execute_pipeline(env, &branch.pipeline, input).await?;
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

/// Check if all remaining steps in the pipeline will be skipped
fn all_remaining_steps_will_be_skipped(
    steps: &[PipelineStep],
    current_idx: usize,
    env: &ExecutionEnv,
    input: &Value
) -> bool {
    for step in steps.iter().skip(current_idx + 1) {
        match step {
            PipelineStep::Regular { tags, .. } => {
                if should_execute_step(tags, env, input) {
                    return false; // Found a step that will execute
                }
            }
            PipelineStep::Result { .. } => {
                return false; // Result steps always execute
            }
        }
    }
    true // All remaining steps will be skipped
}

pub fn execute_pipeline<'a>(
    env: &'a ExecutionEnv,
    pipeline: &'a Pipeline,
    mut input: Value,
) -> PipelineExecFuture<'a> {
    Box::pin(async move {
        let mut content_type = "application/json".to_string();
        let mut status_code_out: Option<u16> = None;
        
        // Track cache details for saving at the end of the pipeline
        let mut pending_cache_key: Option<String> = None;
        let mut pending_cache_ttl: Option<u64> = None;

        for (idx, step) in pipeline.steps.iter().enumerate() {
            let is_last_step = idx + 1 == pipeline.steps.len();
            match step {
                PipelineStep::Regular { name, config, config_type: _, tags } => {
                    // Check if this step should execute based on tags
                    if !should_execute_step(tags, env, &input) {
                        continue; // Skip this step
                    }

                    // Check if this step should run asynchronously
                    if let Some(async_name) = get_async_tag(tags) {
                        // Spawn async task with current state snapshot
                        let env_clone = env.clone();
                        let name_clone = name.clone();
                        let config_clone = config.clone();
                        let input_snapshot = input.clone();

                        let handle = tokio::spawn(async move {
                            let (effective_config, effective_input, _auto_named) =
                                resolve_config_and_autoname(&env_clone.variables, &name_clone, &config_clone, &input_snapshot);

                            if name_clone == "pipeline" {
                                let pipeline_name = effective_config.trim();
                                if let Some(p) = env_clone.named_pipelines.get(pipeline_name) {
                                    let (val, _ct, _st) = execute_pipeline(&env_clone, p, effective_input.clone()).await?;
                                    Ok(val)
                                } else {
                                    Err(WebPipeError::PipelineNotFound(pipeline_name.to_string()))
                                }
                            } else {
                                env_clone.invoker.call(&name_clone, &effective_config, &effective_input).await
                            }
                        });

                        env.async_registry.register(async_name, handle);
                        // Continue to next step without modifying input
                        continue;
                    }

                    let (effective_config, effective_input, auto_named) =
                        resolve_config_and_autoname(&env.variables, name, config, &input);

                    // Special handling for join middleware
                    let exec_result: Result<(Value, Option<u16>, String), WebPipeError> = if name == "join" {
                        // Parse task names from config
                        let task_names = parse_join_task_names(&effective_config)?;

                        // Wait for all async tasks
                        let mut async_results = serde_json::Map::new();
                        for task_name in task_names {
                            if let Some(handle) = env.async_registry.take(&task_name) {
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
                            } else {
                                // Task not found - could be a warning but we'll skip it
                                // In dev mode, this could log a warning
                            }
                        }

                        // Merge results into input under .async key
                        let mut result = effective_input.clone();
                        if let Some(obj) = result.as_object_mut() {
                            obj.insert("async".to_string(), Value::Object(async_results));
                        }
                        Ok((result, None, content_type.clone()))
                    } else if name == "pipeline" {
                        let name = effective_config.trim();
                        if let Some(p) = env.named_pipelines.get(name) {
                            let (val, ct, st) = execute_pipeline(env, p, effective_input.clone()).await?;
                            Ok((val, st, ct))
                        } else {
                            Err(WebPipeError::PipelineNotFound(name.to_string()))
                        }
                    } else {
                        let val = env.invoker.call(name, &effective_config, &effective_input).await?;
                        Ok((val, None, content_type.clone()))
                    };

                    match exec_result {
                        Ok((result, sub_status, sub_content_type)) => {
                            // 1. CHECK FOR STOP SIGNAL (Cache Hit)
                            // If cache middleware returned a hit, return the cached value immediately
                            if let Some(cached_val) = check_cache_control_signal(&result) {
                                return Ok((cached_val, sub_content_type, sub_status));
                            }

                            
                            // // If cache middleware set cache_control metadata, track it for saving at end
                            // if let Some((key, ttl)) = extract_pending_cache_metadata(&result) {
                            //     pending_cache_key = Some(key);
                            //     pending_cache_ttl = Some(ttl);
                            // }
                            // Only extract cache metadata from the actual "cache" middleware step
                            // to prevent nested pipelines from inheriting and saving with the same key
                            if name == "cache" {
                                if let Some((key, ttl)) = extract_pending_cache_metadata(&result) {
                                    pending_cache_key = Some(key);
                                    pending_cache_ttl = Some(ttl);
                                }
                            }

                            // Check if this is the effective last step (either last in array, or all remaining steps will be skipped)
                            let is_effective_last_step = is_last_step || all_remaining_steps_will_be_skipped(&pipeline.steps, idx, env, &input);

                            let mut next_input = if is_effective_last_step { result } else { merge_values_preserving_input(&effective_input, &result) };
                            
                            // Strip _metadata.cache_control after the cache step to prevent it from
                            // leaking into nested pipelines (which would cause them to save with the same key)
                            if name == "cache" {
                                if let Some(obj) = next_input.as_object_mut() {
                                    if let Some(meta) = obj.get_mut("_metadata").and_then(|m| m.as_object_mut()) {
                                        meta.remove("cache_control");
                                    }
                                }
                            }
                            if auto_named {
                                if let Some(obj) = next_input.as_object_mut() { obj.remove("resultName"); }
                            }
                            input = next_input;

                            if name == "handlebars" {
                                content_type = "text/html".to_string();
                            } else if name == "pipeline" && is_effective_last_step {
                                content_type = sub_content_type;
                            }
                            if is_effective_last_step { if let Some(s) = sub_status { status_code_out = Some(s); } }
                        }
                        Err(e) => {
                            return Err(WebPipeError::MiddlewareExecutionError(e.to_string()));
                        }
                    }
                }
                PipelineStep::Result { branches } => {
                    // Before handling result, save to cache if pending
                    // Use "write-once" semantics to prevent race conditions
                    if let (Some(key), Some(ttl), Some(store)) = (&pending_cache_key, pending_cache_ttl, &env.cache) {
                        // Check if cache already has valid data
                        if store.get(key).is_none() {
                            // Clean up _metadata.cache_control before caching
                            let mut cache_value = input.clone();
                            if let Some(obj) = cache_value.as_object_mut() {
                                if let Some(meta) = obj.get_mut("_metadata").and_then(|m| m.as_object_mut()) {
                                    meta.remove("cache_control");
                                    // Also remove the regular cache metadata
                                    meta.remove("cache");
                                }
                            }
                            store.put(key.clone(), cache_value, Some(ttl));
                        }
                    }
                    return handle_result_step(env, branches, input, content_type).await;
                }
            }
        }

        // 3. SAVE TO CACHE IF PENDING
        // At end of pipeline, save the final result to cache if we have a pending key
        // Use "write-once" semantics: don't overwrite if valid data already exists
        // This prevents race conditions where a slow request overwrites good cached data
        if let (Some(key), Some(ttl), Some(store)) = (pending_cache_key, pending_cache_ttl, &env.cache) {
            // Check if cache already has valid data (another request may have written it)
            if store.get(&key).is_none() {
                // Clean up _metadata.cache_control before caching
                let mut cache_value = input.clone();
                if let Some(obj) = cache_value.as_object_mut() {
                    if let Some(meta) = obj.get_mut("_metadata").and_then(|m| m.as_object_mut()) {
                        meta.remove("cache_control");
                        // Also remove the regular cache metadata
                        meta.remove("cache");
                    }
                }
                store.put(key, cache_value, Some(ttl));
            }
        }

        Ok((input, content_type, status_code_out))
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{PipelineStep, Pipeline};
    use std::sync::Arc;

    struct StubInvoker;
    #[async_trait]
    impl MiddlewareInvoker for StubInvoker {
        async fn call(&self, name: &str, cfg: &str, input: &Value) -> Result<Value, WebPipeError> {
            match name {
                "handlebars" => Ok(Value::String(format!("<p>{}</p>", cfg))),
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
                        Ok(result)
                    } else {
                        Ok(serde_json::json!({"echo": cfg, "inputCopy": input }))
                    }
                },
                _ => Ok(serde_json::json!({"ok": true}))
            }
        }
    }

    fn env_with_vars(vars: Vec<Variable>) -> ExecutionEnv {
        ExecutionEnv {
            variables: Arc::new(vars),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
            environment: None,
            async_registry: AsyncTaskRegistry::new(),
            flags: Arc::new(HashMap::new()),
            cache: None,
        }
    }

    #[tokio::test]
    async fn merge_values_preserving_input_for_objects() {
        let input = serde_json::json!({"a":1});
        let result = serde_json::json!({"b":2});
        let merged = super::merge_values_preserving_input(&input, &result);
        assert_eq!(merged, serde_json::json!({"a":1,"b":2}));
    }

    #[tokio::test]
    async fn result_branch_selection_and_status() {
        let env = env_with_vars(vec![]);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "echo".to_string(), config: "{}".to_string(), config_type: crate::ast::ConfigType::Quoted, tags: vec![] },
            PipelineStep::Result { branches: vec![
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Ok, status_code: 201, pipeline: Pipeline { steps: vec![] } },
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Default, status_code: 200, pipeline: Pipeline { steps: vec![] } },
            ]}
        ]};
        let (out, _ct, status) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
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
        let (_out, _ct, status) = execute_pipeline(&env, &pipeline, input).await.unwrap();
        assert_eq!(status, Some(422));
    }

    #[tokio::test]
    async fn handlebars_sets_html_content_type() {
        let env = env_with_vars(vec![]);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "handlebars".to_string(), config: "Hello".to_string(), config_type: crate::ast::ConfigType::Quoted, tags: vec![] }
        ]};
        let (_out, ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
        assert_eq!(ct, "text/html");
    }

    #[tokio::test]
    async fn variable_auto_naming_adds_and_removes_result_name() {
        let vars = vec![Variable { var_type: "echo".to_string(), name: "myVar".to_string(), value: "{}".to_string() }];
        let env = env_with_vars(vars);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "echo".to_string(), config: "myVar".to_string(), config_type: crate::ast::ConfigType::Identifier, tags: vec![] }
        ]};
        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: false, args: vec!["production".to_string()] }]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: false, args: vec!["production".to_string()] }]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({"initial": "data"})).await.unwrap();
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: true, args: vec!["production".to_string()] }]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({"initial": "data"})).await.unwrap();
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: true, args: vec!["production".to_string()] }]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
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
                ]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
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
                tags: vec![crate::ast::Tag { name: "env".to_string(), negated: false, args: vec!["production".to_string()] }]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
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
                ]
            }
        ]};

        // No flags in env - should fail closed (skip the step)
        let (out, _ct, _st) = execute_pipeline(&env_no_flags, &pipeline, serde_json::json!({})).await.unwrap();
        assert!(out.get("echo").is_none());

        // With flag enabled in env - should execute
        let mut flags = HashMap::new();
        flags.insert("beta".to_string(), true);
        let env_with_flag = ExecutionEnv {
            variables: Arc::new(vec![]),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
            environment: None,
            async_registry: AsyncTaskRegistry::new(),
            flags: Arc::new(flags),
            cache: None,
        };
        let (out2, _ct2, _st2) = execute_pipeline(&env_with_flag, &pipeline, serde_json::json!({})).await.unwrap();
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
                ]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
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
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["task1".to_string()] }]
            },
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"sync": "data"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();

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
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["task1".to_string()] }]
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: "task1".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();

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
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["task1".to_string()] }]
            },
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"data": "task2"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["task2".to_string()] }]
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: "task1,task2".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();

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
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["a".to_string()] }]
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: r#"["a"]"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();

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
                tags: vec![]
            },
            // Async task should see counter: 1
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"asyncSaw": 0}"#.to_string(), // Will be replaced with actual counter value
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![crate::ast::Tag { name: "async".to_string(), negated: false, args: vec!["snapshot".to_string()] }]
            },
            // Modify state after async spawn
            PipelineStep::Regular {
                name: "echo".to_string(),
                config: r#"{"counter": 2}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![]
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                config: "snapshot".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                tags: vec![]
            }
        ]};

        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();

        // Main state should have counter: 2
        assert_eq!(out.get("counter").and_then(|v| v.as_u64()), Some(2));

        // Async task should have seen the snapshot with counter: 1
        let async_obj = out.get("async").unwrap().as_object().unwrap();
        let snapshot_result = async_obj.get("snapshot").unwrap();
        // The async echo will copy the input which had counter: 1
        assert!(snapshot_result.get("counter").is_some());
    }
}

