use crate::ast::{Program, Pipeline, PipelineRef};
use crate::middleware::MiddlewareRegistry;
use crate::error::WebPipeError;
use axum::{
    body::{Bytes, Body},
    extract::{Query, State, OriginalUri},
    http::{Method, StatusCode, HeaderMap},
    response::{IntoResponse, Json, Html, Response},
    routing::{get, on, MethodFilter},
    Router,
};
use serde_json::Value;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::signal;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use tokio::fs as tokio_fs;
use std::path::{Path, PathBuf};
use crate::runtime::Context;
use crate::executor::{ExecutionEnv, RealInvoker, PipelineExecFuture};
use crate::http::request::build_request_from_axum;

// number coercion helpers moved to http::request

// merge helper moved to shared executor

pub struct WebPipeServer {
    program: Program,
    middleware_registry: Arc<MiddlewareRegistry>,
}

#[derive(Clone)]
pub struct ServerState {
    middleware_registry: Arc<MiddlewareRegistry>,
    get_router: Arc<matchit::Router<Arc<Pipeline>>>,
    post_router: Arc<matchit::Router<Arc<Pipeline>>>,
    put_router: Arc<matchit::Router<Arc<Pipeline>>>,
    delete_router: Arc<matchit::Router<Arc<Pipeline>>>,
    env: Arc<ExecutionEnv>,
}

impl ServerState {
    // variable resolution now handled by shared executor

    pub fn execute_pipeline<'a>(
        &'a self,
        pipeline: &'a Pipeline,
        input: Value,
    ) -> PipelineExecFuture<'a> {
        crate::executor::execute_pipeline(&self.env, pipeline, input)
    }

    
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WebPipeRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub body: Value,
    pub content_type: String,
}

impl WebPipeServer {
    pub async fn from_program(program: Program) -> Result<Self, WebPipeError> {
        // Build a Context from program configs (also initializes global config)
        let ctx = Context::from_program_configs(program.configs.clone(), &program.variables).await?;
        let middleware_registry = Arc::new(MiddlewareRegistry::with_builtins(Arc::new(ctx)));

        Ok(Self {
            program,
            middleware_registry,
        })
    }

    pub fn router(self) -> Router {
        let mut router = Router::new().route("/health", get(health_check));

        // Build method-specific matchers
        let mut get_router = matchit::Router::new();
        let mut post_router = matchit::Router::new();
        let mut put_router = matchit::Router::new();
        let mut delete_router = matchit::Router::new();

        for route in &self.program.routes {
            let path = route.path.clone();
            let pipeline = match &route.pipeline {
                PipelineRef::Inline(p) => Arc::new(p.clone()),
                PipelineRef::Named(name) => {
                    let p = self.program
                        .pipelines
                        .iter()
                        .find(|pl| pl.name == *name)
                        .expect("pipeline not found")
                        .pipeline
                        .clone();
                    Arc::new(p)
                }
            };

            match route.method.as_str() {
                "GET" => {
                    let _ = get_router.insert(path.clone(), pipeline.clone());
                }
                "POST" => {
                    let _ = post_router.insert(path.clone(), pipeline.clone());
                }
                "PUT" => {
                    let _ = put_router.insert(path.clone(), pipeline.clone());
                }
                "DELETE" => {
                    let _ = delete_router.insert(path.clone(), pipeline.clone());
                }
                other => {
                    warn!("Unsupported HTTP method: {}", other);
                    continue;
                }
            }
        }

        let named_pipelines: HashMap<String, Arc<Pipeline>> = self.program
            .pipelines
            .iter()
            .map(|p| (p.name.clone(), Arc::new(p.pipeline.clone())))
            .collect();

        let env = ExecutionEnv {
            variables: Arc::new(self.program.variables.clone()),
            named_pipelines: Arc::new(named_pipelines),
            invoker: Arc::new(RealInvoker::new(self.middleware_registry.clone())),
        };

        let server_state = ServerState {
            middleware_registry: self.middleware_registry.clone(),
            get_router: Arc::new(get_router),
            post_router: Arc::new(post_router),
            put_router: Arc::new(put_router),
            delete_router: Arc::new(delete_router),
            env: Arc::new(env),
        };

        // Single catch-all per method
        router = router
            .route("/", on(MethodFilter::GET, unified_handler))
            .route("/*path", on(MethodFilter::GET, unified_handler))
            .route("/*path", on(MethodFilter::POST, unified_handler))
            .route("/*path", on(MethodFilter::PUT, unified_handler))
            .route("/*path", on(MethodFilter::DELETE, unified_handler));

        router
            .layer(
                ServiceBuilder::new()
                    .layer(CorsLayer::permissive())
                    .into_inner(),
            )
            .with_state(server_state)
    }

    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        
        info!("WebPipe server starting on {}", addr);
        
        axum::serve(listener, self.router())
            .with_graceful_shutdown(shutdown_signal())
            .await?;
            
        info!("WebPipe server shutdown complete");
        Ok(())
    }

    pub async fn serve_with_shutdown<Sh>(
        self,
        addr: SocketAddr,
        shutdown: Sh,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        Sh: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = tokio::net::TcpListener::bind(addr).await?;

        info!("WebPipe server starting on {}", addr);

        axum::serve(listener, self.router())
            .with_graceful_shutdown(shutdown)
            .await?;

        info!("WebPipe server shutdown complete");
        Ok(())
    }
}

// configure_pg_from_config removed: middleware reads config directly

async fn unified_handler(
    State(state): State<ServerState>,
    method: Method,
    headers: HeaderMap,
    OriginalUri(orig_uri): OriginalUri,
    Query(query_params): Query<HashMap<String, String>>,
    body: Bytes,
) -> impl IntoResponse {
    // Select router based on method
    let path = orig_uri.path().to_string();

    // Try static first on GET so dynamic catch-alls don't shadow assets
    if method == Method::GET {
        if let Some(resp) = try_serve_static(&path).await { return resp; }
    }
    let (pipeline, path_params) = match method.as_str() {
        "GET" => match state.get_router.at(&path) {
            Ok(m) => (m.value.clone(), m.params.iter().map(|(k,v)| (k.to_string(), v.to_string())).collect::<HashMap<_,_>>()),
            Err(_) => {
                if let Some(resp) = try_serve_static(&path).await { return resp; }
                return StatusCode::NOT_FOUND.into_response();
            },
        },
        "POST" => match state.post_router.at(&path) {
            Ok(m) => (m.value.clone(), m.params.iter().map(|(k,v)| (k.to_string(), v.to_string())).collect::<HashMap<_,_>>()),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        "PUT" => match state.put_router.at(&path) {
            Ok(m) => (m.value.clone(), m.params.iter().map(|(k,v)| (k.to_string(), v.to_string())).collect::<HashMap<_,_>>()),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        "DELETE" => match state.delete_router.at(&path) {
            Ok(m) => (m.value.clone(), m.params.iter().map(|(k,v)| (k.to_string(), v.to_string())).collect::<HashMap<_,_>>()),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        _ => return StatusCode::METHOD_NOT_ALLOWED.into_response(),
    };

    respond_with_pipeline(state, method, headers, path_params, query_params, body, pipeline).await
}

async fn respond_with_pipeline(
    state: ServerState,
    method: Method,
    headers: HeaderMap,
    path_params: HashMap<String, String>,
    query_params: HashMap<String, String>,
    body: Bytes,
    pipeline: Arc<Pipeline>,
) -> Response {
    // Build unified request JSON and content type via shared helper
    let (request_json, _content_type) = build_request_from_axum(&method, &headers, &path_params, &query_params, &body);
    
    // Keep a snapshot for post_execute (contains _metadata, originalRequest, headers)
    let request_snapshot = request_json.clone();

    // Execute the pipeline
    match state.execute_pipeline(&pipeline, request_json).await {
        Ok((result, content_type, status_code)) => {
            // Determine the HTTP status code
            let http_status = if let Some(code) = status_code {
                StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
            } else {
                StatusCode::OK
            };
            
            let response = if content_type.starts_with("text/html") {
                match result.as_str() {
                    Some(html) => (http_status, Html(html.to_string())).into_response(),
                    None => {
                        // If not a string but should be HTML, try to serialize as JSON
                        match serde_json::to_string(&result) {
                            Ok(json_str) => (http_status, Html(json_str)).into_response(),
                            Err(_) => (http_status, Html("".to_string())).into_response()
                        }
                    }
                }
            } else {
                // If setCookies present, include Set-Cookie headers
                if let Some(cookies) = result.get("setCookies").and_then(|v| v.as_array()) {
                    use axum::response::Response as AxumResponse;
                    use axum::http::header::SET_COOKIE;
                    use axum::http::HeaderValue;
                    let mut response: AxumResponse = (http_status, Json(result.clone())).into_response();
                    let headers = response.headers_mut();
                    for cookie in cookies.iter().filter_map(|v| v.as_str()) {
                        if let Ok(val) = HeaderValue::from_str(cookie) {
                            headers.append(SET_COOKIE, val);
                        }
                    }
                    response
                } else {
                    (http_status, Json(result.clone())).into_response()
                }
            };

            // After building response, invoke post_execute for all middleware using an envelope that includes
            // originalRequest, headers, and _metadata from the initial request snapshot.
            let registry = state.middleware_registry.clone();
            let mut post_payload = if content_type.starts_with("text/html") {
                Value::Object(serde_json::Map::new())
            } else {
                result.clone()
            };
            if !post_payload.is_object() {
                post_payload = Value::Object(serde_json::Map::new());
            }
            if let Some(obj) = post_payload.as_object_mut() {
                // Only include _metadata when a log step has written it
                if request_snapshot.get("_metadata").and_then(|m| m.get("log")).is_some() {
                    if let Some(meta) = request_snapshot.get("_metadata").cloned() {
                        obj.insert("_metadata".to_string(), meta);
                    }
                }
                if let Some(orig) = request_snapshot.get("originalRequest").cloned() {
                    obj.insert("originalRequest".to_string(), orig);
                }
                if let Some(h) = request_snapshot.get("headers").cloned() {
                    obj.insert("headers".to_string(), h);
                }
            }
            tokio::spawn(async move { registry.post_execute_all(&post_payload).await; });

            response
        }
        Err(e) => {
            warn!("Pipeline execution failed: {}", e);
            let error_response = serde_json::json!({
                "error": "Pipeline execution failed",
                "details": e.to_string()
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response)).into_response()
        }
    }
}


async fn health_check() -> impl IntoResponse {
    let mut status = HashMap::new();
    status.insert("status", "ok");
    status.insert("version", env!("CARGO_PKG_VERSION"));
    
    (StatusCode::OK, serde_json::to_string(&status).unwrap())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received, starting graceful shutdown");
}

// --- Static file serving (public/ directory) ---

fn public_dir_path() -> PathBuf {
    std::env::var("WEBPIPE_PUBLIC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("public"))
}

fn normalize_static_path(request_path: &str) -> Option<PathBuf> {
    if !request_path.starts_with('/') { return None; }
    let mut rel = request_path.trim_start_matches('/');
    if rel.is_empty() { rel = "index.html"; }
    if rel.contains("..") { return None; }
    let mut full = public_dir_path();
    full.push(rel);
    Some(full)
}

fn guess_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

async fn try_serve_static(request_path: &str) -> Option<Response> {
    let mut candidate = normalize_static_path(request_path)?;

    if request_path.ends_with('/') {
        candidate.push("index.html");
    }

    if !candidate.exists() && Path::new(request_path).extension().is_none() {
        let mut idx = candidate.clone();
        idx.push("index.html");
        if idx.exists() { candidate = idx; }
    }

    let data = match tokio_fs::read(&candidate).await {
        Ok(bytes) => bytes,
        Err(_) => return None,
    };

    let ct = guess_content_type(&candidate);
    let mut resp = Response::new(Body::from(data));
    let _ = resp.headers_mut().insert(axum::http::header::CONTENT_TYPE, axum::http::HeaderValue::from_static(ct));
    Some(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn content_type_guessing() {
        assert_eq!(guess_content_type(Path::new("a.html")), "text/html; charset=utf-8");
        assert_eq!(guess_content_type(Path::new("a.css")), "text/css; charset=utf-8");
        assert_eq!(guess_content_type(Path::new("a.json")), "application/json; charset=utf-8");
        assert_eq!(guess_content_type(Path::new("a.bin")), "application/octet-stream");
    }

    #[test]
    fn static_path_normalization() {
        std::env::set_var("WEBPIPE_PUBLIC_DIR", "public");
        // normal
        let p = normalize_static_path("/index.html").unwrap();
        assert!(p.ends_with("index.html"));
        // traversal guarded
        assert!(normalize_static_path("/../secret").is_none());
        // root -> index.html
        let p2 = normalize_static_path("/").unwrap();
        assert!(p2.ends_with("index.html"));
    }

    #[tokio::test]
    async fn health_check_ok() {
        let resp = health_check().await.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }
}