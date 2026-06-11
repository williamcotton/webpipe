use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

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
    let hit_count = Arc::new(AtomicUsize::new(0));
    let route_hit_count = hit_count.clone();
    let app = Router::new().route(
        "/echo",
        get(move || {
            let route_hit_count = route_hit_count.clone();
            async move {
                let count = route_hit_count.fetch_add(1, Ordering::SeqCst) + 1;
                Json(serde_json::json!({"ok": true, "count": count}))
            }
        }),
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
    let (out1, _ct, _st, _ctx1) = execute_pipeline(&env, &pipeline, input.clone(), webpipe::executor::RequestContext::new()).await.unwrap();
    assert!(out1["data"]["e"]["response"]["ok"].as_bool().unwrap());
    assert_eq!(hit_count.load(Ordering::SeqCst), 1);

    // Second call should return from cache without hitting the server again.
    let (out2, _ct, _st, _ctx2) = execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(out1["data"]["e"], out2["data"]["e"]);
    assert_eq!(hit_count.load(Ordering::SeqCst), 1);

    // Stop server
    server.abort();
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

#[tokio::test]
async fn cache_replays_status_code_for_rendered_error_pages() {
    let src = r#"
pipeline p =
  |> cache: `enabled: true, ttl: 60, keyTemplate: status-replay`
  |> jq: `{ errors: [{ type: "notFound", message: "nope" }] }`
  |> result
    notFound(404):
      |> jq: `"rendered 404 page"`
    ok(200):
      |> jq: `"ok"`
"#;
    let (_rest, program) = parse_program(src).unwrap();
    let env = build_env(&program).await;
    let pipeline = find_pipeline_owned(&program, "p");

    let input = serde_json::json!({});
    let (out1, _ct, st1, _) = execute_pipeline(&env, &pipeline, input.clone(), webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(st1, Some(404));
    assert_eq!(out1, serde_json::json!("rendered 404 page"));

    // Second run must come from cache with the stored status code
    let (out2, _ct, st2, _) = execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(st2, Some(404), "cached response must replay its status code");
    assert_eq!(out2, serde_json::json!("rendered 404 page"));
}

#[tokio::test]
async fn cache_never_stores_5xx_responses() {
    let src = r#"
pipeline p =
  |> cache: `enabled: true, ttl: 60, keyTemplate: err5xx`
  |> jq: `{ errors: [{ type: "boom", message: "transient" }] }`
  |> result
    boom(500):
      |> jq: `"error page"`
    ok(200):
      |> jq: `"ok"`
"#;
    let (_rest, program) = parse_program(src).unwrap();
    let env = build_env(&program).await;
    let pipeline = find_pipeline_owned(&program, "p");

    let input = serde_json::json!({});
    let (_out, _ct, st, _) = execute_pipeline(&env, &pipeline, input.clone(), webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(st, Some(500));
    assert!(
        matches!(env.cache.lookup("err5xx"), webpipe::runtime::CacheLookup::Miss),
        "5xx responses must not be cached"
    );

    // The pipeline re-executes on every request while the upstream is failing
    let (_out, _ct, st, _) = execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(st, Some(500));
}

#[tokio::test]
async fn route_and_nested_cache_steps_with_identical_configs_do_not_collide() {
    // Regression: a route-level cache step and a nested pipeline cache step
    // with byte-identical configs used to hash to the same default key; the
    // route then served the inner pipeline's JSON instead of its own output.
    let src = r#"
pipeline inner =
  |> cache: `enabled: true, ttl: 60`
  |> jq: `{ data: "inner-data" }`

pipeline route =
  |> cache: `enabled: true, ttl: 60`
  |> pipeline: "inner"
  |> jq: `"<html>" + .data + "</html>"`
"#;
    let (_rest, program) = parse_program(src).unwrap();
    let env = build_env(&program).await;
    let pipeline = find_pipeline_owned(&program, "route");

    // Request-shaped input: method/path/query/params persist through the
    // nested pipeline via backpack merge, which is what made the keys collide
    let input = serde_json::json!({"method": "GET", "path": "/", "query": {}, "params": {}});
    let (out1, _ct, _st, _) = execute_pipeline(&env, &pipeline, input.clone(), webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(out1, serde_json::json!("<html>inner-data</html>"));

    // Second run hits the route-level cache; it must serve the route's own
    // output, not the inner pipeline's
    let (out2, _ct, _st, _) = execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(out2, serde_json::json!("<html>inner-data</html>"));
}

#[tokio::test]
async fn stale_while_revalidate_prevents_thundering_herd() {
    let hit_count = Arc::new(AtomicUsize::new(0));
    let route_hit_count = hit_count.clone();
    let app = Router::new().route(
        "/echo",
        get(move || {
            let route_hit_count = route_hit_count.clone();
            async move {
                let count = route_hit_count.fetch_add(1, Ordering::SeqCst) + 1;
                Json(serde_json::json!({"ok": true, "count": count}))
            }
        }),
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
  |> cache: `enabled: true, ttl: 1, keyTemplate: herd`
  |> fetch: "{}"
"#,
        url
    );
    let (_rest, program) = parse_program(&src).unwrap();
    let env = build_env(&program).await;
    let pipeline = find_pipeline_owned(&program, "getEcho");

    // Warm the cache
    let input = serde_json::json!({"resultName": "e"});
    let (out1, _ct, _st, _) = execute_pipeline(&env, &pipeline, input.clone(), webpipe::executor::RequestContext::new()).await.unwrap();
    assert!(out1["data"]["e"]["response"]["ok"].as_bool().unwrap());
    assert_eq!(hit_count.load(Ordering::SeqCst), 1);

    // Let the entry go stale (TTL 1s; default SWR grace keeps it servable)
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    // Hammer with 10 concurrent requests: exactly one is elected to refresh,
    // everyone else is served the stale value
    let env = Arc::new(env);
    let pipeline = Arc::new(pipeline);
    let mut handles = Vec::new();
    for _ in 0..10 {
        let env = env.clone();
        let pipeline = pipeline.clone();
        let input = input.clone();
        handles.push(tokio::spawn(async move {
            execute_pipeline(&env, &pipeline, input, webpipe::executor::RequestContext::new()).await
        }));
    }
    for handle in handles {
        let (out, _ct, _st, _) = handle.await.unwrap().unwrap();
        assert!(out["data"]["e"]["response"]["ok"].as_bool().unwrap());
    }
    assert_eq!(
        hit_count.load(Ordering::SeqCst),
        2,
        "only the elected refresher may hit the upstream (initial warm + one refresh)"
    );

    server.abort();
}
