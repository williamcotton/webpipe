use axum::http::{Method, Request, StatusCode};
use axum::body::Body;
use tower::ServiceExt; // oneshot

use webpipe::ast::parse_program;
use webpipe::server::WebPipeServer;

#[tokio::test]
async fn router_matches_and_static_serving() {
    // Simple program with GET /api that returns HTML via handlebars
    let src = r#"
pipeline html =
  |> handlebars: `Hello`

GET /api
  |> pipeline: html
"#;
    let (_rest, program) = parse_program(src).unwrap();
    let server = WebPipeServer::from_program(program, false).await.unwrap();
    let app = server.router();

    // GET /api -> 200
    let resp = app
        .clone()
        .oneshot(Request::builder().method(Method::GET).uri("/api").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET /health -> 200
    let resp = app
        .clone()
        .oneshot(Request::builder().method(Method::GET).uri("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 404 for unknown route
    let resp = app
        .oneshot(Request::builder().method(Method::GET).uri("/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}


