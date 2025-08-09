use std::collections::HashMap;
use std::sync::Arc;

use axum::{routing::get, Json, Router};

use webpipe::ast::{parse_program, Pipeline};
use webpipe::executor::{execute_pipeline, ExecutionEnv, RealInvoker};
use webpipe::middleware::MiddlewareRegistry;
use webpipe::runtime::Context;

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
    let ctx = Context::from_program_configs(program.configs.clone(), &program.variables)
        .await
        .expect("context");
    let registry = Arc::new(MiddlewareRegistry::with_builtins(Arc::new(ctx)));

    let named: HashMap<String, Arc<Pipeline>> = program
        .pipelines
        .iter()
        .map(|p| (p.name.clone(), Arc::new(p.pipeline.clone())))
        .collect();

    ExecutionEnv {
        variables: Arc::new(program.variables.clone()),
        named_pipelines: Arc::new(named),
        invoker: Arc::new(RealInvoker::new(registry)),
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
    let (_out, _ct, status) = execute_pipeline(&env, &pipeline, input).await.unwrap();
    assert_eq!(status, Some(400));

    // Ok path
    let input = serde_json::json!({"body": {"name": "Al"}});
    let (_out, _ct, status) = execute_pipeline(&env, &pipeline, input).await.unwrap();
    assert_eq!(status, Some(200));
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
    let (out1, _ct, _st) = execute_pipeline(&env, &pipeline, input.clone()).await.unwrap();
    assert!(out1["data"]["e"]["response"]["ok"].as_bool().unwrap());

    // Second call should return quickly and use cache (behavioral equivalence)
    let (out2, _ct, _st) = execute_pipeline(&env, &pipeline, input).await.unwrap();
    assert_eq!(out1["data"]["e"], out2["data"]["e"]);

    // Stop server
    drop(server);
}


