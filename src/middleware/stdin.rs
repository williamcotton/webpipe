use crate::error::WebPipeError;
use async_trait::async_trait;

#[derive(Debug)]
pub struct StdinMiddleware;

#[async_trait]
impl super::Middleware for StdinMiddleware {
    async fn execute(
        &self,
        _args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<super::MiddlewareOutput, WebPipeError> {
        if ctx.invocation != crate::executor::InvocationKind::Script {
            return Err(WebPipeError::MiddlewareExecutionError(
                "stdin is only available in script mode".to_string(),
            ));
        }

        let bytes = ctx.stdin_bytes().await?;
        let mode = config.trim();
        let mode = if mode.is_empty() { "text" } else { mode };

        match mode {
            "text" => {
                let text = String::from_utf8(bytes).map_err(|e| {
                    WebPipeError::MiddlewareExecutionError(format!("stdin is not valid UTF-8: {e}"))
                })?;
                pipeline_ctx.state = serde_json::Value::String(text);
            }
            "json" => {
                pipeline_ctx.state = serde_json::from_slice(&bytes).map_err(|e| {
                    WebPipeError::MiddlewareExecutionError(format!("stdin is not valid JSON: {e}"))
                })?;
            }
            "lines" => {
                let text = String::from_utf8(bytes).map_err(|e| {
                    WebPipeError::MiddlewareExecutionError(format!("stdin is not valid UTF-8: {e}"))
                })?;
                pipeline_ctx.state = serde_json::Value::Array(
                    text.lines()
                        .map(|line| serde_json::Value::String(line.to_string()))
                        .collect(),
                );
            }
            other => {
                return Err(WebPipeError::MiddlewareExecutionError(format!(
                    "unknown stdin mode '{other}', expected text, json, or lines"
                )));
            }
        }

        Ok(super::MiddlewareOutput::default())
    }

    fn behavior(&self) -> super::StateBehavior {
        super::StateBehavior::Transform
    }
}
