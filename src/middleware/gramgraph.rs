use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::Value;
use gramgraph::{data::PlotData, parser, runtime, RenderOptions, OutputFormat};
use base64::{Engine as _, engine::general_purpose};

#[derive(Debug)]
pub struct GramGraphMiddleware;

impl GramGraphMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl super::Middleware for GramGraphMiddleware {
    async fn execute(
        &self,
        args: &[String], // Arguments passed like gg({...})
        config: &str,    // The DSL string passed after the colon
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<(), WebPipeError> {
        
        // 1. Parse RenderOptions from the first argument (if present)
        let options: RenderOptions = if let Some(arg_json) = args.first() {
            serde_json::from_str(arg_json).map_err(|e| 
                WebPipeError::MiddlewareExecutionError(format!("Invalid GramGraph options JSON: {}", e))
            )?
        } else {
            RenderOptions::default()
        };

        // 2. Convert Pipeline State (JSON) to PlotData
        // The state must be an array of objects
        let plot_data = PlotData::from_json(&pipeline_ctx.state)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Invalid data for GramGraph: {}", e)))?;
        
        // 3. Parse GramGraph DSL
        let (_, spec) = parser::parse_plot_spec(config)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("GramGraph DSL Parse Error: {:?}", e)))?;

        // 4. Render Plot
        // This returns Vec<u8> (bytes) representing either PNG data or SVG text
        let output_bytes = runtime::render_plot(spec, plot_data, options.clone())
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("GramGraph Render Error: {}", e)))?;

        // 5. Process Output based on Format
        let result_string = match options.format {
            OutputFormat::Svg => {
                // For SVG, simply convert bytes to UTF-8 string
                String::from_utf8(output_bytes)
                    .map_err(|_| WebPipeError::MiddlewareExecutionError("Generated SVG was not valid UTF-8".to_string()))?
            },
            OutputFormat::Png => {
                // For PNG, encode bytes to Base64 string so it can be safely transported in JSON
                general_purpose::STANDARD.encode(&output_bytes)
            }
        };

        // 6. Replace Pipeline State
        pipeline_ctx.state = Value::String(result_string);
        
        Ok(())
    }
    
    fn behavior(&self) -> super::StateBehavior {
        // Transform behavior replaces the state entirely (Data -> Image String)
        super::StateBehavior::Transform
    }
}
