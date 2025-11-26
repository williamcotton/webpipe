use crate::error::WebPipeError;
use crate::middleware::Middleware;
use async_trait::async_trait;

#[derive(Debug)]
pub struct JoinMiddleware;

#[async_trait]
impl Middleware for JoinMiddleware {
    async fn execute(
        &self,
        config: &str,
        _pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
    ) -> Result<(), WebPipeError> {
        // Validate config upfront to fail fast on bad syntax
        let _task_names = parse_join_config(config)?;

        // This middleware is a marker - the executor intercepts "join" steps
        // and handles them via handle_join() which reads the config directly.
        // State unchanged.
        Ok(())
    }
}

fn parse_join_config(config: &str) -> Result<Vec<String>, WebPipeError> {
    let trimmed = config.trim();

    // Try parsing as JSON array first
    if trimmed.starts_with('[') {
        match serde_json::from_str::<Vec<String>>(trimmed) {
            Ok(names) => return Ok(names),
            Err(_) => {
                return Err(WebPipeError::ConfigError(
                    "Invalid JSON array for join config".to_string()
                ));
            }
        }
    }

    // Otherwise parse as comma-separated list
    let names: Vec<String> = trimmed
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if names.is_empty() {
        return Err(WebPipeError::ConfigError(
            "join config must specify at least one task name".to_string()
        ));
    }

    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_comma_separated_names() {
        let names = parse_join_config("user,notifications").unwrap();
        assert_eq!(names, vec!["user", "notifications"]);
    }

    #[test]
    fn parse_comma_separated_with_spaces() {
        let names = parse_join_config("user, notifications, stats").unwrap();
        assert_eq!(names, vec!["user", "notifications", "stats"]);
    }

    #[test]
    fn parse_json_array() {
        let names = parse_join_config(r#"["user","notifications","stats"]"#).unwrap();
        assert_eq!(names, vec!["user", "notifications", "stats"]);
    }

    #[test]
    fn parse_empty_fails() {
        assert!(parse_join_config("").is_err());
    }

    #[test]
    fn parse_whitespace_only_fails() {
        assert!(parse_join_config("   ").is_err());
    }
}
