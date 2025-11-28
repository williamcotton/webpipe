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
use regex::Regex;

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
    async fn call(
        &self,
        name: &str,
        cfg: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
    ) -> Result<(), WebPipeError> {
        if let Some(mock_val) = self.mocks.get_middleware_mock(name, &pipeline_ctx.state) {
            pipeline_ctx.state = mock_val.clone();
            return Ok(());
        }
        self.registry.execute(name, cfg, pipeline_ctx, _env, _ctx).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn mock_resolver_targets_pipeline_and_variable_and_middleware() {
        let mocks = vec![
            Mock { target: "pipeline.inner".to_string(), return_value: "`{\"ok\":true}`".to_string() },
            Mock { target: "fetch.resultA".to_string(), return_value: "`{}`".to_string() },
            Mock { target: "cache".to_string(), return_value: "`{}`".to_string() },
        ];
        let res = MockResolver::from_mocks(&mocks).unwrap();
        assert!(res.get_pipeline_mock("inner").is_some());
        assert!(res.get_middleware_mock("cache", &json!({})).is_some());
        let with_name = res.get_middleware_mock("fetch", &json!({"resultName":"resultA"}));
        assert!(with_name.is_some());
    }

    #[tokio::test]
    async fn test_conditions_status_range_and_output_contains_and_matches() {
        let program_src = r#"
        pipeline ok =
          |> jq: `{ a: 1, s: "<p>x</p>" }`

        describe "c"
          it "d"
            when executing pipeline ok
            then status in 200..299
            and output contains `{ "a": 1 }`
            and output `.s` matches `^<p>x</p>$`
        "#;
        let (_rest, program) = crate::ast::parse_program(program_src).unwrap();
        let summary = run_tests(program, false).await.unwrap();
        assert_eq!(summary.total, 1);
        assert_eq!(summary.failed, 0);
    }

    #[test]
    fn parse_backticked_json_handles_strings_and_json() {
        let j = parse_backticked_json("`{\"x\":1}`").unwrap();
        assert_eq!(j["x"], json!(1));
        let s = parse_backticked_json("`hello`").unwrap();
        assert_eq!(s, json!("hello"));
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

 

// Removed local helper functions in favor of shared executor

fn find_variable<'a>(variables: &'a [Variable], var_type: &str, name: &str) -> Option<&'a Variable> {
    variables.iter().find(|v| v.var_type == var_type && v.name == name)
}

pub async fn run_tests(program: Program, verbose: bool) -> Result<TestSummary, WebPipeError> {
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

            let (status, output_value, content_type, pass, msg) = match &test.when {
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
                                environment: None,
                                
                                
                                cache: crate::runtime::context::CacheStore::new(8, 60),
                    rate_limit: crate::runtime::context::RateLimitStore::new(1000),
                                
                            };

                            // Pipeline-level mock when route uses a named pipeline
                            if let Some(name) = selected_pipeline_name {
                                if let Some(mock) = mocks.get_pipeline_mock(name) {
                                    (200u16, mock.clone(), "application/json".to_string(), true, String::new())
                                } else {
                                    let ctx = crate::executor::RequestContext::new();
                    let (out, ct, s, _ctx) = crate::executor::execute_pipeline(&env, selected_pipeline, input, ctx).await?;
                                    (s.unwrap_or(200u16), out, ct, true, String::new())
                                }
                            } else {
                                let ctx = crate::executor::RequestContext::new();
                    let (out, ct, s, _ctx) = crate::executor::execute_pipeline(&env, selected_pipeline, input, ctx).await?;
                                (s.unwrap_or(200u16), out, ct, true, String::new())
                            }
                        }
                        Err(_) => (404u16, Value::Object(serde_json::Map::new()), "application/json".to_string(), false, format!("Route not found: {} {}", method, path)),
                    }
                }
                When::ExecutingPipeline { name } => {
                    // If pipeline mock exists, return it directly
                    if let Some(mock) = mocks.get_pipeline_mock(name) {
                        (200u16, mock.clone(), "application/json".to_string(), true, String::new())
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
                            environment: None,
                            
                            
                            cache: crate::runtime::context::CacheStore::new(8, 60),
                    rate_limit: crate::runtime::context::RateLimitStore::new(1000),
                                
                        };
                        let ctx = crate::executor::RequestContext::new();
                        let (out, ct, status_opt, _ctx) = crate::executor::execute_pipeline(&env, &pipeline.pipeline, input, ctx).await?;
                        (status_opt.unwrap_or(200u16), out, ct, true, String::new())
                    }
                }
                When::ExecutingVariable { var_type, name } => {
                    let var = find_variable(&program.variables, var_type, name)
                        .ok_or_else(|| WebPipeError::BadRequest(format!("Variable not found: {} {}", var_type, name)))?;
                    let input = parse_optional_input(&test.input)?;
                    // If there's an explicit variable-level mock, return it directly
                    if let Some(mock) = mocks.variable_mocks.get(&(var.var_type.clone(), var.name.clone())) {
                        (200u16, mock.clone(), "application/json".to_string(), true, String::new())
                    } else {
                        // Single-step pipeline invoking the variable's middleware
                        let pipeline = Pipeline { steps: vec![PipelineStep::Regular { name: var.var_type.clone(), config: var.value.clone(), config_type: crate::ast::ConfigType::Backtick, tags: vec![], parsed_join_targets: None }] };
                        let named: HashMap<String, Arc<Pipeline>> = program
                            .pipelines
                            .iter()
                            .map(|p| (p.name.clone(), Arc::new(p.pipeline.clone())))
                            .collect();
                        let env = ExecutionEnv {
                            variables: Arc::new(program.variables.clone()),
                            named_pipelines: Arc::new(named),
                            invoker: Arc::new(MockingInvoker { registry: registry.clone(), mocks: mocks.clone() }),
                            environment: None,
                            
                            
                            cache: crate::runtime::context::CacheStore::new(8, 60),
                    rate_limit: crate::runtime::context::RateLimitStore::new(1000),
                                
                        };
                        let ctx = crate::executor::RequestContext::new();
                        let (out, ct, status_opt, _ctx) = crate::executor::execute_pipeline(&env, &pipeline, input, ctx).await?;
                        (status_opt.unwrap_or(200u16), out, ct, true, String::new())
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

                // Helpers
                fn deep_contains(actual: &Value, expected: &Value) -> bool {
                    match (actual, expected) {
                        (Value::Object(ao), Value::Object(eo)) => eo
                            .iter()
                            .all(|(k, ev)| ao.get(k).is_some_and(|av| deep_contains(av, ev))),
                        (Value::Array(aa), Value::Array(ea)) => {
                            // Every expected element must appear at least once in actual
                            ea.iter().all(|ev| aa.iter().any(|av| av == ev || deep_contains(av, ev)))
                        }
                        (Value::String(as_) , Value::String(es_)) => as_.contains(es_),
                        _ => actual == expected,
                    }
                }
                fn value_to_string(v: &Value) -> String {
                    match v {
                        Value::String(s) => s.clone(),
                        _ => serde_json::to_string(v).unwrap_or_default(),
                    }
                }
                fn eval_jq_filter(input: &Value, filter: &str) -> Result<Value, WebPipeError> {
                    let input_json = serde_json::to_string(input)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Input serialization error: {}", e)))?;
                    let mut program = jq_rs::compile(filter)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ compilation error: {}", e)))?;
                    let result_json = program
                        .run(&input_json)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ execution error: {}", e)))?;
                    serde_json::from_str(&result_json)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ result parse error: {}", e)))
                }

                // Handle comparisons
                match (field, cmp) {
                    ("status", "is") | ("status", "equals") => {
                        if let Ok(expected) = val_str.parse::<u16>() {
                            if status != expected { 
                                cond_pass = false; 
                                failure_msgs.push(format!("expected status {} got {}", expected, status)); 
                            }
                        } else { cond_pass = false; failure_msgs.push(format!("invalid expected status: {}", val_str)); }
                    }
                    ("status", "in") => {
                        // parse N..M
                        let parts: Vec<&str> = val_str.split("..").collect();
                        if parts.len() == 2 {
                            if let (Ok(start), Ok(end)) = (parts[0].trim().parse::<u16>(), parts[1].trim().parse::<u16>()) {
                                if status < start || status > end {
                                    cond_pass = false; failure_msgs.push(format!("expected status in {}..{} got {}", start, end, status));
                                }
                            } else { cond_pass = false; failure_msgs.push(format!("invalid status range: {}", val_str)); }
                        } else { cond_pass = false; failure_msgs.push(format!("invalid status range: {}", val_str)); }
                    }
                    ("contentType", "is") | ("contentType", "equals") => {
                        let expected = val_str.trim_matches('"');
                        if content_type != expected { cond_pass = false; failure_msgs.push(format!("expected contentType {} got {}", expected, content_type)); }
                    }
                    ("output", op) if ["equals","contains","matches"].contains(&op) => {
                        // Possibly apply jq
                        let mut target = output_value.clone();
                        if let Some(expr) = &cond.jq_expr {
                            // Only if JSON
                            target = eval_jq_filter(&target, expr)?;
                        }
                        match op {
                            "equals" => {
                                let expected = parse_backticked_json(val_str)?;
                                if target != expected { 
                                    cond_pass = false; 
                                    if verbose {
                                        failure_msgs.push(format!("output mismatch\nExpected: {}\nActual: {}", 
                                            serde_json::to_string_pretty(&expected).unwrap_or_else(|_| "<invalid JSON>".to_string()),
                                            serde_json::to_string_pretty(&target).unwrap_or_else(|_| "<invalid JSON>".to_string()
                                        )));
                                    } else {
                                        failure_msgs.push("output mismatch".to_string());
                                    }
                                }
                            }
                            "contains" => {
                                let expected = parse_backticked_json(val_str)?;
                                if !deep_contains(&target, &expected) { 
                                    cond_pass = false; 
                                    if verbose {
                                        failure_msgs.push(format!("output missing expected fragment\nExpected to contain: {}\nActual output: {}", 
                                            serde_json::to_string_pretty(&expected).unwrap_or_else(|_| "<invalid JSON>".to_string()),
                                            serde_json::to_string_pretty(&target).unwrap_or_else(|_| "<invalid JSON>".to_string()
                                        )));
                                    } else {
                                        failure_msgs.push("output missing expected fragment".to_string());
                                    }
                                }
                            }
                            "matches" => {
                                let expected = parse_backticked_json(val_str)?;
                                let pattern = if let Value::String(s) = expected { s } else { value_to_string(&expected) };
                                let re = Regex::new(&pattern).map_err(|e| WebPipeError::BadRequest(format!("invalid regex: {}", e)))?;
                                let hay = value_to_string(&target);
                                if !re.is_match(&hay) { cond_pass = false; failure_msgs.push("output does not match regex".to_string()); }
                            }
                            _ => {}
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


