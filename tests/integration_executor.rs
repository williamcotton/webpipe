use std::collections::HashMap;
use std::sync::Arc;

use axum::{routing::get, Json, Router};

use webpipe::ast::{parse_program, Pipeline};
use webpipe::executor::{execute_pipeline, ExecutionEnv, ModuleRegistry, RealInvoker};
use webpipe::middleware::{AssertMiddleware, JqMiddleware, MiddlewareRegistry};
use webpipe::runtime::context::{CacheStore, RateLimitStore};
use webpipe::runtime::Context;
use webpipe::ast::Variable;

fn find_pipeline_owned(program: &webpipe::ast::Program, name: &str) -> Pipeline {
    program
        .pipelines
        .iter()
        .find(|p| p.name == name)
        .expect("pipeline not found")
        .pipeline
        .clone()
}

async fn build_env(program: &webpipe::ast::Program) -> ExecutionEnv {
    let ctx = Arc::new(Context::from_program_configs(program.configs.clone(), &program.variables)
        .await
        .expect("context"));
    let registry = Arc::new(MiddlewareRegistry::with_builtins(ctx.clone()));

    let named: HashMap<(Option<usize>, String), Arc<Pipeline>> = program
        .pipelines
        .iter()
        .map(|p| ((None, p.name.clone()), Arc::new(p.pipeline.clone())))
        .collect();

    // Convert variables to HashMap
    let variables_map: HashMap<(Option<usize>, String, String), Variable> = program.variables
        .iter()
        .map(|v| ((None, v.var_type.clone(), v.name.clone()), v.clone()))
        .collect();

    ExecutionEnv {
        variables: Arc::new(variables_map),
        named_pipelines: Arc::new(named),
            imports: Arc::new(std::collections::HashMap::new()),
        invoker: Arc::new(RealInvoker::new(registry.clone())),
        registry: registry.clone(),
        environment: None,
        cache: ctx.cache.clone(),
        rate_limit: ctx.rate_limit.clone(),
        module_registry: Arc::new(ModuleRegistry::new()),
        debugger: None,
    }
}

fn build_assert_env(program: &webpipe::ast::Program) -> ExecutionEnv {
    let mut registry = MiddlewareRegistry::empty();
    registry.register("assert", Box::new(AssertMiddleware));
    registry.register("jq", Box::new(JqMiddleware::new()));
    let registry = Arc::new(registry);

    let named: HashMap<(Option<usize>, String), Arc<Pipeline>> = program
        .pipelines
        .iter()
        .map(|p| ((None, p.name.clone()), Arc::new(p.pipeline.clone())))
        .collect();

    let variables_map: HashMap<(Option<usize>, String, String), Variable> = program.variables
        .iter()
        .map(|v| ((None, v.var_type.clone(), v.name.clone()), v.clone()))
        .collect();

    ExecutionEnv {
        variables: Arc::new(variables_map),
        named_pipelines: Arc::new(named),
        imports: Arc::new(std::collections::HashMap::new()),
        invoker: Arc::new(RealInvoker::new(registry.clone())),
        registry,
        environment: None,
        cache: CacheStore::new(8, 60),
        rate_limit: RateLimitStore::new(1000),
        module_registry: Arc::new(ModuleRegistry::new()),
        debugger: None,
    }
}

#[tokio::test]
async fn validate_jq_result_branching() {
    let src = r#"
pipeline p =
  |> validate: `{ name: string(2..4) }`
  |> result
    validationError(400):
      |> jq: `.`
    ok(200):
      |> jq: `.`
"#;
    let (_rest, program) = parse_program(src).unwrap();
    let env = build_env(&program).await;
    let pipeline = find_pipeline_owned(&program, "p");

    // Error path
    let input = serde_json::json!({"body": {}});
    let (_out, _ct, status, _ctx) = execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(status, Some(400));

    // Ok path
    let input = serde_json::json!({"body": {"name": "Al"}});
    let (_out, _ct, status, _ctx) = execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(status, Some(200));
}

#[tokio::test]
async fn named_assert_contract_refines_without_leaking_result_name() {
    let src = r#"
assert PersonState = `{
  id: string | number,
  name: string,
  email?: email
}`

pipeline p =
  |> assert: PersonState
"#;
    let (_rest, program) = parse_program(src).unwrap();
    let env = build_assert_env(&program);
    let pipeline = find_pipeline_owned(&program, "p");

    let input = serde_json::json!({
        "id": "123",
        "name": "Ada"
    });
    let (out, _ct, _status, _ctx) = execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();

    assert_eq!(out["name"], serde_json::json!("Ada"));
    assert!(out.get("resultName").is_none());
    assert!(out.get("errors").is_none());
}

#[tokio::test]
async fn failed_assert_selects_assertion_error_result_branch() {
    let src = r#"
assert PersonState = `{
  id: string | number,
  name: string
}`

pipeline p =
  |> assert: PersonState
  |> result
    assertionError(400):
      |> jq: `{ field: .errors[0].field, message: .errors[0].message }`
    ok(200):
      |> jq: `{ ok: true }`
"#;
    let (_rest, program) = parse_program(src).unwrap();
    let env = build_assert_env(&program);
    let pipeline = find_pipeline_owned(&program, "p");

    let input = serde_json::json!({ "id": "123" });
    let (out, _ct, status, _ctx) = execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();

    assert_eq!(status, Some(400));
    assert_eq!(out["field"], serde_json::json!("name"));
    assert!(out["message"].as_str().unwrap().contains("Missing required field"));
}

#[tokio::test]
async fn fetch_with_local_server_and_cache() {
    // Start a tiny local server
    let app = Router::new().route(
        "/echo",
        get(|| async { Json(serde_json::json!({"ok": true})) }),
    );
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let url = format!("http://{}/echo", addr);
    let src = format!(
        r#"
pipeline getEcho =
  |> cache: `enabled: true, ttl: 60, keyTemplate: echo`
  |> fetch: "{}"
"#,
        url
    );
    let (_rest, program) = parse_program(&src).unwrap();
    let env = build_env(&program).await;
    let pipeline = find_pipeline_owned(&program, "getEcho");

    // First call hits network
    let input = serde_json::json!({"resultName": "e"});
    let (out1, _ct, _st, ctx1) = execute_pipeline(&env, &pipeline, input.clone(), webpipe::executor::RequestContext::new()).await.unwrap();
    assert!(out1["data"]["e"]["response"]["ok"].as_bool().unwrap());

    // CRITICAL: Run deferred actions to populate the cache
    ctx1.run_deferred(&out1, "application/json", &env);

    // Second call should return quickly and use cache (behavioral equivalence)
    let (out2, _ct, _st, _ctx2) = execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(out1["data"]["e"], out2["data"]["e"]);

    // Stop server
    drop(server);
}

#[tokio::test]
async fn cache_with_nested_pipeline_and_lua() {
    let src = r#"
pipeline fetchData =
  |> jq: `{rawData: "fetched content"}`

pipeline processData =
  |> cache: `enabled: true, ttl: 60, keyTemplate: test-nested`
  |> pipeline: "fetchData"
  |> lua: "return {articles = {request.rawData}}"
"#;
    let (_rest, program) = parse_program(src).unwrap();
    let pipeline = find_pipeline_owned(&program, "processData");

    // Build base env once (simulates ServerState.env in production)
    let base_env = build_env(&program).await;

    // First request uses shared env but separate RequestContext
    let input = serde_json::json!({});
    let (out1, _ct, _st, ctx1) = execute_pipeline(&base_env, &pipeline, input.clone(), webpipe::executor::RequestContext::new()).await.unwrap();
    println!("First call result: {}", serde_json::to_string_pretty(&out1).unwrap());

    // Run deferred actions to save result to cache (simulates server.rs line 433)
    ctx1.run_deferred(&out1, "application/json", &base_env);

    // Verify the result has the processed articles field
    assert_eq!(out1["articles"][0], serde_json::json!("fetched content"));

    // Second request with SHARED env/cache but NEW RequestContext
    let (out2, _ct, _st, _ctx2) = execute_pipeline(&base_env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();
    println!("Second call result: {}", serde_json::to_string_pretty(&out2).unwrap());

    // The cached result should match the first result exactly
    assert_eq!(out1, out2);
    assert_eq!(out2["articles"][0], serde_json::json!("fetched content"));
}
