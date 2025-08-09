use crate::ast::{Program, Pipeline, PipelineRef, PipelineStep, Variable};
use crate::middleware::{MiddlewareRegistry, configure_handlebars_partials};
use crate::config::{self};
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
use std::{collections::HashMap, net::SocketAddr, sync::Arc, pin::Pin, future::Future};
use tokio::signal;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};
use tokio::fs as tokio_fs;
use std::path::{Path, PathBuf};

fn string_to_number_or_string(s: &str) -> Value {
    // Try integer first
    if let Ok(i) = s.parse::<i64>() {
        return Value::Number(i.into());
    }
    // Then float
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

fn merge_values_preserving_input(input: &serde_json::Value, result: &serde_json::Value) -> serde_json::Value {
    match (input, result) {
        (serde_json::Value::Object(base), serde_json::Value::Object(patch)) => {
            let mut merged = base.clone();
            for (k, v) in patch {
                merged.insert(k.clone(), v.clone());
            }
            serde_json::Value::Object(merged)
        }
        _ => result.clone(),
    }
}

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
    variables: Arc<Vec<Variable>>, // for resolving middleware variables and auto-naming
    named_pipelines: Arc<std::collections::HashMap<String, Arc<Pipeline>>>,
}

impl ServerState {
    fn resolve_config_and_autoname(
        &self,
        middleware_name: &str,
        step_config: &str,
        input: &serde_json::Value,
    ) -> (String, serde_json::Value, bool) {
        // Try to find a variable matching this middleware and config
        if let Some(var) = self
            .variables
            .iter()
            .find(|v| v.var_type == middleware_name && v.name == step_config)
        {
            // Use the variable's value as config
            let resolved_config = var.value.clone();

            // If input is an object and resultName is missing, set it to the variable name
            let mut new_input = input.clone();
            let mut auto_named = false;
            if let Some(obj) = new_input.as_object_mut() {
                let has_result_name = obj.get("resultName").is_some();
                if !has_result_name {
                    obj.insert("resultName".to_string(), serde_json::Value::String(var.name.clone()));
                    auto_named = true;
                }
            }

            return (resolved_config, new_input, auto_named);
        }

        // No variable resolution; return as-is
        (step_config.to_string(), input.clone(), false)
    }

    pub fn execute_pipeline<'a>(
        &'a self,
        pipeline: &'a Pipeline,
        mut input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<(Value, String, Option<u16>), WebPipeError>> + Send + 'a>> {
        Box::pin(async move {
        let mut content_type = "application/json".to_string();
        let mut status_code_out: Option<u16> = None;
        
        for (idx, step) in pipeline.steps.iter().enumerate() {
            let is_last_step = idx + 1 == pipeline.steps.len();
            match step {
                PipelineStep::Regular { name, config } => {
                    // info!("Executing middleware: {} with config: {}", name, config);
                    
                    // Resolve variables and auto-name if applicable
                    let (effective_config, effective_input, auto_named) =
                        self.resolve_config_and_autoname(name, config, &input);

                    // Support a virtual middleware step for named pipelines: `pipeline: <name>`
                    let exec_result: Result<(Value, Option<u16>, String), WebPipeError> = if name == "pipeline" {
                        let name = effective_config.trim();
                        if let Some(p) = self.named_pipelines.get(name) {
                            let (val, ct, st) = self.execute_pipeline(p, effective_input.clone()).await?;
                            Ok((val, st, ct))
                        } else {
                            Err(WebPipeError::PipelineNotFound(name.to_string()))
                        }
                    } else {
                        let val = self.middleware_registry.execute(name, &effective_config, &effective_input).await?;
                        Ok((val, None, content_type.clone()))
                    };

                    match exec_result {
                        Ok((result, sub_status, sub_content_type)) => {
                            let mut next_input = if is_last_step {
                                // final step controls output
                                result
                            } else {
                                merge_values_preserving_input(&effective_input, &result)
                            };
                            if auto_named {
                                if let Some(obj) = next_input.as_object_mut() {
                                    obj.remove("resultName");
                                }
                            }
                            input = next_input;
                            
                            // Special handling for content type changes
                            if name == "handlebars" {
                                content_type = "text/html".to_string();
                            } else if name == "pipeline" && is_last_step {
                                content_type = sub_content_type;
                            }
                            if is_last_step {
                                if let Some(s) = sub_status { status_code_out = Some(s); }
                            }
                        }
                        Err(e) => {
                            warn!("Middleware {} failed: {}", name, e);
                            return Err(WebPipeError::MiddlewareExecutionError(e.to_string()));
                        }
                    }
                }
                PipelineStep::Result { branches } => {
                    return self.handle_result_step(branches, input, content_type).await;
                }
            }
        }
        
        Ok((input, content_type, status_code_out))
        })
    }

    async fn handle_result_step(
        &self,
        branches: &[crate::ast::ResultBranch],
        input: Value,
        mut content_type: String,
    ) -> Result<(Value, String, Option<u16>), WebPipeError> {
        // Check for errors in the input data
        let error_type = self.extract_error_type(&input);
        
        // Find matching branch
        let selected_branch = self.select_branch(branches, &error_type);
        
        if let Some(branch) = selected_branch {
            // Preserve any setCookies from input unless branch overwrites it
            let inherited_cookies = input.get("setCookies").cloned();
            // info!("Executing result branch: {:?} with status {}", branch.branch_type, branch.status_code);
            
            // Execute the branch's pipeline
            let (mut result, branch_content_type, _) = self.execute_pipeline(&branch.pipeline, input).await?;
            if inherited_cookies.is_some() {
                if let Some(obj) = result.as_object_mut() {
                    if !obj.contains_key("setCookies") {
                        if let Some(c) = inherited_cookies { obj.insert("setCookies".to_string(), c); }
                    }
                }
            }
            
            // Update content type if the branch changed it
            if branch_content_type != "application/json" {
                content_type = branch_content_type;
            }
            
            // Return the result with the status code from the branch
            Ok((result, content_type, Some(branch.status_code)))
        } else {
            // No branch matched - should not happen if parser is correct
            warn!("No matching branch found for error type: {:?}", error_type);
            Ok((input, content_type, None))
        }
    }

    fn extract_error_type(&self, input: &Value) -> Option<String> {
        // Look for errors array in the input
        if let Some(errors) = input.get("errors") {
            if let Some(errors_array) = errors.as_array() {
                if let Some(first_error) = errors_array.first() {
                    if let Some(error_type) = first_error.get("type") {
                        if let Some(type_str) = error_type.as_str() {
                            return Some(type_str.to_string());
                        }
                    }
                }
            }
        }
        None
    }

    fn select_branch<'a>(
        &self,
        branches: &'a [crate::ast::ResultBranch],
        error_type: &Option<String>,
    ) -> Option<&'a crate::ast::ResultBranch> {
        use crate::ast::ResultBranchType;
        
        // If there's an error, try to match it to a custom branch
        if let Some(err_type) = error_type {
            for branch in branches {
                if let ResultBranchType::Custom(branch_name) = &branch.branch_type {
                    if branch_name == err_type {
                        return Some(branch);
                    }
                }
            }
        }
        
        // If no error or no matching custom branch, try to find 'ok' branch
        if error_type.is_none() {
            for branch in branches {
                if matches!(branch.branch_type, ResultBranchType::Ok) {
                    return Some(branch);
                }
            }
        }
        
        // Fall back to default branch
        for branch in branches {
            if matches!(branch.branch_type, ResultBranchType::Default) {
                return Some(branch);
            }
        }
        
        None
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
    pub fn from_program(program: Program) -> Self {
        let middleware_registry = Arc::new(MiddlewareRegistry::new());
        // Initialize partials from variables for handlebars mustache-like behavior
        configure_handlebars_partials(&program.variables);

        // Initialize global config from program configs
        config::init_global(program.configs.clone());
        Self {
            program,
            middleware_registry,
        }
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

            info!("Registered route: {} {}", route.method, path);
        }

        let server_state = ServerState {
            middleware_registry: self.middleware_registry.clone(),
            get_router: Arc::new(get_router),
            post_router: Arc::new(post_router),
            put_router: Arc::new(put_router),
            delete_router: Arc::new(delete_router),
            variables: Arc::new(self.program.variables.clone()),
            named_pipelines: Arc::new(self.program.pipelines.iter().map(|p| (p.name.clone(), Arc::new(p.pipeline.clone()))).collect()),
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
    let method_str = method.to_string();
    
    // Convert headers to HashMap
    let headers_map: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    
    // Extract cookies from headers
    let cookies: HashMap<String, String> = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .filter_map(|cookie| {
            let mut parts = cookie.trim().split('=');
            let key = parts.next()?.to_string();
            let value = parts.next().unwrap_or("").to_string();
            Some((key, value))
        })
        .collect();
    
    // Determine content type
    let content_type = headers_map
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| "application/json".to_string());
    
    // Parse body based on content type
    let parsed_body = if body.is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        match content_type.as_str() {
            ct if ct.starts_with("application/json") => {
                serde_json::from_slice(&body).unwrap_or(Value::Null)
            }
            ct if ct.starts_with("application/x-www-form-urlencoded") => {
                let form_str = String::from_utf8_lossy(&body);
                let form_data: HashMap<String, String> = form_str
                    .split('&')
                    .filter_map(|pair| {
                        let mut parts = pair.split('=');
                        let key = parts.next()?.to_string();
                        let value = parts.next().unwrap_or("").to_string();
                        Some((key, value))
                    })
                    .collect();
                string_map_to_json_with_number_coercion(&form_data)
            }
            _ => Value::String(String::from_utf8_lossy(&body).to_string()),
        }
    };
    
    // Create WebPipe request object
    let webpipe_request = WebPipeRequest {
        method: method_str.clone(),
        path: "".to_string(), // Path is handled by Axum routing
        query: query_params.clone(),
        params: path_params.clone(),
        headers: headers_map,
        cookies,
        body: parsed_body,
        content_type,
    };
    
    // Convert to JSON for pipeline processing
    let mut request_json = match serde_json::to_value(&webpipe_request) {
        Ok(json) => json,
        Err(e) => {
            warn!("Failed to serialize request: {}", e);
            let error_response = serde_json::json!({
                "error": "Failed to serialize request",
                "details": e.to_string()
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response)).into_response();
        }
    };

    // Coerce numeric-looking strings in params and query into numbers
    if let Some(obj) = request_json.as_object_mut() {
        let coerced_params = string_map_to_json_with_number_coercion(&path_params);
        let coerced_query = string_map_to_json_with_number_coercion(&query_params);
        obj.insert("params".to_string(), coerced_params.clone());
        obj.insert("query".to_string(), coerced_query.clone());
        // Attach originalRequest snapshot for later reference in pipelines
        let mut orig = serde_json::Map::new();
        orig.insert("method".to_string(), Value::String(method_str.clone()));
        orig.insert("params".to_string(), coerced_params);
        orig.insert("query".to_string(), coerced_query);
        obj.insert("originalRequest".to_string(), Value::Object(orig));
    }
    
    // Ensure a baseline _metadata.log.startTimeMs exists for duration calc
    if let Some(obj) = request_json.as_object_mut() {
        let meta_entry = obj.entry("_metadata").or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !meta_entry.is_object() { *meta_entry = Value::Object(serde_json::Map::new()); }
        if let Some(meta) = meta_entry.as_object_mut() {
            let log_entry = meta.entry("log").or_insert_with(|| Value::Object(serde_json::Map::new()));
            if !log_entry.is_object() { *log_entry = Value::Object(serde_json::Map::new()); }
            if let Some(log_obj) = log_entry.as_object_mut() {
                if !log_obj.contains_key("startTimeMs") {
                    use std::time::{SystemTime, UNIX_EPOCH};
                    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                    let ms = now.as_millis() as u64;
                    log_obj.insert("startTimeMs".to_string(), Value::Number(serde_json::Number::from(ms)));
                }
            }
        }
    }

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
                if let Some(meta) = request_snapshot.get("_metadata").cloned() {
                    obj.insert("_metadata".to_string(), meta);
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

    if !candidate.exists() {
        if Path::new(request_path).extension().is_none() {
            let mut idx = candidate.clone();
            idx.push("index.html");
            if idx.exists() { candidate = idx; }
        }
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