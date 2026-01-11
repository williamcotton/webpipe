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
    variables: HashMap<String, String>,
}

#[async_trait]
impl super::Middleware for GramGraphMiddleware {
    async fn execute(
        &self,
        args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<(), WebPipeError> {
        // 1. Parse Options and Variables
        let (options, variables) = if let Some(arg_json) = args.first() {
            let cfg: GramGraphConfig = serde_json::from_str(arg_json).map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!(
                    "Invalid GramGraph config: {}",
                    e
                ))
            })?;
            (cfg.options, cfg.variables)
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

        // 5. Process Output
        let result_string = match options.format {
            OutputFormat::Svg => String::from_utf8(output_bytes).map_err(|_| {
                WebPipeError::MiddlewareExecutionError(
                    "Generated SVG was not valid UTF-8".to_string(),
                )
            })?,
            OutputFormat::Png => general_purpose::STANDARD.encode(&output_bytes),
        };

        // 6. Replace Pipeline State
        pipeline_ctx.state = Value::String(result_string);

        Ok(())
    }

    fn behavior(&self) -> super::StateBehavior {
        super::StateBehavior::Transform
    }
}
