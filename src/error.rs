use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WebPipeError {
    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Route not found: {0}")]
    RouteNotFound(String),

    #[error("Pipeline not found: {0}")]
    PipelineNotFound(String),

    #[error("Middleware not found: {0}")]
    MiddlewareNotFound(String),

    #[error("Middleware execution failed: {0}")]
    MiddlewareExecutionError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Authentication error: {0}")]
    AuthError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Template error: {0}")]
    TemplateError(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Cache error: {0}")]
    CacheError(String),

    #[error("Script execution error: {0}")]
    ScriptError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    JsonError(#[from] serde_json::Error),

    #[error("Internal server error: {0}")]
    InternalError(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Method not allowed: {0}")]
    MethodNotAllowed(String),

    #[error("Timeout error: {0}")]
    Timeout(String),
}

impl WebPipeError {
    pub fn status_code(&self) -> StatusCode {
        match self {
            WebPipeError::ParseError(_) => StatusCode::BAD_REQUEST,
            WebPipeError::RouteNotFound(_) => StatusCode::NOT_FOUND,
            WebPipeError::PipelineNotFound(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::MiddlewareNotFound(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::MiddlewareExecutionError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::ConfigError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::DatabaseError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::AuthError(_) => StatusCode::UNAUTHORIZED,
            WebPipeError::ValidationError(_) => StatusCode::BAD_REQUEST,
            WebPipeError::TemplateError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::NetworkError(_) => StatusCode::BAD_GATEWAY,
            WebPipeError::CacheError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::ScriptError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::IoError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::JsonError(_) => StatusCode::BAD_REQUEST,
            WebPipeError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::BadRequest(_) => StatusCode::BAD_REQUEST,
            WebPipeError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            WebPipeError::Forbidden(_) => StatusCode::FORBIDDEN,
            WebPipeError::NotFound(_) => StatusCode::NOT_FOUND,
            WebPipeError::MethodNotAllowed(_) => StatusCode::METHOD_NOT_ALLOWED,
            WebPipeError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self {
            WebPipeError::ParseError(_) => "parse_error",
            WebPipeError::RouteNotFound(_) => "route_not_found",
            WebPipeError::PipelineNotFound(_) => "pipeline_not_found",
            WebPipeError::MiddlewareNotFound(_) => "middleware_not_found",
            WebPipeError::MiddlewareExecutionError(_) => "middleware_execution_error",
            WebPipeError::ConfigError(_) => "config_error",
            WebPipeError::DatabaseError(_) => "database_error",
            WebPipeError::AuthError(_) => "auth_error",
            WebPipeError::ValidationError(_) => "validation_error",
            WebPipeError::TemplateError(_) => "template_error",
            WebPipeError::NetworkError(_) => "network_error",
            WebPipeError::CacheError(_) => "cache_error",
            WebPipeError::ScriptError(_) => "script_error",
            WebPipeError::IoError(_) => "io_error",
            WebPipeError::JsonError(_) => "json_error",
            WebPipeError::InternalError(_) => "internal_error",
            WebPipeError::BadRequest(_) => "bad_request",
            WebPipeError::Unauthorized(_) => "unauthorized",
            WebPipeError::Forbidden(_) => "forbidden",
            WebPipeError::NotFound(_) => "not_found",
            WebPipeError::MethodNotAllowed(_) => "method_not_allowed",
            WebPipeError::Timeout(_) => "timeout",
        }
    }
}

impl IntoResponse for WebPipeError {
    fn into_response(self) -> Response {
        let status = self.status_code();
        let error_type = self.error_type();
        let message = self.to_string();

        let body = json!({
            "error": {
                "type": error_type,
                "message": message,
                "status": status.as_u16()
            }
        });

        (status, Json(body)).into_response()
    }
}

pub type Result<T> = std::result::Result<T, WebPipeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_code_mapping_basic() {
        assert_eq!(WebPipeError::ParseError("x".into()).status_code(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(WebPipeError::AuthError("x".into()).status_code(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(WebPipeError::NotFound("x".into()).status_code(), axum::http::StatusCode::NOT_FOUND);
    }
}