use crate::ast::{Mock, Pipeline, PipelineRef, PipelineStep, Program, Variable, When, DomAssertType};
use crate::error::WebPipeError;
use crate::middleware::MiddlewareRegistry;
use crate::executor::{ExecutionEnv, MiddlewareInvoker};
use crate::runtime::Context;
use crate::config;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use crate::http::request::build_request_for_tests;
use regex::Regex;
use scraper::{Html, Selector};
use handlebars::Handlebars;

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

    fn from_mocks(mocks: &[Mock], variables_ctx: &serde_json::Map<String, Value>) -> Result<Self, WebPipeError> {
        let mut resolver = Self::empty();
        for m in mocks {
            let value = evaluate_jq_input(&m.return_value, variables_ctx)?;
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

    /// Get mock by key directly (for GraphQL query.users, mutation.createTodo etc.)
    fn get_mock_by_key(&self, key: &str) -> Option<&Value> {
        // Check if it's a pipeline mock (pipeline.name)
        if let Some((prefix, name)) = key.split_once('.') {
            if prefix == "pipeline" {
                return self.pipeline_mocks.get(name);
            }
            // Check if it's a variable mock (middleware.varname)
            if let Some(v) = self.variable_mocks.get(&(prefix.to_string(), name.to_string())) {
                return Some(v);
            }
        }
        // Check middleware-level mock
        self.middleware_mocks.get(key)
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

    fn get_mock(&self, target: &str) -> Option<Value> {
        // Check if we have a mock for "query.users", "mutation.createTodo" etc.
        self.mocks.get_mock_by_key(target).cloned()
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
        let empty_ctx = serde_json::Map::new();
        let res = MockResolver::from_mocks(&mocks, &empty_ctx).unwrap();
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

/// Evaluates a JQ expression using the let variables as JQ variables ($name syntax)
fn evaluate_jq_input(expr: &str, variables_ctx: &serde_json::Map<String, Value>) -> Result<Value, WebPipeError> {
    let trimmed = expr.trim();
    let content = if let (Some(start), Some(end)) = (trimmed.find('`'), trimmed.rfind('`')) {
        if end > start { &trimmed[start + 1..end] } else { trimmed }
    } else {
        trimmed
    };

    // Create input JSON from variables context
    let input = Value::Object(variables_ctx.clone());
    let input_json = serde_json::to_string(&input)
        .map_err(|e| WebPipeError::BadRequest(format!("Variable context serialization error: {}", e)))?;

    // Build JQ program that binds each variable as a JQ variable ($varname)
    // Example: .name as $name | .count as $count | <user_expression>
    let mut jq_program = String::new();
    for (var_name, _) in variables_ctx.iter() {
        jq_program.push_str(&format!(".{} as ${} | ", var_name, var_name));
    }
    jq_program.push_str(content);

    // Compile and run JQ expression
    let mut program = jq_rs::compile(&jq_program)
        .map_err(|e| WebPipeError::BadRequest(format!("JQ compilation error in test input: {}", e)))?;
    let result_json = program
        .run(&input_json)
        .map_err(|e| WebPipeError::BadRequest(format!("JQ execution error in test input: {}", e)))?;

    // Parse result
    serde_json::from_str(&result_json)
        .map_err(|e| WebPipeError::BadRequest(format!("JQ result parse error: {}", e)))
}



// Removed local helper functions in favor of shared executor

fn find_variable<'a>(variables: &'a [Variable], var_type: &str, name: &str) -> Option<&'a Variable> {
    variables.iter().find(|v| v.var_type == var_type && v.name == name)
}

pub async fn run_tests(program: Program, verbose: bool) -> Result<TestSummary, WebPipeError> {
    // Initialize global config (Context builder will also set globals and register partials)
    config::init_global(program.configs.clone());

    let mut ctx = Context::from_program_configs(program.configs.clone(), &program.variables).await?;

    // Initialize GraphQL runtime if schema is defined
    if program.graphql_schema.is_some() || !program.queries.is_empty() || !program.mutations.is_empty() {
        let graphql_runtime = crate::graphql::GraphQLRuntime::from_program(&program)
            .map_err(|e| WebPipeError::ConfigError(e.to_string()))?;
        ctx.graphql = Some(Arc::new(graphql_runtime));
    }

    let registry: Arc<MiddlewareRegistry> = Arc::new(MiddlewareRegistry::with_builtins(std::sync::Arc::new(ctx)));

    let mut outcomes: Vec<TestOutcome> = Vec::new();

    for describe in &program.describes {
        // Describe-level mocks use empty context (no test variables available yet)
        let empty_ctx = serde_json::Map::new();
        let describe_mocks = MockResolver::from_mocks(&describe.mocks, &empty_ctx)?;
        for test in &describe.tests {
            // Process let variables into Handlebars context
            let mut variables_ctx = serde_json::Map::new();
            for (name, raw_val) in &test.variables {
                // Try parsing as JSON (handles numbers, bools, quoted strings, objects)
                // If it fails (e.g. bare word), treat as string
                let val = match serde_json::from_str::<Value>(raw_val) {
                    Ok(v) => v,
                    Err(_) => Value::String(raw_val.clone()),
                };
                variables_ctx.insert(name.clone(), val);
            }

            // Helper to render strings with variables
            let hb = Handlebars::new();
            let render = |s: &str| -> Result<String, WebPipeError> {
                hb.render_template(s, &variables_ctx)
                    .map_err(|e| WebPipeError::BadRequest(format!("Template substitution failed: {}", e)))
            };

            let test_mocks = MockResolver::from_mocks(&test.mocks, &variables_ctx)?;
            // Test-level mocks override describe-level mocks (overlay describe <- test)
            let mocks = describe_mocks.overlay(&test_mocks);

            let (status, output_value, content_type, pass, msg, exec_call_log) = match &test.when {
                When::CallingRoute { method, path } => {
                    // Render variables in path (e.g. /teams/{{id}})
                    let path_rendered = render(path)?;

                    // Split query string if present
                    let mut path_str = path_rendered;
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

                            // Evaluate JQ expressions for test body, headers, and cookies using let variables
                            let test_body = if let Some(body_str) = &test.body {
                                evaluate_jq_input(body_str, &variables_ctx)?
                            } else {
                                Value::Object(serde_json::Map::new())
                            };

                            let test_headers = if let Some(h_str) = &test.headers {
                                let parsed = evaluate_jq_input(h_str, &variables_ctx)?;
                                if let Value::Object(map) = parsed {
                                    Some(map.into_iter().map(|(k, v)| (k, v.as_str().unwrap_or("").to_string())).collect())
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            let test_cookies = if let Some(c_str) = &test.cookies {
                                let parsed = evaluate_jq_input(c_str, &variables_ctx)?;
                                if let Value::Object(map) = parsed {
                                    Some(map.into_iter().map(|(k, v)| (k, v.as_str().unwrap_or("").to_string())).collect())
                                } else {
                                    None
                                }
                            } else {
                                None
                            };

                            // Build request JSON via shared helper
                            let input = build_request_for_tests(
                                method,
                                &path_str,
                                &params_map,
                                &query_map,
                                test_headers.as_ref(),
                                test_cookies.as_ref(),
                                test_body
                            );

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
                                registry: registry.clone(),
                                environment: None,


                                cache: crate::runtime::context::CacheStore::new(8, 60),
                    rate_limit: crate::runtime::context::RateLimitStore::new(1000),
                                
                            };

                            // Pipeline-level mock when route uses a named pipeline
                            if let Some(name) = selected_pipeline_name {
                                if let Some(mock) = mocks.get_pipeline_mock(name) {
                                    (200u16, mock.clone(), "application/json".to_string(), true, String::new(), HashMap::new())
                                } else {
                                    let ctx = crate::executor::RequestContext::new();
                                    let (out, ct, s, ctx) = crate::executor::execute_pipeline(&env, selected_pipeline, input, ctx).await?;
                                    let log = ctx.call_log.clone();
                                    (s.unwrap_or(200u16), out, ct, true, String::new(), log)
                                }
                            } else {
                                let ctx = crate::executor::RequestContext::new();
                                let (out, ct, s, ctx) = crate::executor::execute_pipeline(&env, selected_pipeline, input, ctx).await?;
                                let log = ctx.call_log.clone();
                                (s.unwrap_or(200u16), out, ct, true, String::new(), log)
                            }
                        }
                        Err(_) => (404u16, Value::Object(serde_json::Map::new()), "application/json".to_string(), false, format!("Route not found: {} {}", method, path), HashMap::new()),
                    }
                }
                When::ExecutingPipeline { name } => {
                    // If pipeline mock exists, return it directly
                    if let Some(mock) = mocks.get_pipeline_mock(name) {
                        (200u16, mock.clone(), "application/json".to_string(), true, String::new(), HashMap::new())
                    } else {
                        // Evaluate JQ input expression with test variables
                        let input = if let Some(input_str) = &test.input {
                            evaluate_jq_input(input_str, &variables_ctx)?
                        } else {
                            Value::Object(serde_json::Map::new())
                        };
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
                            registry: registry.clone(),
                            environment: None,


                            cache: crate::runtime::context::CacheStore::new(8, 60),
                    rate_limit: crate::runtime::context::RateLimitStore::new(1000),

                        };
                        let ctx = crate::executor::RequestContext::new();
                        let (out, ct, status_opt, ctx) = crate::executor::execute_pipeline(&env, &pipeline.pipeline, input, ctx).await?;
                        let log = ctx.call_log.clone();
                        (status_opt.unwrap_or(200u16), out, ct, true, String::new(), log)
                    }
                }
                When::ExecutingVariable { var_type, name } => {
                    let var = find_variable(&program.variables, var_type, name)
                        .ok_or_else(|| WebPipeError::BadRequest(format!("Variable not found: {} {}", var_type, name)))?;
                    // Evaluate JQ input expression with test variables
                    let input = if let Some(input_str) = &test.input {
                        evaluate_jq_input(input_str, &variables_ctx)?
                    } else {
                        Value::Object(serde_json::Map::new())
                    };
                    // If there's an explicit variable-level mock, return it directly
                    if let Some(mock) = mocks.variable_mocks.get(&(var.var_type.clone(), var.name.clone())) {
                        (200u16, mock.clone(), "application/json".to_string(), true, String::new(), HashMap::new())
                    } else {
                        // Single-step pipeline invoking the variable's middleware
                        let pipeline = Pipeline { steps: vec![PipelineStep::Regular { name: var.var_type.clone(), config: var.value.clone(), config_type: crate::ast::ConfigType::Backtick, condition: None, parsed_join_targets: None }] };
                        let named: HashMap<String, Arc<Pipeline>> = program
                            .pipelines
                            .iter()
                            .map(|p| (p.name.clone(), Arc::new(p.pipeline.clone())))
                            .collect();
                        let env = ExecutionEnv {
                            variables: Arc::new(program.variables.clone()),
                            named_pipelines: Arc::new(named),
                            invoker: Arc::new(MockingInvoker { registry: registry.clone(), mocks: mocks.clone() }),
                            registry: registry.clone(),
                            environment: None,


                            cache: crate::runtime::context::CacheStore::new(8, 60),
                    rate_limit: crate::runtime::context::RateLimitStore::new(1000),

                        };
                        let ctx = crate::executor::RequestContext::new();
                        let (out, ct, status_opt, ctx) = crate::executor::execute_pipeline(&env, &pipeline, input, ctx).await?;
                        let log = ctx.call_log.clone();
                        (status_opt.unwrap_or(200u16), out, ct, true, String::new(), log)
                    }
                }
            };

            // Evaluate conditions
            let mut cond_pass = pass;
            let mut failure_msgs: Vec<String> = Vec::new();

            for cond in &test.conditions {
                // Handle call assertions separately
                if cond.is_call_assertion {
                    if let Some(target) = &cond.call_target {
                        // Render variable substitutions in condition value
                        let val_str_rendered = render(&cond.value)?;
                        let expected_args = parse_backticked_json(&val_str_rendered)?;

                        // Check call log (exec_call_log from execution)
                        if let Some(calls) = exec_call_log.get(target) {
                            // Check if ANY call matches the expected arguments
                            let match_found = calls.iter().any(|call_args| {
                                deep_equals(call_args, &expected_args)
                            });

                            if !match_found {
                                cond_pass = false;
                                failure_msgs.push(format!(
                                    "Expected {} to be called with {:?}, but no matching call found",
                                    target, expected_args
                                ));
                            }
                        } else {
                            cond_pass = false;
                            failure_msgs.push(format!("{} was never called", target));
                        }
                    }
                    continue; // Skip to next condition
                }

                let field = cond.field.trim();
                let cmp = cond.comparison.trim();
                // Render variable substitutions in condition value
                let val_str_rendered = render(&cond.value)?;
                let val_str = val_str_rendered.trim();

                // Helpers
                fn deep_equals(actual: &Value, expected: &Value) -> bool {
                    // For call assertions, we use deep_contains logic
                    // This allows partial matching (expected args must be present in actual)
                    deep_contains(actual, expected)
                }
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
                    ("header", op) if ["contains", "equals", "matches"].contains(&op) => {
                        // Header assertions
                        if let Some(header_name) = &cond.header_name {
                            // Extract header value from setCookies or other header mechanisms
                            // For now, we primarily support setCookies which appears in the output JSON
                            let header_value = if header_name.to_lowercase() == "set-cookie" {
                                // Extract from setCookies array in output
                                if let Some(cookies) = output_value.get("setCookies").and_then(|v| v.as_array()) {
                                    // Join all cookies with "; " for matching
                                    Some(cookies.iter()
                                        .filter_map(|c| c.as_str())
                                        .collect::<Vec<_>>()
                                        .join("; "))
                                } else {
                                    None
                                }
                            } else {
                                // For other headers, check if they're in the output JSON
                                output_value.get(header_name).and_then(|v| v.as_str()).map(|s| s.to_string())
                            };

                            match op {
                                "contains" => {
                                    let expected = parse_backticked_json(val_str)?;
                                    let expected_str = value_to_string(&expected);
                                    if let Some(actual) = header_value {
                                        if !actual.contains(&expected_str) {
                                            cond_pass = false;
                                            failure_msgs.push(format!("header '{}' does not contain '{}'", header_name, expected_str));
                                        }
                                    } else {
                                        cond_pass = false;
                                        failure_msgs.push(format!("header '{}' not found", header_name));
                                    }
                                }
                                "equals" => {
                                    let expected = parse_backticked_json(val_str)?;
                                    let expected_str = value_to_string(&expected);
                                    if let Some(actual) = header_value {
                                        if actual != expected_str {
                                            cond_pass = false;
                                            failure_msgs.push(format!("header '{}' expected '{}' got '{}'", header_name, expected_str, actual));
                                        }
                                    } else {
                                        cond_pass = false;
                                        failure_msgs.push(format!("header '{}' not found", header_name));
                                    }
                                }
                                "matches" => {
                                    let expected = parse_backticked_json(val_str)?;
                                    let pattern = value_to_string(&expected);
                                    let re = Regex::new(&pattern).map_err(|e| WebPipeError::BadRequest(format!("invalid regex: {}", e)))?;
                                    if let Some(actual) = header_value {
                                        if !re.is_match(&actual) {
                                            cond_pass = false;
                                            failure_msgs.push(format!("header '{}' does not match pattern '{}'", header_name, pattern));
                                        }
                                    } else {
                                        cond_pass = false;
                                        failure_msgs.push(format!("header '{}' not found", header_name));
                                    }
                                }
                                _ => {}
                            }
                        } else {
                            cond_pass = false;
                            failure_msgs.push("header assertion requires header name".to_string());
                        }
                    }
                    _ => {
                        // Check for DOM selector assertions
                        if field == "selector" && cond.selector.is_some() {
                            // Render variable substitutions in selector
                            let selector_str = render(cond.selector.as_ref().unwrap())?;
                            let dom_assert = cond.dom_assert.as_ref().unwrap();

                            // Convert output to HTML string
                            let html_str = match &output_value {
                                Value::String(s) => s.clone(),
                                _ => {
                                    cond_pass = false;
                                    failure_msgs.push(format!(
                                        "Selector assertion requires HTML string output, got: {}",
                                        output_value
                                    ));
                                    continue;
                                }
                            };

                            // Parse HTML
                            let document = Html::parse_document(&html_str);

                            // Parse CSS selector
                            let selector = match Selector::parse(&selector_str) {
                                Ok(s) => s,
                                Err(e) => {
                                    cond_pass = false;
                                    failure_msgs.push(format!("Invalid CSS selector '{}': {:?}", selector_str, e));
                                    continue;
                                }
                            };

                            // Evaluate based on assertion type
                            match dom_assert {
                                DomAssertType::Exists => {
                                    let exists = document.select(&selector).next().is_some();
                                    let expected = cmp == "exists";
                                    if exists != expected {
                                        cond_pass = false;
                                        failure_msgs.push(format!(
                                            "Selector '{}' {} (expected: {})",
                                            selector_str,
                                            if exists { "exists" } else { "does not exist" },
                                            if expected { "exists" } else { "does not exist" }
                                        ));
                                    }
                                }

                                DomAssertType::Count => {
                                    let count = document.select(&selector).count();
                                    let expected: usize = val_str.trim().parse().unwrap_or(0);

                                    let matches = match cmp {
                                        "equals" | "is" => count == expected,
                                        "is greater than" | "greater than" => count > expected,
                                        "is less than" | "less than" => count < expected,
                                        _ => false,
                                    };

                                    if !matches {
                                        cond_pass = false;
                                        failure_msgs.push(format!(
                                            "Selector '{}' count is {} (expected {} {})",
                                            selector_str, count, cmp, expected
                                        ));
                                    }
                                }

                                DomAssertType::Text => {
                                    let element = document.select(&selector).next();

                                    match element {
                                        Some(el) => {
                                            let text: String = el.text().collect::<Vec<_>>().join("");
                                            let text = text.trim();

                                            let matches = match cmp {
                                                "equals" | "is" => text == val_str,
                                                "contains" => text.contains(val_str),
                                                "matches" => {
                                                    match Regex::new(val_str) {
                                                        Ok(re) => re.is_match(text),
                                                        Err(_) => false,
                                                    }
                                                }
                                                _ => false,
                                            };

                                            if !matches {
                                                cond_pass = false;
                                                failure_msgs.push(format!(
                                                    "Selector '{}' text '{}' does not {} '{}'",
                                                    selector_str, text, cmp, val_str
                                                ));
                                            }
                                        }
                                        None => {
                                            cond_pass = false;
                                            failure_msgs.push(format!(
                                                "Selector '{}' not found in HTML",
                                                selector_str
                                            ));
                                        }
                                    }
                                }

                                DomAssertType::Attribute(attr_name) => {
                                    let element = document.select(&selector).next();

                                    match element {
                                        Some(el) => {
                                            match el.value().attr(attr_name) {
                                                Some(attr_value) => {
                                                    let (matches, is_negated) = match cmp {
                                                        "equals" | "is" => (attr_value == val_str, false),
                                                        "contains" => (attr_value.contains(val_str), false),
                                                        "matches" => {
                                                            let m = match Regex::new(val_str) {
                                                                Ok(re) => re.is_match(attr_value),
                                                                Err(_) => false,
                                                            };
                                                            (m, false)
                                                        }
                                                        "does not equal" => (attr_value == val_str, true),
                                                        "does not contain" => (attr_value.contains(val_str), true),
                                                        "does not match" => {
                                                            let m = match Regex::new(val_str) {
                                                                Ok(re) => re.is_match(attr_value),
                                                                Err(_) => false,
                                                            };
                                                            (m, true)
                                                        }
                                                        _ => (false, false),
                                                    };

                                                    let should_fail = if is_negated { matches } else { !matches };

                                                    if should_fail {
                                                        cond_pass = false;
                                                        let op_desc = if is_negated {
                                                            if matches { "matches" } else { "doesn't match" }
                                                        } else {
                                                            "doesn't match"
                                                        };
                                                        failure_msgs.push(format!(
                                                            "Selector '{}' attribute '{}' = '{}' {} expected condition '{} {}'",
                                                            selector_str, attr_name, attr_value, op_desc, cmp, val_str
                                                        ));
                                                    }
                                                }
                                                None => {
                                                    cond_pass = false;
                                                    failure_msgs.push(format!(
                                                        "Selector '{}' found but attribute '{}' not present",
                                                        selector_str, attr_name
                                                    ));
                                                }
                                            }
                                        }
                                        None => {
                                            cond_pass = false;
                                            failure_msgs.push(format!(
                                                "Selector '{}' not found in HTML",
                                                selector_str
                                            ));
                                        }
                                    }
                                }
                            }
                        } else {
                            // Unsupported condition -> mark failure to surface quickly
                            cond_pass = false;
                            failure_msgs.push(format!("unsupported condition: {} {} {}", field, cmp, val_str));
                        }
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


