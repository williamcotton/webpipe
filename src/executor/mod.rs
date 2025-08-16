use std::{collections::HashMap, pin::Pin, sync::Arc, future::Future};

use async_trait::async_trait;
use serde_json::Value;

use crate::{
    ast::{Pipeline, PipelineStep, Variable},
    error::WebPipeError,
    middleware::MiddlewareRegistry,
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

#[derive(Clone)]
pub struct ExecutionEnv {
    pub variables: Arc<Vec<Variable>>, // for variable resolution and auto-naming
    pub named_pipelines: Arc<HashMap<String, Arc<Pipeline>>>,
    pub invoker: Arc<dyn MiddlewareInvoker>,
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

pub fn execute_pipeline<'a>(
    env: &'a ExecutionEnv,
    pipeline: &'a Pipeline,
    mut input: Value,
) -> PipelineExecFuture<'a> {
    Box::pin(async move {
        let mut content_type = "application/json".to_string();
        let mut status_code_out: Option<u16> = None;

        for (idx, step) in pipeline.steps.iter().enumerate() {
            let is_last_step = idx + 1 == pipeline.steps.len();
            match step {
                PipelineStep::Regular { name, config, config_type: _ } => {
                    let (effective_config, effective_input, auto_named) =
                        resolve_config_and_autoname(&env.variables, name, config, &input);

                    let exec_result: Result<(Value, Option<u16>, String), WebPipeError> = if name == "pipeline" {
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
                            let mut next_input = if is_last_step { result } else { merge_values_preserving_input(&effective_input, &result) };
                            if auto_named {
                                if let Some(obj) = next_input.as_object_mut() { obj.remove("resultName"); }
                            }
                            input = next_input;

                            if name == "handlebars" {
                                content_type = "text/html".to_string();
                            } else if name == "pipeline" && is_last_step {
                                content_type = sub_content_type;
                            }
                            if is_last_step { if let Some(s) = sub_status { status_code_out = Some(s); } }
                        }
                        Err(e) => {
                            return Err(WebPipeError::MiddlewareExecutionError(e.to_string()));
                        }
                    }
                }
                PipelineStep::Result { branches } => {
                    return handle_result_step(env, branches, input, content_type).await;
                }
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
                "echo" => Ok(serde_json::json!({"echo": cfg, "inputCopy": input })),
                _ => Ok(serde_json::json!({"ok": true}))
            }
        }
    }

    fn env_with_vars(vars: Vec<Variable>) -> ExecutionEnv {
        ExecutionEnv {
            variables: Arc::new(vars),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
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
            PipelineStep::Regular { name: "echo".to_string(), config: "{}".to_string(), config_type: crate::ast::ConfigType::Quoted },
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
            PipelineStep::Regular { name: "handlebars".to_string(), config: "Hello".to_string(), config_type: crate::ast::ConfigType::Quoted }
        ]};
        let (_out, ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
        assert_eq!(ct, "text/html");
    }

    #[tokio::test]
    async fn variable_auto_naming_adds_and_removes_result_name() {
        let vars = vec![Variable { var_type: "echo".to_string(), name: "myVar".to_string(), value: "{}".to_string() }];
        let env = env_with_vars(vars);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "echo".to_string(), config: "myVar".to_string(), config_type: crate::ast::ConfigType::Identifier }
        ]};
        let (out, _ct, _st) = execute_pipeline(&env, &pipeline, serde_json::json!({})).await.unwrap();
        assert!(out.get("resultName").is_none());
        // Ensure output is an object and not empty
        assert!(out.is_object());
    }
}

