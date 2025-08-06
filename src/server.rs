use crate::ast::{Program, Pipeline, PipelineRef, PipelineStep};
use crate::middleware::MiddlewareRegistry;
use axum::{
    body::Bytes,
    extract::{Path as AxumPath, Query, State},
    http::{Method, StatusCode, HeaderMap},
    response::{IntoResponse, Json},
    routing::{get, on, MethodFilter},
    Router,
};
use serde_json::Value;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::signal;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

pub struct WebPipeServer {
    program: Program,
    middleware_registry: Arc<MiddlewareRegistry>,
}

#[derive(Debug, Clone)]
pub struct ServerState {
    program: Program,
    middleware_registry: Arc<MiddlewareRegistry>,
}

impl ServerState {
    pub async fn execute_pipeline(
        &self,
        pipeline: &Pipeline,
        mut input: Value,
    ) -> Result<(Value, String), Box<dyn std::error::Error>> {
        let mut content_type = "application/json".to_string();
        
        for step in &pipeline.steps {
            match step {
                PipelineStep::Regular { name, config } => {
                    // info!("Executing middleware: {} with config: {}", name, config);
                    
                    match self.middleware_registry.execute(name, config, &input).await {
                        Ok(result) => {
                            input = result;
                            
                            // Special handling for content type changes
                            if name == "mustache" {
                                content_type = "text/html".to_string();
                            }
                        }
                        Err(e) => {
                            warn!("Middleware {} failed: {}", name, e);
                            return Err(Box::new(e));
                        }
                    }
                }
                PipelineStep::Result { branches: _branches } => {
                    // TODO: Implement result step handling
                    info!("Result step not yet implemented");
                }
            }
        }
        
        Ok((input, content_type))
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
        Self {
            program,
            middleware_registry,
        }
    }

    pub fn router(self) -> Router {
        let mut router = Router::new().route("/health", get(health_check));

        let server_state = ServerState {
            program: self.program.clone(),
            middleware_registry: self.middleware_registry.clone(),
        };

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

            let filter = match route.method.as_str() {
                "GET" => MethodFilter::GET,
                "POST" => MethodFilter::POST,
                "PUT" => MethodFilter::PUT,
                "DELETE" => MethodFilter::DELETE,
                other => {
                    warn!("Unsupported HTTP method: {}", other);
                    continue;
                }
            };

            info!("Registering route: {} {}", route.method, path);
            
            let pipeline_clone = pipeline.clone();
            let handler = move |
                state: State<ServerState>,
                method: Method,
                headers: HeaderMap,
                path_params: AxumPath<HashMap<String, String>>,
                query_params: Query<HashMap<String, String>>,
                body: Bytes,
            | async move {
                handle_pipeline_request(state, method, headers, path_params, query_params, body, pipeline_clone.clone()).await
            };
            
            router = router.route(&path, on(filter, handler));
        }

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
}

async fn handle_pipeline_request(
    State(state): State<ServerState>,
    method: Method,
    headers: HeaderMap,
    AxumPath(path_params): AxumPath<HashMap<String, String>>,
    Query(query_params): Query<HashMap<String, String>>,
    body: Bytes,
    pipeline: Arc<Pipeline>,
) -> impl IntoResponse {
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
                serde_json::to_value(form_data).unwrap_or(Value::Null)
            }
            _ => Value::String(String::from_utf8_lossy(&body).to_string()),
        }
    };
    
    // Create WebPipe request object
    let webpipe_request = WebPipeRequest {
        method: method_str,
        path: "".to_string(), // Path is handled by Axum routing
        query: query_params,
        params: path_params,
        headers: headers_map,
        cookies,
        body: parsed_body,
        content_type,
    };
    
    // Convert to JSON for pipeline processing
    let request_json = match serde_json::to_value(&webpipe_request) {
        Ok(json) => json,
        Err(e) => {
            warn!("Failed to serialize request: {}", e);
            let error_response = serde_json::json!({
                "error": "Failed to serialize request",
                "details": e.to_string()
            });
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response));
        }
    };
    
    // Execute the pipeline
    match state.execute_pipeline(&pipeline, request_json).await {
        Ok((result, content_type)) => {
            if content_type.starts_with("text/html") {
                match result.as_str() {
                    Some(html) => (StatusCode::OK, Json(serde_json::json!({"html": html}))),
                    None => (StatusCode::OK, Json(result))
                }
            } else {
                (StatusCode::OK, Json(result))
            }
        }
        Err(e) => {
            warn!("Pipeline execution failed: {}", e);
            let error_response = serde_json::json!({
                "error": "Pipeline execution failed",
                "details": e.to_string()
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response))
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