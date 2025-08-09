use crate::ast::{Mock, Pipeline, PipelineRef, PipelineStep, Program, Variable, When};
use crate::error::WebPipeError;
use crate::middleware::MiddlewareRegistry;
use crate::executor::{ExecutionEnv, MiddlewareInvoker};
use crate::runtime::Context;
use crate::config;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use crate::http::request::build_minimal_request_for_tests;

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

#[derive(Clone)]
struct MockingInvoker {
    registry: Arc<MiddlewareRegistry>,
    mocks: MockResolver,
}

#[async_trait::async_trait]
impl MiddlewareInvoker for MockingInvoker {
    async fn call(&self, name: &str, cfg: &str, input: &Value) -> Result<Value, WebPipeError> {
        if let Some(mock_val) = self.mocks.get_middleware_mock(name, input) {
            return Ok(mock_val.clone());
        }
        self.registry.execute(name, cfg, input).await
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

// number coercion helpers are provided by http::request for server/test build paths

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

fn execute_pipeline_with_env<'a>(
    env: &'a ExecutionEnv,
    pipeline: &'a Pipeline,
    input: Value,
) -> Pin<Box<dyn Future<Output = Result<(Value, Option<u16>), WebPipeError>> + Send + 'a>> {
    Box::pin(async move {
        let (out, _ct, status) = crate::executor::execute_pipeline(env, pipeline, input).await?;
        Ok((out, status))
    })
}

// Removed local helper functions in favor of shared executor

fn find_variable<'a>(variables: &'a [Variable], var_type: &str, name: &str) -> Option<&'a Variable> {
    variables.iter().find(|v| v.var_type == var_type && v.name == name)
}

pub async fn run_tests(program: Program) -> Result<TestSummary, WebPipeError> {
    // Initialize global config (Context builder will also set globals and register partials)
    config::init_global(program.configs.clone());

    let ctx = Context::from_program_configs(program.configs.clone(), &program.variables).await?;
    let registry: Arc<MiddlewareRegistry> = Arc::new(MiddlewareRegistry::with_builtins(std::sync::Arc::new(ctx)));

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
                            let selected_pipeline = m.value.0;
                            let selected_pipeline_name = m.value.1;
                            let params_map = m
                                .params
                                .iter()
                                .map(|(k, v)| (k.to_string(), v.to_string()))
                                .collect::<HashMap<_, _>>();

                            // Build request JSON via shared helper
                            let input = build_minimal_request_for_tests(method, &path_str, &params_map, &query_map);

                            // Build shared ExecutionEnv with MockingInvoker
                            let named: HashMap<String, Arc<Pipeline>> = program
                                .pipelines
                                .iter()
                                .map(|p| (p.name.clone(), Arc::new(p.pipeline.clone())))
                                .collect();

                            let env = ExecutionEnv {
                                variables: Arc::new(program.variables.clone()),
                                named_pipelines: Arc::new(named),
                                invoker: Arc::new(MockingInvoker { registry: registry.clone(), mocks: mocks.clone() }),
                            };

                            // Pipeline-level mock when route uses a named pipeline
                            if let Some(name) = selected_pipeline_name {
                                if let Some(mock) = mocks.get_pipeline_mock(name) {
                                    (200u16, mock.clone(), true, String::new())
                                } else {
                                    let (out, s) = execute_pipeline_with_env(&env, selected_pipeline, input).await?;
                                    (s.unwrap_or(200u16), out, true, String::new())
                                }
                            } else {
                                let (out, s) = execute_pipeline_with_env(&env, selected_pipeline, input).await?;
                                (s.unwrap_or(200u16), out, true, String::new())
                            }
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
                        let named: HashMap<String, Arc<Pipeline>> = program
                            .pipelines
                            .iter()
                            .map(|p| (p.name.clone(), Arc::new(p.pipeline.clone())))
                            .collect();
                        let env = ExecutionEnv {
                            variables: Arc::new(program.variables.clone()),
                            named_pipelines: Arc::new(named),
                            invoker: Arc::new(MockingInvoker { registry: registry.clone(), mocks: mocks.clone() }),
                        };
                        let (out, status_opt) = execute_pipeline_with_env(&env, &pipeline.pipeline, input).await?;
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
                        let named: HashMap<String, Arc<Pipeline>> = program
                            .pipelines
                            .iter()
                            .map(|p| (p.name.clone(), Arc::new(p.pipeline.clone())))
                            .collect();
                        let env = ExecutionEnv {
                            variables: Arc::new(program.variables.clone()),
                            named_pipelines: Arc::new(named),
                            invoker: Arc::new(MockingInvoker { registry: registry.clone(), mocks: mocks.clone() }),
                        };
                        let (out, status_opt) = execute_pipeline_with_env(&env, &pipeline, input).await?;
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


