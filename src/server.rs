use crate::ast::{Program, Pipeline, PipelineRef, PipelineStep};
use crate::middleware::MiddlewareRegistry;
use crate::error::WebPipeError;
use axum::{
    body::{Bytes, Body},
    extract::{Query, State, OriginalUri, ConnectInfo},
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
use tracing::{debug, info, warn};
use tokio::fs as tokio_fs;
use std::path::{Path, PathBuf};
use crate::runtime::Context;
use crate::executor::{ExecutionEnv, RealInvoker, PipelineExecFuture};
use crate::http::request::build_request_from_axum_with_ip;

// number coercion helpers moved to http::request

// merge helper moved to shared executor

/// Recursive function to detect if a pipeline requires feature flags
fn pipeline_needs_flags(
    pipeline: &Pipeline,
    named_pipelines: &HashMap<String, Arc<Pipeline>>
) -> bool {
    for step in &pipeline.steps {
        match step {
            PipelineStep::Regular { name, config, tags, .. } => {
                // 1. Direct Usage: @flag(...)
                if tags.iter().any(|t| t.name == "flag") {
                    return true;
                }

                // 2. Explicit Opt-in: @needs(flags)
                if tags.iter().any(|t| t.name == "needs" && t.args.contains(&"flags".to_string())) {
                    return true;
                }

                // 3. Recursive Reference: |> pipeline: name
                if name == "pipeline" {
                    let target = config.trim();
                    if let Some(sub) = named_pipelines.get(target) {
                        if pipeline_needs_flags(sub, named_pipelines) {
                            return true;
                        }
                    }
                }
            },
            PipelineStep::Result { branches } => {
                // 4. Recursive Branching: |> result ...
                for branch in branches {
                    if pipeline_needs_flags(&branch.pipeline, named_pipelines) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

pub struct WebPipeServer {
    program: Program,
    middleware_registry: Arc<MiddlewareRegistry>,
    ctx: Arc<Context>,
}

#[derive(Clone)]
pub struct RoutePayload {
    pub pipeline: Arc<Pipeline>,
    pub needs_flags: bool,
}

#[derive(Clone)]
pub struct ServerState {
    middleware_registry: Arc<MiddlewareRegistry>,
    get_router: Arc<matchit::Router<RoutePayload>>,
    post_router: Arc<matchit::Router<RoutePayload>>,
    put_router: Arc<matchit::Router<RoutePayload>>,
    delete_router: Arc<matchit::Router<RoutePayload>>,
    env: Arc<ExecutionEnv>,
    feature_flags: Option<Arc<Pipeline>>,
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

    /// Extract feature flags from request JSON and create a new ExecutionEnv with those flags
    fn env_with_flags(&self, request_json: &Value) -> ExecutionEnv {
        let mut flags = HashMap::new();

        // Extract flags from _metadata.flags
        if let Some(metadata) = request_json.get("_metadata") {
            if let Some(flags_obj) = metadata.get("flags") {
                if let Some(flags_map) = flags_obj.as_object() {
                    for (key, value) in flags_map {
                        if let Some(bool_val) = value.as_bool() {
                            flags.insert(key.clone(), bool_val);
                        }
                    }
                }
            }
        }

        // Create a new env with the extracted flags
        ExecutionEnv {
            variables: self.env.variables.clone(),
            named_pipelines: self.env.named_pipelines.clone(),
            invoker: self.env.invoker.clone(),
            environment: self.env.environment.clone(),
            async_registry: crate::executor::AsyncTaskRegistry::new(),
            flags: Arc::new(flags),
            cache: self.env.cache.clone(),
            deferred: Arc::new(parking_lot::Mutex::new(Vec::new())),
        }
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
        // 1. Build GraphQL runtime first (if schema exists)
        let graphql_runtime = if program.graphql_schema.is_some() {
            Some(Arc::new(
                crate::graphql::GraphQLRuntime::from_program(&program)
                    .map_err(|e| WebPipeError::ConfigError(format!("GraphQL initialization failed: {}", e)))?
            ))
        } else {
            None
        };

        // 2. Build a Context from program configs (also initializes global config)
        let mut ctx = Context::from_program_configs(program.configs.clone(), &program.variables).await?;

        // 3. Add GraphQL runtime to context
        ctx.graphql = graphql_runtime;

        // 4. Build middleware registry with context
        let ctx_arc = Arc::new(ctx);
        let middleware_registry = Arc::new(MiddlewareRegistry::with_builtins(ctx_arc.clone()));

        Ok(Self {
            program,
            middleware_registry,
            ctx: ctx_arc,
        })
    }

    pub fn router(self) -> Router {
        let mut router = Router::new().route("/health", get(health_check));

        // Build named_pipelines first for static analysis
        let mut named_pipelines: HashMap<String, Arc<Pipeline>> = self.program
            .pipelines
            .iter()
            .map(|p| (p.name.clone(), Arc::new(p.pipeline.clone())))
            .collect();

        // Add GraphQL query resolvers to named_pipelines
        for query in &self.program.queries {
            named_pipelines.insert(
                format!("query::{}", query.name),
                Arc::new(query.pipeline.clone())
            );
        }

        // Add GraphQL mutation resolvers to named_pipelines
        for mutation in &self.program.mutations {
            named_pipelines.insert(
                format!("mutation::{}", mutation.name),
                Arc::new(mutation.pipeline.clone())
            );
        }

        // Build method-specific matchers with RoutePayload
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

            // Static analysis: determine if this route needs flags
            let needs_flags = pipeline_needs_flags(&pipeline, &named_pipelines);

            let payload = RoutePayload {
                pipeline: pipeline.clone(),
                needs_flags,
            };

            match route.method.as_str() {
                "GET" => {
                    let _ = get_router.insert(path.clone(), payload.clone());
                }
                "POST" => {
                    let _ = post_router.insert(path.clone(), payload.clone());
                }
                "PUT" => {
                    let _ = put_router.insert(path.clone(), payload.clone());
                }
                "DELETE" => {
                    let _ = delete_router.insert(path.clone(), payload.clone());
                }
                other => {
                    warn!("Unsupported HTTP method: {}", other);
                    continue;
                }
            }
        }

        let env = Arc::new(ExecutionEnv {
            variables: Arc::new(self.program.variables.clone()),
            named_pipelines: Arc::new(named_pipelines),
            invoker: Arc::new(RealInvoker::new(self.middleware_registry.clone())),
            environment: std::env::var("WEBPIPE_ENV").ok(),
            async_registry: crate::executor::AsyncTaskRegistry::new(),
            flags: Arc::new(HashMap::new()),
            cache: Some(self.ctx.cache.clone()),
            deferred: Arc::new(parking_lot::Mutex::new(Vec::new())),
        });

        // Set the execution environment in the Context so GraphQL middleware can access it
        *self.ctx.execution_env.write() = Some(env.clone());

        let server_state = ServerState {
            middleware_registry: self.middleware_registry.clone(),
            get_router: Arc::new(get_router),
            post_router: Arc::new(post_router),
            put_router: Arc::new(put_router),
            delete_router: Arc::new(delete_router),
            env: env.clone(),
            feature_flags: self.program.feature_flags.as_ref().map(|p| Arc::new(p.clone())),
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
        
        info!("Web Pipe server starting on {}", addr);
        
        axum::serve(listener, self.router().into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown_signal())
            .await?;
            
        info!("Web Pipe server shutdown complete");
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

        info!("Web Pipe server starting on {}", addr);

        axum::serve(listener, self.router().into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown)
            .await?;

        info!("Web Pipe server shutdown complete");
        Ok(())
    }
}

// configure_pg_from_config removed: middleware reads config directly

async fn unified_handler(
    State(state): State<ServerState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    method: Method,
    headers: HeaderMap,
    OriginalUri(orig_uri): OriginalUri,
    Query(query_params): Query<HashMap<String, String>>,
    body: Bytes,
) -> impl IntoResponse {
    // Select router based on method
    let path = orig_uri.path().to_string();
    
    // Extract remote IP from connection (if available)
    let remote_ip = connect_info.map(|ConnectInfo(addr)| addr.ip().to_string());

    // Try static first on GET so dynamic catch-alls don't shadow assets
    if method == Method::GET {
        if let Some(resp) = try_serve_static(&path).await { return resp; }
    }
    let (payload, path_params) = match method.as_str() {
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

    respond_with_pipeline(state, method, headers, path, path_params, query_params, body, payload, remote_ip.as_deref()).await
}

async fn respond_with_pipeline(
    state: ServerState,
    method: Method,
    headers: HeaderMap,
    path: String,
    path_params: HashMap<String, String>,
    query_params: HashMap<String, String>,
    body: Bytes,
    payload: RoutePayload,
    remote_ip: Option<&str>,
) -> Response {
    // Build unified request JSON and content type via shared helper
    let (mut request_json, _content_type) = build_request_from_axum_with_ip(&method, &headers, &path, &path_params, &query_params, &body, remote_ip);

    // --- PRE-FLIGHT CHECK: Execute feature flags pipeline if needed ---
    if payload.needs_flags {
        if let Some(flag_pipeline) = &state.feature_flags {
            // Execute Flag Pipeline with Timeout (50ms)
            let flag_result = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                state.execute_pipeline(flag_pipeline, request_json.clone())
            ).await;

            match flag_result {
                Ok(Ok((result_json, _, _))) => {
                    // Merge _metadata from result back into request_json
                    if let Some(new_meta) = result_json.get("_metadata") {
                        if let Some(obj) = request_json.as_object_mut() {
                            obj.insert("_metadata".to_string(), new_meta.clone());
                        }
                    }

                    // Merge user (if auth happened in pre-flight)
                    if let Some(user) = result_json.get("user") {
                        if let Some(obj) = request_json.as_object_mut() {
                            obj.insert("user".to_string(), user.clone());
                        }
                    }
                },
                Ok(Err(e)) => {
                    warn!("Flag resolution error: {}", e);
                },
                Err(_) => {
                    warn!("Flag resolution timed out");
                }
            }
        }
    }

    // Keep a snapshot for post_execute (contains _metadata, originalRequest, headers)
    let request_snapshot = request_json.clone();

    // Create ExecutionEnv with feature flags from request
    let env_with_flags = state.env_with_flags(&request_json);

    // Execute the pipeline with the flags-aware environment
    match crate::executor::execute_pipeline(&env_with_flags, &payload.pipeline, request_json.clone()).await {
        Ok((result, content_type, status_code)) => {
            // Run all deferred actions (e.g. caching, logging) with the clean result
            // Logging middleware captures request context upfront, cache stores clean result
            env_with_flags.run_deferred(&result, &content_type);

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
            let _registry = state.middleware_registry.clone();
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
            response
        }
        Err(e) => {
            // Use debug logging for rate limits (expected during testing), warn for actual errors
            match &e {
                crate::error::WebPipeError::RateLimitExceeded(_) => {
                    debug!("Rate limit exceeded: {}", e);
                }
                _ => {
                    warn!("Pipeline execution failed: {}", e);
                }
            }
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
    use std::path::Component;
    use percent_encoding::percent_decode_str;

    if !request_path.starts_with('/') { return None; }

    // Decode URL encoding to catch encoded traversal attempts like %2e%2e
    let decoded = match percent_decode_str(request_path).decode_utf8() {
        Ok(s) => s.into_owned(),
        Err(_) => return None,
    };

    let mut rel = decoded.trim_start_matches('/');
    if rel.is_empty() { rel = "index.html"; }

    // Parse the path and ensure no parent directory components exist
    let path = Path::new(rel);
    for component in path.components() {
        match component {
            Component::Normal(_) => continue,
            Component::RootDir | Component::CurDir => continue,
            Component::ParentDir => return None, // Block any .. components
            Component::Prefix(_) => return None,  // Block Windows drive prefixes
        }
    }

    // Build the full path and canonicalize to resolve any remaining tricks
    let public_dir = public_dir_path();
    let full = public_dir.join(rel);

    // Canonicalize both paths to resolve symlinks and relative components
    // This is the final defense - ensure the resolved path is still under public_dir
    match (full.canonicalize(), public_dir.canonicalize()) {
        (Ok(canonical_full), Ok(canonical_public)) => {
            if canonical_full.starts_with(&canonical_public) {
                Some(full)
            } else {
                None
            }
        }
        _ => {
            // If canonicalization fails (file doesn't exist yet), do a prefix check on the non-canonical paths
            // This allows serving of files that exist while still protecting against traversal
            if full.starts_with(&public_dir) {
                Some(full)
            } else {
                None
            }
        }
    }
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
    use axum::http::HeaderValue;
    use std::collections::HashMap;

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

    #[test]
    fn path_traversal_attacks_blocked() {
        std::env::set_var("WEBPIPE_PUBLIC_DIR", "public");

        // Classic path traversal
        assert!(normalize_static_path("/../../../etc/passwd").is_none());
        assert!(normalize_static_path("/../../secret").is_none());
        assert!(normalize_static_path("/foo/../../bar").is_none());

        // URL-encoded traversal attempts
        assert!(normalize_static_path("/%2e%2e/etc/passwd").is_none());
        assert!(normalize_static_path("/%2e%2e%2f%2e%2e%2fetc%2fpasswd").is_none());
        assert!(normalize_static_path("/foo/%2e%2e/%2e%2e/secret").is_none());

        // Mixed encoding
        assert!(normalize_static_path("/foo/..%2f..%2fsecret").is_none());
        assert!(normalize_static_path("/%2e%2e/../etc/passwd").is_none());

        // Double-encoded attempts (decodes to %2e%2e which is not a valid filename, so it will return Some but the file won't exist)
        // This is acceptable since the path still doesn't escape the public directory

        // On Windows, backslashes are path separators and should be blocked in combination with ..
        // On Unix, backslash is just a regular filename character, so /..\ is a directory named ".."
        #[cfg(target_os = "windows")]
        {
            assert!(normalize_static_path("/..\\..\\secret").is_none());
        }

        // Null byte injection (caught by UTF-8 validation)
        assert!(normalize_static_path("/../secret%00.txt").is_none() ||
                normalize_static_path("/../secret%00.txt").unwrap().to_str().unwrap().contains("secret%00"));

        // Valid paths should still work
        assert!(normalize_static_path("/index.html").is_some());
        assert!(normalize_static_path("/assets/style.css").is_some());
        assert!(normalize_static_path("/js/app.js").is_some());
        assert!(normalize_static_path("/").is_some());
    }

    #[test]
    fn path_normalization_with_special_chars() {
        std::env::set_var("WEBPIPE_PUBLIC_DIR", "public");

        // URL-encoded normal characters should work
        assert!(normalize_static_path("/file%20name.html").is_some());
        assert!(normalize_static_path("/foo%2Fbar.txt").is_some());

        // Current directory references should be handled
        assert!(normalize_static_path("/./file.html").is_some());
        assert!(normalize_static_path("/foo/./bar.txt").is_some());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_drive_prefixes_blocked() {
        std::env::set_var("WEBPIPE_PUBLIC_DIR", "public");

        // Windows absolute paths and drive letters should be blocked
        assert!(normalize_static_path("/C:/Windows/System32/config/sam").is_none());
        assert!(normalize_static_path("/C:\\Windows\\System32").is_none());
    }

    #[tokio::test]
    async fn health_check_ok() {
        let resp = health_check().await.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn try_serve_static_serves_file_and_none_for_missing() {
        let base = std::env::temp_dir().join(format!("wp_public_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&base).await.unwrap();
        std::env::set_var("WEBPIPE_PUBLIC_DIR", &base);
        let file_path = base.join("hello.txt");
        tokio_fs::write(&file_path, b"hi").await.unwrap();
        // direct file
        let resp_opt = try_serve_static("/hello.txt").await;
        assert!(resp_opt.is_some());
        // missing -> None
        let none = try_serve_static("/nope.txt").await;
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn respond_with_pipeline_sets_cookies_and_html() {
        // Build a minimal state with jq/handlebars available
        let ctx = Context::from_program_configs(vec![], &[]).await.unwrap();
        let registry = Arc::new(MiddlewareRegistry::with_builtins(Arc::new(ctx)));
        let env = ExecutionEnv {
            variables: Arc::new(vec![]),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(RealInvoker::new(registry.clone())),
            environment: None,
            async_registry: crate::executor::AsyncTaskRegistry::new(),
            flags: Arc::new(HashMap::new()),
            cache: None,
            deferred: Arc::new(parking_lot::Mutex::new(Vec::new())),
        };
        let state = ServerState {
            middleware_registry: registry.clone(),
            get_router: Arc::new(matchit::Router::new()),
            post_router: Arc::new(matchit::Router::new()),
            put_router: Arc::new(matchit::Router::new()),
            delete_router: Arc::new(matchit::Router::new()),
            env: Arc::new(env),
            feature_flags: None,
        };
        // Craft a tiny pipeline that sets cookies via jq
        let p_set_cookie = Arc::new(Pipeline { steps: vec![
            crate::ast::PipelineStep::Regular { name: "jq".to_string(), config: "{ setCookies: [\"a=b\"] }".to_string(), config_type: crate::ast::ConfigType::Quoted, tags: vec![] }
        ]});
        let resp = respond_with_pipeline(
            state.clone(),
            Method::GET,
            HeaderMap::new(),
            "/test".to_string(),
            HashMap::new(),
            HashMap::new(),
            axum::body::Bytes::new(),
            RoutePayload { pipeline: p_set_cookie.clone(), needs_flags: false },
            Some("127.0.0.1"),
        ).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // Header present
        assert!(resp.headers().get_all(axum::http::header::SET_COOKIE).iter().any(|h| h == &HeaderValue::from_static("a=b")));

        // Pipeline that renders HTML
        let p_html = Arc::new(Pipeline { steps: vec![
            crate::ast::PipelineStep::Regular { name: "handlebars".to_string(), config: "<p>OK</p>".to_string(), config_type: crate::ast::ConfigType::Quoted, tags: vec![] }
        ]});
        let resp2 = respond_with_pipeline(
            state,
            Method::GET,
            HeaderMap::new(),
            "/test".to_string(),
            HashMap::new(),
            HashMap::new(),
            axum::body::Bytes::new(),
            RoutePayload { pipeline: p_html.clone(), needs_flags: false },
            Some("127.0.0.1"),
        ).await;
        assert_eq!(resp2.status(), StatusCode::OK);
        assert_eq!(resp2.headers().get(axum::http::header::CONTENT_TYPE).unwrap(), "text/html; charset=utf-8");
    }
}