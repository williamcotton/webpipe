use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WebPipeError {
    #[error("At {location}: {source}")]
    Located {
        location: String,
        #[source]
        source: Box<WebPipeError>,
    },

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

    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),

    #[error("Import not found: {0}")]
    ImportNotFound(String),

    #[error("Circular import detected: {0}")]
    CircularImport(String),

    #[error("Duplicate import alias: {0}")]
    DuplicateAlias(String),

    #[error("Undefined scoped reference: {0}")]
    UndefinedScopedReference(String),

    #[error("Debugger error: {0}")]
    DebuggerError(String),
}

impl WebPipeError {
    fn format_middleware_details(message: &str) -> String {
        if let Some((summary, payload)) = message.split_once(": ") {
            format!("Middleware execution failed, {summary}:\n{payload}")
        } else {
            format!("Middleware execution failed:\n{message}")
        }
    }

    pub fn with_source_location(self, location: &crate::ast::SourceLocation) -> Self {
        let Some(location) = location.error_label() else {
            return self;
        };

        match self {
            WebPipeError::Located { .. } => self,
            other => WebPipeError::Located {
                location,
                source: Box::new(other),
            },
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            WebPipeError::Located { source, .. } => source.status_code(),
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
            WebPipeError::RateLimitExceeded(_) => StatusCode::TOO_MANY_REQUESTS,
            WebPipeError::ImportNotFound(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::CircularImport(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::DuplicateAlias(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::UndefinedScopedReference(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WebPipeError::DebuggerError(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn error_type(&self) -> &'static str {
        match self {
            WebPipeError::Located { source, .. } => source.error_type(),
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
            WebPipeError::RateLimitExceeded(_) => "rate_limit_exceeded",
            WebPipeError::ImportNotFound(_) => "import_not_found",
            WebPipeError::CircularImport(_) => "circular_import",
            WebPipeError::DuplicateAlias(_) => "duplicate_alias",
            WebPipeError::UndefinedScopedReference(_) => "undefined_scoped_reference",
            WebPipeError::DebuggerError(_) => "debugger_error",
        }
    }

    pub fn display_pipeline_details(&self) -> String {
        match self {
            WebPipeError::Located { location, source } => match source.as_ref() {
                WebPipeError::MiddlewareExecutionError(message) => {
                    format!("At {location}\n{}", Self::format_middleware_details(message))
                }
                other => format!("At {location}\n{other}"),
            },
            WebPipeError::MiddlewareExecutionError(message) => {
                Self::format_middleware_details(message)
            }
            other => other.to_string(),
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
    use axum::body;
    use crate::ast::SourceLocation;

    #[test]
    fn status_code_mapping_basic() {
        assert_eq!(WebPipeError::ParseError("x".into()).status_code(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(WebPipeError::AuthError("x".into()).status_code(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(WebPipeError::NotFound("x".into()).status_code(), axum::http::StatusCode::NOT_FOUND);
    }

    #[test]
    fn into_response_wraps_type_message_and_status() {
        let err = WebPipeError::BadRequest("oops".into());
        let resp = err.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        // Extract body to ensure structure
        let body = resp.into_body();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let bytes = rt.block_on(body::to_bytes(body, usize::MAX)).expect("bytes");
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["error"]["type"], serde_json::json!("bad_request"));
        assert_eq!(v["error"]["status"], serde_json::json!(400));
    }

    #[test]
    fn located_error_delegates_status_and_type() {
        let loc = SourceLocation::with_file(4, 1, 0, "app.wp".to_string());
        let err = WebPipeError::BadRequest("oops".into()).with_source_location(&loc);
        assert_eq!(err.status_code(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(err.error_type(), "bad_request");
        assert_eq!(err.to_string(), "At app.wp:4: Bad request: oops");
    }

    #[test]
    fn located_error_is_not_wrapped_twice() {
        let loc = SourceLocation::with_file(4, 1, 0, "app.wp".to_string());
        let err = WebPipeError::BadRequest("oops".into())
            .with_source_location(&loc)
            .with_source_location(&loc);
        assert_eq!(err.to_string(), "At app.wp:4: Bad request: oops");
    }

    #[test]
    fn display_pipeline_details_formats_structured_middleware_payload() {
        let loc = SourceLocation::with_file(4493, 1, 0, "/tmp/test.wp".to_string());
        let err = WebPipeError::MiddlewareExecutionError(
            "JQ compilation error: [(File { code: \".context as $context | .state | { test: params[0] }\", path: () }, [(\"params\", Filter(0))])]".into()
        ).with_source_location(&loc);

        assert_eq!(
            err.display_pipeline_details(),
            "At /tmp/test.wp:4493\nMiddleware execution failed, JQ compilation error:\n[(File { code: \".context as $context | .state | { test: params[0] }\", path: () }, [(\"params\", Filter(0))])]"
        );
    }

    #[test]
    fn display_pipeline_details_formats_any_middleware_payload() {
        let loc = SourceLocation::with_file(12, 1, 0, "/tmp/test.wp".to_string());
        let err = WebPipeError::MiddlewareExecutionError(
            "Handlebars render error: missing key `title`".into()
        ).with_source_location(&loc);

        assert_eq!(
            err.display_pipeline_details(),
            "At /tmp/test.wp:12\nMiddleware execution failed, Handlebars render error:\nmissing key `title`"
        );
    }
}
