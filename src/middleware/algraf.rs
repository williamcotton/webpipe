use crate::error::WebPipeError;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use algraf_data::Format;
use algraf_render::{
    render_embedded_with_io, EmbeddedOutputFormat, EmbeddedRenderError, EmbeddedRenderOptions,
    InMemoryDriverIo,
};

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

    /// Format for caller-provided `Chart(data: input)` / `Chart(data: stdin)`.
    ///
    /// Quoted sources such as `Chart(data: "rows.csv")` and
    /// `Table cities = "cities.json"` continue to use Algraf's normal
    /// extension-based format inference.
    #[serde(default = "default_data_format")]
    data_format: FormatOption,

    /// Named virtual files made available to quoted Algraf data sources.
    ///
    /// String values are used as raw UTF-8 file text. Non-string JSON values are
    /// serialized as JSON, so they should normally be addressed by a `.json`
    /// source in Algraf.
    #[serde(default)]
    data: HashMap<String, Value>,

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

impl From<AlgrafOutputType> for EmbeddedOutputFormat {
    fn from(value: AlgrafOutputType) -> Self {
        match value {
            AlgrafOutputType::Svg => EmbeddedOutputFormat::Svg,
            AlgrafOutputType::Png => EmbeddedOutputFormat::Png,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct FormatOption(String);

impl FormatOption {
    fn parse(&self) -> Result<Format, WebPipeError> {
        Format::from_str(&self.0).map_err(|message| {
            WebPipeError::MiddlewareExecutionError(format!("Invalid Algraf dataFormat: {message}"))
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

        let mut options = cfg.render_options()?;
        options.variables = variables_to_strings(cfg.variables);

        let input_bytes = input_bytes_for_format(&pipeline_ctx.state, options.data_format)?;
        let io = build_driver_io(input_bytes, cfg.data)?;
        let result = render_embedded_with_io(config, &io, options).map_err(algraf_error)?;
        let content_type = result.content_type;

        pipeline_ctx.state = match content_type {
            "image/svg+xml" => {
                let svg = String::from_utf8(result.bytes).map_err(|_| {
                    WebPipeError::MiddlewareExecutionError(
                        "Generated Algraf SVG was not valid UTF-8".to_string(),
                    )
                })?;
                Value::String(svg)
            }
            "image/png" => Value::String(general_purpose::STANDARD.encode(result.bytes)),
            other => {
                return Err(WebPipeError::MiddlewareExecutionError(format!(
                    "Unsupported Algraf content type: {other}"
                )));
            }
        };

        Ok(super::MiddlewareOutput {
            content_type: Some(content_type.to_string()),
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
            data: HashMap::new(),
            width: None,
            height: None,
            theme: None,
            strict: false,
            png_scale: None,
            png_dpi: None,
            variables: HashMap::new(),
        }
    }

    fn render_options(&self) -> Result<EmbeddedRenderOptions, WebPipeError> {
        let mut options = EmbeddedRenderOptions {
            data_format: self.data_format.parse()?,
            width: self.width,
            height: self.height,
            theme: self.theme.clone(),
            output_format: self.output_type.into(),
            strict: self.strict,
            png_dpi: self.png_dpi,
            ..EmbeddedRenderOptions::default()
        };

        if let Some(scale) = self.png_scale {
            options.png_scale = scale;
        }

        Ok(options)
    }
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

fn input_bytes_for_format(value: &Value, format: Format) -> Result<Vec<u8>, WebPipeError> {
    match format {
        Format::Json => {
            if let Value::String(text) = value {
                Ok(text.as_bytes().to_vec())
            } else {
                serde_json::to_vec(value).map_err(|e| {
                    WebPipeError::MiddlewareExecutionError(format!(
                        "Invalid Algraf pipeline state input: {e}"
                    ))
                })
            }
        }
        Format::Csv | Format::Tsv | Format::NdJson | Format::GeoJson | Format::TopoJson => {
            let text = value.as_str().ok_or_else(|| {
                WebPipeError::MiddlewareExecutionError(
                    "Algraf data input must be a string for non-JSON dataFormat".to_string(),
                )
            })?;
            Ok(text.as_bytes().to_vec())
        }
        Format::Shapefile => Err(WebPipeError::MiddlewareExecutionError(
            "Algraf dataFormat shapefile is not supported for Chart(data: input)".to_string(),
        )),
        #[allow(unreachable_patterns)]
        other => Err(WebPipeError::MiddlewareExecutionError(format!(
            "Algraf dataFormat {other:?} is not supported for Chart(data: input)"
        ))),
    }
}

fn build_driver_io(
    input_bytes: Vec<u8>,
    files: HashMap<String, Value>,
) -> Result<InMemoryDriverIo, WebPipeError> {
    let mut io = InMemoryDriverIo::new(input_bytes);

    for (name, value) in files {
        let path = virtual_file_path(&name)?;
        let bytes = virtual_file_bytes(&name, value)?;
        io = io.with_file(path, bytes);
    }

    Ok(io)
}

fn virtual_file_path(name: &str) -> Result<PathBuf, WebPipeError> {
    if name.trim().is_empty() {
        return Err(WebPipeError::MiddlewareExecutionError(
            "Algraf data file name must not be empty".to_string(),
        ));
    }

    let path = Path::new(name);
    if path.is_absolute() {
        return Err(WebPipeError::MiddlewareExecutionError(format!(
            "Algraf data file name must be relative: {name}"
        )));
    }

    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::ParentDir
            | Component::CurDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(WebPipeError::MiddlewareExecutionError(format!(
                    "Algraf data file name must stay inside the virtual data map: {name}"
                )));
            }
        }
    }

    Ok(PathBuf::from(".").join(path))
}

fn virtual_file_bytes(name: &str, value: Value) -> Result<Vec<u8>, WebPipeError> {
    match value {
        Value::String(text) => Ok(text.into_bytes()),
        other => serde_json::to_vec(&other).map_err(|e| {
            WebPipeError::MiddlewareExecutionError(format!("Invalid Algraf data file {name}: {e}"))
        }),
    }
}

fn algraf_error(error: EmbeddedRenderError) -> WebPipeError {
    match error {
        EmbeddedRenderError::Usage(message)
        | EmbeddedRenderError::Driver(message)
        | EmbeddedRenderError::Render(message) => {
            WebPipeError::MiddlewareExecutionError(format!("Algraf Render Error: {message}"))
        }
        EmbeddedRenderError::Diagnostics {
            diagnostics,
            data_warnings,
        } => {
            let mut parts = diagnostics
                .into_iter()
                .map(|diagnostic| {
                    format!(
                        "{} {:?}: {}",
                        diagnostic.code, diagnostic.severity, diagnostic.message
                    )
                })
                .collect::<Vec<_>>();
            parts.extend(
                data_warnings
                    .into_iter()
                    .map(|warning| format!("warning: {}", warning.message())),
            );
            WebPipeError::MiddlewareExecutionError(format!(
                "Algraf diagnostics blocked rendering: {}",
                parts.join("; ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{virtual_file_bytes, virtual_file_path};
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn virtual_paths_resolve_like_inline_algraf_sources() {
        assert_eq!(
            virtual_file_path("minard_troops.csv").unwrap(),
            PathBuf::from("./minard_troops.csv")
        );
        assert_eq!(
            virtual_file_path("data/minard_troops.csv").unwrap(),
            PathBuf::from("./data/minard_troops.csv")
        );
        assert!(virtual_file_path("../secret.csv").is_err());
        assert!(virtual_file_path("/tmp/secret.csv").is_err());
    }

    #[test]
    fn virtual_file_values_are_raw_strings_or_json() {
        assert_eq!(
            virtual_file_bytes("points.csv", json!("x,y\n1,2\n")).unwrap(),
            b"x,y\n1,2\n".to_vec()
        );
        assert_eq!(
            virtual_file_bytes("points.json", json!([{ "x": 1, "y": 2 }])).unwrap(),
            br#"[{"x":1,"y":2}]"#.to_vec()
        );
    }
}
