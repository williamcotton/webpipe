use crate::error::WebPipeError;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use algraf_core::Severity;
use algraf_data::{read_format, read_json_str, Format, LoadResult, Table};
use algraf_render::{render, Theme};
use algraf_semantics::analyze;
use algraf_syntax::parse;

#[derive(Debug)]
pub struct AlgrafMiddleware;

impl AlgrafMiddleware {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AlgrafConfig {
    #[serde(rename = "type", default)]
    output_type: AlgrafOutputType,

    #[serde(default = "default_data_format")]
    data_format: FormatOption,

    #[serde(default)]
    width: Option<u32>,

    #[serde(default)]
    height: Option<u32>,

    #[serde(default)]
    theme: Option<String>,

    #[serde(default)]
    strict: bool,

    #[serde(default)]
    png_scale: Option<f32>,

    #[serde(default)]
    png_dpi: Option<u32>,

    #[serde(default)]
    variables: HashMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum AlgrafOutputType {
    Svg,
    Png,
}

impl Default for AlgrafOutputType {
    fn default() -> Self {
        AlgrafOutputType::Svg
    }
}

#[derive(Debug, Clone, Deserialize)]
struct FormatOption(String);

impl FormatOption {
    fn parse(&self) -> Result<Format, WebPipeError> {
        Ok(match self.0.to_ascii_lowercase().as_str() {
            "csv" => Format::Csv,
            "tsv" | "tab" => Format::Tsv,
            "json" => Format::Json,
            "ndjson" | "jsonl" => Format::NdJson,
            "geojson" => Format::GeoJson,
            "topojson" => Format::TopoJson,
            "shp" | "shapefile" => Format::Shapefile,
            other => {
                return Err(WebPipeError::MiddlewareExecutionError(format!(
                    "Invalid Algraf dataFormat: {other}"
                )))
            }
        })
    }
}

fn default_data_format() -> FormatOption {
    FormatOption("json".to_string())
}

#[async_trait]
impl super::Middleware for AlgrafMiddleware {
    async fn execute(
        &self,
        args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &crate::executor::ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<super::MiddlewareOutput, WebPipeError> {
        let cfg = if let Some(arg_expr) = args.first() {
            let combined_input = serde_json::json!({
                "state": pipeline_ctx.state,
                "context": ctx.to_value(env),
            });
            let wrapped_expr = format!(".context as $context | .state | ({})", arg_expr);
            let cfg_value = crate::runtime::jq::evaluate(&wrapped_expr, &combined_input)?;
            serde_json::from_value(cfg_value).map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!("Invalid Algraf config: {e}"))
            })?
        } else {
            AlgrafConfig::default_for_inline_json()
        };

        if matches!(cfg.output_type, AlgrafOutputType::Png) {
            return Err(WebPipeError::MiddlewareExecutionError(
                "Algraf PNG output is not supported by the current renderer; use type: \"svg\""
                    .to_string(),
            ));
        }

        let expanded_spec = expand_variables(config, &variables_to_strings(cfg.variables.clone()));
        let options = cfg.into_render_options()?;
        let LoadResult {
            frame,
            warnings: data_warnings,
        } = load_data(&pipeline_ctx.state, options.data_format)?;
        let parse_result = parse(&expanded_spec);
        let syntax = parse_result.syntax();
        let mut diagnostics = parse_result.into_diagnostics();
        let analysis = analyze(&syntax, frame.schema());
        diagnostics.extend(analysis.diagnostics);

        if should_block(&diagnostics, options.strict) {
            return Err(diagnostics_error("Algraf diagnostics blocked rendering", &diagnostics));
        }

        let ir = analysis.ir.ok_or_else(|| {
            WebPipeError::MiddlewareExecutionError(
                "Algraf analysis did not produce a chart IR".to_string(),
            )
        })?;

        let theme = options
            .theme
            .as_deref()
            .map(Theme::by_name)
            .unwrap_or_else(Theme::minimal);

        let render_result = render(&ir, &frame, &theme, None).map_err(|e| {
            WebPipeError::MiddlewareExecutionError(format!("Algraf Render Error: {e}"))
        })?;

        if should_block(&render_result.diagnostics, options.strict) {
            return Err(diagnostics_error(
                "Algraf diagnostics blocked rendering",
                &render_result.diagnostics,
            ));
        }
        if options.strict && !data_warnings.is_empty() {
            return Err(WebPipeError::MiddlewareExecutionError(format!(
                "Algraf data warnings blocked rendering: {}",
                data_warnings
                    .into_iter()
                    .map(|w| w.message)
                    .collect::<Vec<_>>()
                    .join("; ")
            )));
        }

        pipeline_ctx.state = Value::String(render_result.svg);

        Ok(super::MiddlewareOutput {
            content_type: Some("image/svg+xml".to_string()),
        })
    }

    fn behavior(&self) -> super::StateBehavior {
        super::StateBehavior::Transform
    }
}

impl AlgrafConfig {
    fn default_for_inline_json() -> Self {
        AlgrafConfig {
            output_type: AlgrafOutputType::Svg,
            data_format: default_data_format(),
            width: None,
            height: None,
            theme: None,
            strict: false,
            png_scale: None,
            png_dpi: None,
            variables: HashMap::new(),
        }
    }

    fn into_render_options(self) -> Result<AlgrafRenderOptions, WebPipeError> {
        Ok(AlgrafRenderOptions {
            data_format: self.data_format.parse()?,
            theme: self.theme,
            strict: self.strict,
        })
    }
}

struct AlgrafRenderOptions {
    data_format: Format,
    theme: Option<String>,
    strict: bool,
}

fn variables_to_strings(variables: HashMap<String, Value>) -> HashMap<String, String> {
    variables
        .into_iter()
        .map(|(key, value)| {
            let value = match value {
                Value::String(value) => value,
                Value::Number(value) => value.to_string(),
                Value::Bool(value) => value.to_string(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            (key, value)
        })
        .collect()
}

fn load_data(value: &Value, format: Format) -> Result<LoadResult, WebPipeError> {
    match format {
        Format::Json => {
            let json_text = if let Value::String(text) = value {
                text.clone()
            } else {
                serde_json::to_string(value).map_err(|e| {
                    WebPipeError::MiddlewareExecutionError(format!(
                        "Invalid Algraf pipeline state input: {e}"
                    ))
                })?
            };
            read_json_str(&json_text).map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!("Invalid Algraf data input: {e}"))
            })
        }
        Format::Csv | Format::Tsv | Format::NdJson | Format::GeoJson | Format::TopoJson => {
            let text = value.as_str().ok_or_else(|| {
                WebPipeError::MiddlewareExecutionError(
                    "Algraf data input must be a string for non-JSON dataFormat".to_string(),
                )
            })?;
            read_format(text.as_bytes(), format).map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!("Invalid Algraf data input: {e}"))
            })
        }
        Format::Shapefile => Err(WebPipeError::MiddlewareExecutionError(
            "Algraf dataFormat shapefile is not supported for inline middleware input".to_string(),
        )),
    }
}

fn should_block(diagnostics: &[algraf_core::Diagnostic], strict: bool) -> bool {
    diagnostics.iter().any(|d| {
        matches!(d.severity, Severity::Error) || (strict && matches!(d.severity, Severity::Warning))
    })
}

fn diagnostics_error(message: &str, diagnostics: &[algraf_core::Diagnostic]) -> WebPipeError {
    let details = diagnostics
        .iter()
        .map(|diagnostic| format!("{} {:?}: {}", diagnostic.code, diagnostic.severity, diagnostic.message))
        .collect::<Vec<_>>()
        .join("; ");
    WebPipeError::MiddlewareExecutionError(format!("{message}: {details}"))
}

fn expand_variables(spec: &str, variables: &HashMap<String, String>) -> String {
    let re = regex::Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").expect("valid algraf variable regex");
    re.replace_all(spec, |caps: &regex::Captures| {
        let key = &caps[1];
        variables
            .get(key)
            .cloned()
            .unwrap_or_else(|| caps[0].to_string())
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::expand_variables;
    use std::collections::HashMap;

    #[test]
    fn expand_variables_replaces_scalar_tokens() {
        let mut vars = HashMap::new();
        vars.insert("size".to_string(), "3".to_string());
        vars.insert("color".to_string(), "#e74c3c".to_string());
        let spec = r#"Line(stroke: "$color", strokeWidth: $size)"#;
        let expanded = expand_variables(spec, &vars);
        assert_eq!(
            expanded,
            "Line(stroke: \"#e74c3c\", strokeWidth: 3)"
        );
    }
}
