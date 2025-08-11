use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use handlebars::Handlebars;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Debug)]
pub struct HandlebarsMiddleware {
    pub(crate) handlebars: Arc<Mutex<Handlebars<'static>>>,
    pub(crate) registered_templates: Arc<Mutex<HashSet<String>>>,
}

impl Default for HandlebarsMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

impl HandlebarsMiddleware {
    pub fn new() -> Self {
        Self {
            handlebars: Arc::new(Mutex::new(Handlebars::new())),
            registered_templates: Arc::new(Mutex::new(HashSet::new())),
        }
    }
    pub fn new_with_ctx(ctx: Arc<Context>) -> Self {
        Self { handlebars: ctx.hb.clone(), registered_templates: Arc::new(Mutex::new(HashSet::new())) }
    }

    fn dedent_multiline(input: &str) -> String {
        // Trim only newline characters at the boundaries so that leading spaces
        // on the first line are preserved for indentation detection.
        let trimmed = input.trim_matches(['\n', '\r'].as_ref());
        let lines: Vec<&str> = trimmed.lines().collect();
        if lines.is_empty() { return String::new(); }

        let mut min_indent: Option<usize> = None;
        for line in &lines {
            if line.trim().is_empty() { continue; }
            let indent = line.chars().take_while(|c| *c == ' ' || *c == '\t').count();
            min_indent = Some(match min_indent { Some(current) => current.min(indent), None => indent });
        }

        let common = min_indent.unwrap_or(0);
        if common == 0 { return trimmed.to_string(); }

        let mut out = String::with_capacity(trimmed.len());
        for (i, line) in lines.iter().enumerate() {
            let mut to_strip = common; let mut byte_idx = 0;
            for ch in line.chars() {
                if to_strip > 0 && (ch == ' ' || ch == '\t') { to_strip -= 1; byte_idx += ch.len_utf8(); } else { break; }
            }
            out.push_str(&line[byte_idx..]);
            if i < lines.len() - 1 { out.push('\n'); }
        }
        out
    }

    fn render_template(&self, template: &str, data: &Value) -> Result<String, WebPipeError> {
        let mut handlebars = self.handlebars.lock();
        let tpl = Self::dedent_multiline(template);
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        tpl.hash(&mut hasher);
        let id = format!("tpl_{:x}", hasher.finish());

        let mut compiled = self.registered_templates.lock();
        if !compiled.contains(&id) {
            if let Err(e) = handlebars.register_template_string(&id, &tpl) {
                return Err(WebPipeError::MiddlewareExecutionError(format!("Handlebars template compile error: {}", e)));
            }
            compiled.insert(id.clone());
        }

        handlebars
            .render(&id, data)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Handlebars render error: {}", e)))
    }
}

#[async_trait]
impl super::Middleware for HandlebarsMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        let rendered = self.render_template(config, input)?;
        Ok(Value::String(rendered))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::Middleware;

    #[test]
    fn test_dedent_multiline() {
        let s = "\n    hello\n      world\n";
        let out = HandlebarsMiddleware::dedent_multiline(s);
        assert_eq!(out, "hello\n  world");
    }

    #[tokio::test]
    async fn render_template_and_register_once() {
        let hb = HandlebarsMiddleware::new();
        let data = serde_json::json!({"name":"Ada"});
        let v1 = hb.execute("Hello {{name}}", &data).await.unwrap();
        assert_eq!(v1, serde_json::json!("Hello Ada"));
        let v2 = hb.execute("Hello {{name}}", &data).await.unwrap();
        assert_eq!(v2, serde_json::json!("Hello Ada"));
    }

    #[test]
    fn test_dedent_preserves_first_line_leading_spaces_for_detection() {
        let s = "\n  a\n    b\n";
        let out = HandlebarsMiddleware::dedent_multiline(s);
        assert_eq!(out, "a\n  b");
    }
}

