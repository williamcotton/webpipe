use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::{Number, Value};

#[derive(Debug)]
pub struct AssertMiddleware;

#[derive(Debug, Clone)]
enum AssertSchema {
    Unknown,
    Null,
    Bool,
    Number,
    String {
        min: Option<usize>,
        max: Option<usize>,
    },
    Email,
    Literal(Value),
    Array(Box<AssertSchema>),
    Object(Vec<ObjectField>),
    Union(Vec<AssertSchema>),
}

#[derive(Debug, Clone)]
struct ObjectField {
    name: String,
    optional: bool,
    schema: AssertSchema,
}

#[async_trait]
impl super::Middleware for AssertMiddleware {
    async fn execute(
        &self,
        _args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<super::MiddlewareOutput, WebPipeError> {
        let schema = Parser::new(config).parse().map_err(|err| {
            WebPipeError::MiddlewareExecutionError(format!("assert schema parse error: {err}"))
        })?;

        if let Err(err) = validate_value(&pipeline_ctx.state, &schema, "$") {
            pipeline_ctx.state = make_assertion_error(&err.path, &err.message);
        }

        Ok(super::MiddlewareOutput::default())
    }
}

#[derive(Debug, Clone)]
struct ValidationIssue {
    path: String,
    message: String,
}

fn validate_value(value: &Value, schema: &AssertSchema, path: &str) -> Result<(), ValidationIssue> {
    match schema {
        AssertSchema::Unknown => Ok(()),
        AssertSchema::Null => {
            if value.is_null() {
                Ok(())
            } else {
                Err(type_issue(path, "null", value))
            }
        }
        AssertSchema::Bool => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(type_issue(path, "boolean", value))
            }
        }
        AssertSchema::Number => {
            if value.is_number() {
                Ok(())
            } else {
                Err(type_issue(path, "number", value))
            }
        }
        AssertSchema::String { min, max } => {
            let Some(s) = value.as_str() else {
                return Err(type_issue(path, "string", value));
            };

            let len = s.chars().count();
            if let Some(min) = min {
                if len < *min {
                    return Err(ValidationIssue {
                        path: path.to_string(),
                        message: format!(
                            "Expected string at {path} to be at least {min} characters"
                        ),
                    });
                }
            }
            if let Some(max) = max {
                if len > *max {
                    return Err(ValidationIssue {
                        path: path.to_string(),
                        message: format!(
                            "Expected string at {path} to be at most {max} characters"
                        ),
                    });
                }
            }
            Ok(())
        }
        AssertSchema::Email => {
            let Some(s) = value.as_str() else {
                return Err(type_issue(path, "email string", value));
            };
            if s.contains('@') && s.contains('.') && !s.starts_with('@') && !s.ends_with('@') {
                Ok(())
            } else {
                Err(ValidationIssue {
                    path: path.to_string(),
                    message: format!("Expected valid email at {path}"),
                })
            }
        }
        AssertSchema::Literal(expected) => {
            if value == expected {
                Ok(())
            } else {
                Err(ValidationIssue {
                    path: path.to_string(),
                    message: format!(
                        "Expected literal {} at {path}, got {}",
                        display_json(expected),
                        type_name(value)
                    ),
                })
            }
        }
        AssertSchema::Array(item_schema) => {
            let Some(items) = value.as_array() else {
                return Err(type_issue(path, "array", value));
            };
            for (idx, item) in items.iter().enumerate() {
                validate_value(item, item_schema, &format!("{path}[{idx}]"))?;
            }
            Ok(())
        }
        AssertSchema::Object(fields) => {
            let Some(obj) = value.as_object() else {
                return Err(type_issue(path, "object", value));
            };

            for field in fields {
                let child_path = if path == "$" {
                    format!("$.{}", field.name)
                } else {
                    format!("{path}.{}", field.name)
                };
                match obj.get(&field.name) {
                    Some(child) => validate_value(child, &field.schema, &child_path)?,
                    None if field.optional => {}
                    None => {
                        return Err(ValidationIssue {
                            path: child_path.clone(),
                            message: format!("Missing required field {child_path}"),
                        });
                    }
                }
            }

            Ok(())
        }
        AssertSchema::Union(options) => {
            let mut messages = Vec::new();
            for option in options {
                match validate_value(value, option, path) {
                    Ok(()) => return Ok(()),
                    Err(err) => messages.push(err.message),
                }
            }
            Err(ValidationIssue {
                path: path.to_string(),
                message: format!(
                    "Value at {path} did not match any union member: {}",
                    messages.join("; ")
                ),
            })
        }
    }
}

fn type_issue(path: &str, expected: &str, actual: &Value) -> ValidationIssue {
    ValidationIssue {
        path: path.to_string(),
        message: format!("Expected {expected} at {path}, got {}", type_name(actual)),
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn display_json(value: &Value) -> String {
    match value {
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    }
}

fn make_assertion_error(path: &str, message: &str) -> Value {
    let field = path.strip_prefix("$.").unwrap_or(path);
    serde_json::json!({
        "errors": [{
            "type": "assertionError",
            "field": field,
            "context": field,
            "rule": "assert",
            "code": "assert_failed",
            "message": message
        }]
    })
}

#[derive(Debug)]
struct Parser<'a> {
    input: &'a str,
    chars: Vec<char>,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<AssertSchema, String> {
        let schema = self.parse_union()?;
        self.skip_ws();
        if self.is_eof() {
            Ok(schema)
        } else {
            Err(format!(
                "unexpected token '{}'",
                self.peek().unwrap_or('\0')
            ))
        }
    }

    fn parse_union(&mut self) -> Result<AssertSchema, String> {
        let mut options = vec![self.parse_primary()?];
        loop {
            self.skip_ws();
            if !self.eat('|') {
                break;
            }
            options.push(self.parse_primary()?);
        }

        if options.len() == 1 {
            Ok(options.remove(0))
        } else {
            Ok(AssertSchema::Union(options))
        }
    }

    fn parse_primary(&mut self) -> Result<AssertSchema, String> {
        self.skip_ws();
        match self.peek() {
            Some('{') => self.parse_object(),
            Some('[') => self.parse_array(),
            Some('"') => Ok(AssertSchema::Literal(Value::String(self.parse_string()?))),
            Some(ch) if ch == '-' || ch.is_ascii_digit() => self.parse_number_literal(),
            Some(ch) if is_ident_start(ch) => self.parse_named_type_or_literal(),
            Some(ch) => Err(format!("unexpected token '{ch}'")),
            None => Err("unexpected end of schema".to_string()),
        }
    }

    fn parse_object(&mut self) -> Result<AssertSchema, String> {
        self.expect('{')?;
        let mut fields = Vec::new();

        loop {
            self.skip_ws_and_commas();
            if self.eat('}') {
                break;
            }
            if self.consume_ellipsis() {
                self.skip_ws_and_commas();
                if self.eat('}') {
                    break;
                }
                continue;
            }

            let name = self.parse_key()?;
            self.skip_ws();
            let optional = self.eat('?');
            self.skip_ws();
            self.expect(':')?;
            let schema = self.parse_union()?;
            fields.push(ObjectField {
                name,
                optional,
                schema,
            });
            self.skip_ws_and_commas();
            if self.eat('}') {
                break;
            }
        }

        Ok(AssertSchema::Object(fields))
    }

    fn parse_array(&mut self) -> Result<AssertSchema, String> {
        self.expect('[')?;
        self.skip_ws();
        if self.eat(']') {
            return Ok(AssertSchema::Array(Box::new(AssertSchema::Unknown)));
        }

        let item = self.parse_union()?;
        self.skip_ws_and_commas();
        self.expect(']')?;
        Ok(AssertSchema::Array(Box::new(item)))
    }

    fn parse_key(&mut self) -> Result<String, String> {
        self.skip_ws();
        match self.peek() {
            Some('"') => self.parse_string(),
            Some(ch) if is_ident_start(ch) => Ok(self.parse_identifier()),
            Some(ch) => Err(format!("expected object field name, got '{ch}'")),
            None => Err("expected object field name, got end of schema".to_string()),
        }
    }

    fn parse_named_type_or_literal(&mut self) -> Result<AssertSchema, String> {
        let ident = self.parse_identifier();
        match ident.as_str() {
            "unknown" | "any" => Ok(AssertSchema::Unknown),
            "null" => Ok(AssertSchema::Null),
            "bool" | "boolean" => Ok(AssertSchema::Bool),
            "number" => Ok(AssertSchema::Number),
            "email" => Ok(AssertSchema::Email),
            "true" => Ok(AssertSchema::Literal(Value::Bool(true))),
            "false" => Ok(AssertSchema::Literal(Value::Bool(false))),
            "string" => {
                let (min, max) = self.parse_optional_range()?;
                Ok(AssertSchema::String { min, max })
            }
            other => Err(format!("unknown assert type '{other}'")),
        }
    }

    fn parse_number_literal(&mut self) -> Result<AssertSchema, String> {
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
            self.pos += 1;
        }
        if self.peek() == Some('.') {
            self.pos += 1;
            while matches!(self.peek(), Some(ch) if ch.is_ascii_digit()) {
                self.pos += 1;
            }
        }
        let raw = self.slice(start, self.pos);
        let number = raw
            .parse::<f64>()
            .ok()
            .and_then(Number::from_f64)
            .ok_or_else(|| format!("invalid number literal '{raw}'"))?;
        Ok(AssertSchema::Literal(Value::Number(number)))
    }

    fn parse_optional_range(&mut self) -> Result<(Option<usize>, Option<usize>), String> {
        self.skip_ws();
        if !self.eat('(') {
            return Ok((None, None));
        }

        self.skip_ws();
        let start = self.pos;
        while !self.is_eof() && self.peek() != Some(')') {
            self.pos += 1;
        }
        let raw = self.slice(start, self.pos).trim().to_string();
        self.expect(')')?;

        let parts: Vec<_> = raw.split("..").map(str::trim).collect();
        match parts.as_slice() {
            [single] if !single.is_empty() => {
                let value = single
                    .parse::<usize>()
                    .map_err(|_| format!("invalid string range '{raw}'"))?;
                Ok((Some(value), Some(value)))
            }
            [min, max] => {
                let min = if min.is_empty() {
                    None
                } else {
                    Some(
                        min.parse::<usize>()
                            .map_err(|_| format!("invalid string range '{raw}'"))?,
                    )
                };
                let max = if max.is_empty() {
                    None
                } else {
                    Some(
                        max.parse::<usize>()
                            .map_err(|_| format!("invalid string range '{raw}'"))?,
                    )
                };
                Ok((min, max))
            }
            _ => Err(format!("invalid string range '{raw}'")),
        }
    }

    fn parse_identifier(&mut self) -> String {
        let start = self.pos;
        self.pos += 1;
        while matches!(self.peek(), Some(ch) if is_ident_continue(ch)) {
            self.pos += 1;
        }
        self.slice(start, self.pos)
    }

    fn parse_string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            self.pos += 1;
            match ch {
                '"' => return Ok(out),
                '\\' => {
                    let escaped = self
                        .peek()
                        .ok_or_else(|| "unterminated string escape".to_string())?;
                    self.pos += 1;
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        other => return Err(format!("unsupported string escape '\\{other}'")),
                    }
                }
                other => out.push(other),
            }
        }
        Err("unterminated string literal".to_string())
    }

    fn consume_ellipsis(&mut self) -> bool {
        if self.chars.get(self.pos..self.pos + 3) == Some(&['.', '.', '.']) {
            self.pos += 3;
            true
        } else {
            false
        }
    }

    fn skip_ws_and_commas(&mut self) {
        loop {
            self.skip_ws();
            if !self.eat(',') {
                break;
            }
        }
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
            self.pos += 1;
        }
    }

    fn expect(&mut self, expected: char) -> Result<(), String> {
        self.skip_ws();
        if self.eat(expected) {
            Ok(())
        } else {
            Err(format!(
                "expected '{expected}', got '{}'",
                self.peek()
                    .map(|ch| ch.to_string())
                    .unwrap_or_else(|| "end of schema".to_string())
            ))
        }
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.chars.len()
    }

    fn slice(&self, start: usize, end: usize) -> String {
        self.input.chars().skip(start).take(end - start).collect()
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::Middleware;

    struct StubInvoker;
    #[async_trait::async_trait]
    impl crate::executor::MiddlewareInvoker for StubInvoker {
        async fn call(
            &self,
            _name: &str,
            _args: &[String],
            _cfg: &str,
            _pipeline_ctx: &mut crate::runtime::PipelineContext,
            _env: &crate::executor::ExecutionEnv,
            _ctx: &mut crate::executor::RequestContext,
            _target_name: Option<&str>,
        ) -> Result<crate::middleware::MiddlewareOutput, WebPipeError> {
            Ok(crate::middleware::MiddlewareOutput::default())
        }
    }

    fn dummy_env() -> crate::executor::ExecutionEnv {
        use crate::executor::ExecutionEnv;
        use crate::runtime::context::{CacheStore, RateLimitStore};
        use std::collections::HashMap;
        use std::sync::Arc;

        ExecutionEnv {
            variables: Arc::new(HashMap::new()),
            named_pipelines: Arc::new(HashMap::new()),
            imports: Arc::new(std::collections::HashMap::new()),
            invoker: Arc::new(StubInvoker),
            registry: Arc::new(crate::middleware::MiddlewareRegistry::empty()),
            environment: None,
            cache: CacheStore::new(8, 60),
            rate_limit: RateLimitStore::new(1000),
            module_registry: Arc::new(crate::executor::ModuleRegistry::new()),
            debugger: None,
        }
    }

    #[tokio::test]
    async fn assert_refines_object_contract_without_changing_state() {
        let mw = AssertMiddleware;
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let cfg = r#"{
            data: {
                rows: [{ id: string, name: string }],
                rowCount: number
            }
        }"#;
        let original = serde_json::json!({
            "data": {
                "rows": [{ "id": "1", "name": "webpipe" }],
                "rowCount": 1
            },
            "params": { "id": "1" }
        });
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(original.clone());

        mw.execute(&[], cfg, &mut pipeline_ctx, &env, &mut ctx, None)
            .await
            .unwrap();

        assert_eq!(pipeline_ctx.state, original);
    }

    #[tokio::test]
    async fn assert_failure_emits_assertion_error_envelope() {
        let mw = AssertMiddleware;
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({
            "data": { "rows": [] }
        }));

        mw.execute(
            &[],
            "{ data: { rows: [{ id: string }], rowCount: number } }",
            &mut pipeline_ctx,
            &env,
            &mut ctx,
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            pipeline_ctx.state["errors"][0]["type"],
            serde_json::json!("assertionError")
        );
        assert_eq!(
            pipeline_ctx.state["errors"][0]["field"],
            serde_json::json!("data.rowCount")
        );
        assert_eq!(
            pipeline_ctx.state["errors"][0]["rule"],
            serde_json::json!("assert")
        );
    }

    #[tokio::test]
    async fn assert_supports_optional_nullable_arrays_and_literals() {
        let mw = AssertMiddleware;
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({
            "type": "ready",
            "name": null,
            "roles": ["admin", "editor"]
        }));

        mw.execute(
            &[],
            r#"{ type: "ready", email?: string, name: string | null, roles: [string] }"#,
            &mut pipeline_ctx,
            &env,
            &mut ctx,
            None,
        )
        .await
        .unwrap();

        assert!(pipeline_ctx.state.get("errors").is_none());
    }
}
