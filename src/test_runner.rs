use crate::ast::{Mock, Pipeline, PipelineRef, PipelineStep, Program, ResultBranchType, Variable, When};
use crate::error::WebPipeError;
use crate::middleware::MiddlewareRegistry;
use crate::runtime::Context;
use crate::config;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
struct MockResolver {
    // middleware -> mocked value
    middleware_mocks: HashMap<String, Value>,
    // (middleware, variable_name) -> mocked value
    variable_mocks: HashMap<(String, String), Value>,
    // pipeline_name -> mocked value
    pipeline_mocks: HashMap<String, Value>,
}

impl MockResolver {
    fn empty() -> Self {
        Self {
            middleware_mocks: HashMap::new(),
            variable_mocks: HashMap::new(),
            pipeline_mocks: HashMap::new(),
        }
    }

    fn from_mocks(mocks: &[Mock]) -> Result<Self, WebPipeError> {
        let mut resolver = Self::empty();
        for m in mocks {
            let value = parse_backticked_json(&m.return_value)?;
            if let Some((left, right)) = m.target.split_once('.') {
                // Could be pipeline.<name> or <middleware>.<variable>
                if left == "pipeline" {
                    resolver.pipeline_mocks.insert(right.to_string(), value);
                } else {
                    resolver
                        .variable_mocks
                        .insert((left.to_string(), right.to_string()), value);
                }
            } else {
                // Middleware-level mock
                resolver
                    .middleware_mocks
                    .insert(m.target.to_string(), value);
            }
        }
        Ok(resolver)
    }

    fn overlay(&self, other: &MockResolver) -> MockResolver {
        // other takes precedence
        let mut merged = self.clone();
        merged.middleware_mocks.extend(other.middleware_mocks.clone());
        merged.variable_mocks.extend(other.variable_mocks.clone());
        merged.pipeline_mocks.extend(other.pipeline_mocks.clone());
        merged
    }

    fn get_pipeline_mock(&self, name: &str) -> Option<&Value> {
        self.pipeline_mocks.get(name)
    }

    fn get_middleware_mock<'a>(&'a self, middleware: &str, input: &Value) -> Option<&'a Value> {
        // Prefer variable-specific mock if resultName hints at variable name
        if let Some(var_name) = input.get("resultName").and_then(|v| v.as_str()) {
            if let Some(v) = self.variable_mocks.get(&(middleware.to_string(), var_name.to_string())) {
                return Some(v);
            }
        }
        self.middleware_mocks.get(middleware)
    }
}

#[derive(Debug, Clone)]
pub struct TestOutcome {
    pub describe: String,
    pub test: String,
    pub passed: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct TestSummary {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub outcomes: Vec<TestOutcome>,
}

fn string_to_number_or_string(s: &str) -> Value {
    if let Ok(i) = s.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(f) = s.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(s.to_string())
}

fn string_map_to_json_with_number_coercion(map: &HashMap<String, String>) -> Value {
    let mut obj = serde_json::Map::new();
    for (k, v) in map {
        obj.insert(k.clone(), string_to_number_or_string(v));
    }
    Value::Object(obj)
}

fn parse_backticked_json(s: &str) -> Result<Value, WebPipeError> {
    let trimmed = s.trim();
    let content = if let (Some(start), Some(end)) = (trimmed.find('`'), trimmed.rfind('`')) {
        if end > start { &trimmed[start + 1..end] } else { trimmed }
    } else {
        trimmed
    };
    // Try JSON first; if fails, treat as a string literal
    match serde_json::from_str::<Value>(content) {
        Ok(v) => Ok(v),
        Err(_) => Ok(Value::String(content.to_string())),
    }
}

fn parse_optional_input(input: &Option<String>) -> Result<Value, WebPipeError> {
    if let Some(s) = input {
        parse_backticked_json(s)
    } else {
        Ok(Value::Object(serde_json::Map::new()))
    }
}

use std::future::Future;
use std::pin::Pin;

fn execute_pipeline_with_mocks<'a>(
    pipeline: &'a Pipeline,
    input: Value,
    registry: &'a MiddlewareRegistry,
    mocks: &'a MockResolver,
    variables: &'a [Variable],
) -> Pin<Box<dyn Future<Output = Result<(Value, Option<u16>), WebPipeError>> + Send + 'a>> {
    Box::pin(async move {
        let mut current_input = input;
        let mut status_code: Option<u16> = None;

        for (idx, step) in pipeline.steps.iter().enumerate() {
            let is_last_step = idx + 1 == pipeline.steps.len();
            match step {
                PipelineStep::Regular { name, config } => {
                    // Resolve variable references and auto-name like the server
                    let (effective_config, mut effective_input, auto_named) = resolve_config_and_autoname(variables, name, config, &current_input);

                    if let Some(mock_val) = mocks.get_middleware_mock(name, &effective_input) {
                        let result = mock_val.clone();
                        let mut next = if is_last_step { result } else { merge_values_preserving_input(&effective_input, &result) };
                        if auto_named { if let Some(obj) = next.as_object_mut() { obj.remove("resultName"); } }
                        current_input = next;
                    } else {
                        let result = registry.execute(name, &effective_config, &effective_input).await?;
                        let mut next = if is_last_step { result } else { merge_values_preserving_input(&effective_input, &result) };
                        if auto_named { if let Some(obj) = next.as_object_mut() { obj.remove("resultName"); } }
                        current_input = next;
                    }
                }
                PipelineStep::Result { branches } => {
                    // Emulate server's result routing
                    let error_type = extract_error_type(&current_input);
                    let selected = select_branch(branches, &error_type);
                    if let Some(branch) = selected {
                        let (res, _status_unused) = execute_pipeline_with_mocks(&branch.pipeline, current_input, registry, mocks, variables).await?;
                        current_input = res;
                        status_code = Some(branch.status_code);
                        return Ok((current_input, status_code));
                    } else {
                        return Ok((current_input, status_code));
                    }
                }
            }
        }

        Ok((current_input, status_code))
    })
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
    if let Some(err_type) = error_type {
        for branch in branches {
            if let ResultBranchType::Custom(name) = &branch.branch_type {
                if name == err_type { return Some(branch); }
            }
        }
    }
    if error_type.is_none() {
        for branch in branches {
            if matches!(branch.branch_type, ResultBranchType::Ok) { return Some(branch); }
        }
    }
    for branch in branches {
        if matches!(branch.branch_type, ResultBranchType::Default) { return Some(branch); }
    }
    None
}

fn find_variable<'a>(variables: &'a [Variable], var_type: &str, name: &str) -> Option<&'a Variable> {
    variables.iter().find(|v| v.var_type == var_type && v.name == name)
}

pub async fn run_tests(program: Program) -> Result<TestSummary, WebPipeError> {
    // Initialize global config (Context builder will also set globals and register partials)
    config::init_global(program.configs.clone());

    let ctx = Context::from_program_configs(program.configs.clone(), &program.variables).await?;
    let registry = MiddlewareRegistry::with_builtins(std::sync::Arc::new(ctx));

    let mut outcomes: Vec<TestOutcome> = Vec::new();

    for describe in &program.describes {
        let describe_mocks = MockResolver::from_mocks(&describe.mocks)?;
        for test in &describe.tests {
            let test_mocks = MockResolver::from_mocks(&test.mocks)?;
            // Test-level mocks override describe-level mocks (overlay describe <- test)
            let mocks = describe_mocks.overlay(&test_mocks);

            let (status, output_value, pass, msg) = match &test.when {
                When::CallingRoute { method, path } => {
                    // Split query string if present
                    let mut path_str = path.clone();
                    let mut query_map: HashMap<String, String> = HashMap::new();
                    if let Some(qpos) = path_str.find('?') {
                        let qs = path_str[qpos + 1..].to_string();
                        path_str = path_str[..qpos].to_string();
                        for pair in qs.split('&') {
                            if pair.is_empty() { continue; }
                            let mut it = pair.splitn(2, '=');
                            let k = it.next().unwrap_or("").to_string();
                            let v = it.next().unwrap_or("").to_string();
                            if !k.is_empty() { query_map.insert(k, v); }
                        }
                    }

                    // Build a matchit router for this method
                    let mut router: matchit::Router<(&Pipeline, Option<&str>)> = matchit::Router::new();
                    let mut selected_pipeline: Option<&Pipeline> = None;
                    let mut selected_pipeline_name: Option<&str> = None;
                    for route in &program.routes {
                        if &route.method == method {
                            match &route.pipeline {
                                PipelineRef::Inline(p) => { let _ = router.insert(route.path.clone(), (p, None)); },
                                PipelineRef::Named(name) => {
                                    if let Some(named) = program.pipelines.iter().find(|np| np.name == *name) {
                                        let _ = router.insert(route.path.clone(), (&named.pipeline, Some(name.as_str())));
                                    }
                                }
                            }
                        }
                    }
                    match router.at(&path_str) {
                        Ok(m) => {
                            selected_pipeline = Some(m.value.0);
                            selected_pipeline_name = m.value.1;
                            let params_map = m
                                .params
                                .iter()
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .collect::<HashMap<_, _>>();

                            // Build request JSON similar to server
                            let mut request_obj = serde_json::Map::new();
                            request_obj.insert("method".to_string(), Value::String(method.clone()));
                            request_obj.insert("path".to_string(), Value::String(path_str.clone()));
                            request_obj.insert("query".to_string(), string_map_to_json_with_number_coercion(&query_map));
                            request_obj.insert("params".to_string(), string_map_to_json_with_number_coercion(&params_map));
                            request_obj.insert("headers".to_string(), Value::Object(serde_json::Map::new()));
                            request_obj.insert("cookies".to_string(), Value::Object(serde_json::Map::new()));
                            request_obj.insert("body".to_string(), Value::Object(serde_json::Map::new()));
                            request_obj.insert("content_type".to_string(), Value::String("application/json".to_string()));
                            let input = Value::Object(request_obj);

                            // Pipeline-level mock when route uses a named pipeline
                        let mut status = 200u16;
                            let output_value = if let Some(name) = selected_pipeline_name {
                                if let Some(mock) = mocks.get_pipeline_mock(name) {
                                    mock.clone()
                                } else {
                                    let (out, s) = execute_pipeline_with_mocks(selected_pipeline.unwrap(), input, &registry, &mocks, &program.variables).await?;
                                    if let Some(sc) = s { status = sc; }
                                    out
                                }
                            } else {
                                let (out, s) = execute_pipeline_with_mocks(selected_pipeline.unwrap(), input, &registry, &mocks, &program.variables).await?;
                                if let Some(sc) = s { status = sc; }
                                out
                            };

                            (status, output_value, true, String::new())
                        }
                        Err(_) => (404u16, Value::Object(serde_json::Map::new()), false, format!("Route not found: {} {}", method, path)),
                    }
                }
                When::ExecutingPipeline { name } => {
                    // If pipeline mock exists, return it directly
                    if let Some(mock) = mocks.get_pipeline_mock(name) {
                        (200u16, mock.clone(), true, String::new())
                    } else {
                        let input = parse_optional_input(&test.input)?;
                        let pipeline = program
                            .pipelines
                            .iter()
                            .find(|p| p.name == *name)
                            .ok_or_else(|| WebPipeError::PipelineNotFound(name.clone()))?;
                        let (out, status_opt) = execute_pipeline_with_mocks(&pipeline.pipeline, input, &registry, &mocks, &program.variables).await?;
                        (status_opt.unwrap_or(200u16), out, true, String::new())
                    }
                }
                When::ExecutingVariable { var_type, name } => {
                    let var = find_variable(&program.variables, var_type, name)
                        .ok_or_else(|| WebPipeError::BadRequest(format!("Variable not found: {} {}", var_type, name)))?;
                    let input = parse_optional_input(&test.input)?;
                    // If there's an explicit variable-level mock, return it directly
                    if let Some(mock) = mocks.variable_mocks.get(&(var.var_type.clone(), var.name.clone())) {
                        (200u16, mock.clone(), true, String::new())
                    } else {
                    // Single-step pipeline invoking the variable's middleware
                    let pipeline = Pipeline { steps: vec![PipelineStep::Regular { name: var.var_type.clone(), config: var.value.clone() }] };
                    let (out, status_opt) = execute_pipeline_with_mocks(&pipeline, input, &registry, &mocks, &program.variables).await?;
                    (status_opt.unwrap_or(200u16), out, true, String::new())
                    }
                }
            };

            // Evaluate conditions
            let mut cond_pass = pass;
            let mut failure_msgs: Vec<String> = Vec::new();
            for cond in &test.conditions {
                let field = cond.field.trim();
                let cmp = cond.comparison.trim();
                let val_str = cond.value.trim();
                match (field, cmp) {
                    ("status", "is") | ("status", "equals") => {
                        if let Ok(expected) = val_str.parse::<u16>() {
                            if status != expected { cond_pass = false; failure_msgs.push(format!("expected status {} got {}", expected, status)); }
                        } else {
                            cond_pass = false; failure_msgs.push(format!("invalid expected status: {}", val_str));
                        }
                    }
                    ("output", "equals") => {
                        let expected = parse_backticked_json(val_str)?;
                        if output_value != expected {
                            cond_pass = false;
                            failure_msgs.push(format!("output mismatch"));
                        }
                    }
                    _ => {
                        // Unsupported condition -> mark failure to surface quickly
                        cond_pass = false;
                        failure_msgs.push(format!("unsupported condition: {} {} {}", field, cmp, val_str));
                    }
                }
            }

            if cond_pass {
                outcomes.push(TestOutcome { describe: describe.name.clone(), test: test.name.clone(), passed: true, message: String::new() });
            } else {
                let msg = if !failure_msgs.is_empty() { failure_msgs.join("; ") } else { msg };
                outcomes.push(TestOutcome { describe: describe.name.clone(), test: test.name.clone(), passed: false, message: msg });
            }
        }
    }

    let total = outcomes.len();
    let passed = outcomes.iter().filter(|o| o.passed).count();
    let failed = total - passed;

    Ok(TestSummary { total, passed, failed, outcomes })
}


