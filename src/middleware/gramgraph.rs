use crate::error::WebPipeError;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use gramgraph::{
    data::PlotData,
    parser,
    runtime,
    preprocessor,
    RenderOptions,
    OutputFormat,
};
use base64::{Engine as _, engine::general_purpose};

#[derive(Debug)]
pub struct GramGraphMiddleware;

impl GramGraphMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Deserialize)]
struct GramGraphConfig {
    #[serde(flatten)]
    options: RenderOptions,

    #[serde(default)]
    variables: HashMap<String, Value>,
}

#[async_trait]
impl super::Middleware for GramGraphMiddleware {
    async fn execute(
        &self,
        args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &crate::executor::ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<super::MiddlewareOutput, WebPipeError> {
        // 1. Parse Options and Variables
        let (options, variables) = if let Some(arg_expr) = args.first() {
            // Create combined input with context
            let combined_input = serde_json::json!({
                "state": pipeline_ctx.state,
                "context": ctx.to_value(env)
            });

            // Wrap expression to bind $context and evaluate against state
            let wrapped_expr = format!(".context as $context | .state | ({})", arg_expr);

            // Evaluate JQ expression
            let cfg_value = crate::runtime::jq::evaluate(&wrapped_expr, &combined_input)?;

            // Parse result as config
            let cfg: GramGraphConfig = serde_json::from_value(cfg_value).map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!(
                    "Invalid GramGraph config: {}",
                    e
                ))
            })?;

            // Convert variables to strings (handle numbers, booleans, etc.)
            let variables_str: HashMap<String, String> = cfg.variables.into_iter().map(|(k, v)| {
                let s = match v {
                    Value::String(s) => s,
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Null => String::new(),
                    _ => v.to_string(),
                };
                (k, s)
            }).collect();

            (cfg.options, variables_str)
        } else {
            (RenderOptions::default(), HashMap::new())
        };

        // 2. Convert Pipeline State (JSON) to PlotData
        let plot_data = PlotData::from_json(&pipeline_ctx.state).map_err(|e| {
            WebPipeError::MiddlewareExecutionError(format!(
                "Invalid data for GramGraph: {}",
                e
            ))
        })?;

        // 3a. Preprocess: Expand Variables
        let expanded_dsl =
            preprocessor::expand_variables(config, &variables).map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!(
                    "GramGraph Variable Expansion Error: {}",
                    e
                ))
            })?;

        // 3b. Parse GramGraph DSL using expanded string
        let (_, spec) = parser::parse_plot_spec(&expanded_dsl).map_err(|e| {
            WebPipeError::MiddlewareExecutionError(format!(
                "GramGraph DSL Parse Error: {:?}",
                e
            ))
        })?;

        // 4. Render Plot (updated signature)
        let output_bytes =
            runtime::render_plot(spec, plot_data, options.clone()).map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!(
                    "GramGraph Render Error: {}",
                    e
                ))
            })?;

        // 5. Determine Content Type
        let content_type = match options.format {
            OutputFormat::Svg => "image/svg+xml",
            OutputFormat::Png => "image/png",
        };

        // 6. Process Output
        let result_string = match options.format {
            OutputFormat::Svg => String::from_utf8(output_bytes).map_err(|_| {
                WebPipeError::MiddlewareExecutionError(
                    "Generated SVG was not valid UTF-8".to_string(),
                )
            })?,
            OutputFormat::Png => general_purpose::STANDARD.encode(&output_bytes),
        };

        // 7. Replace Pipeline State
        pipeline_ctx.state = Value::String(result_string);

        Ok(super::MiddlewareOutput {
            content_type: Some(content_type.to_string()),
        })
    }

    fn behavior(&self) -> super::StateBehavior {
        super::StateBehavior::Transform
    }
}
