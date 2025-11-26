use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug)]
pub struct ValidateMiddleware;

#[async_trait]
impl super::Middleware for ValidateMiddleware {
    async fn execute(&self, config: &str, input: &Value, _env: &crate::executor::ExecutionEnv) -> Result<Value, WebPipeError> {
        #[derive(Debug)]
        struct Rule { field: String, kind: String, min: Option<usize>, max: Option<usize> }

        fn trim_wrapping_braces(s: &str) -> &str {
            let t = s.trim();
            if t.starts_with('{') && t.ends_with('}') { &t[1..t.len()-1] } else { t }
        }

        fn parse_range(spec: &str) -> (Option<usize>, Option<usize>) {
            let mut min = None; let mut max = None;
            let parts: Vec<&str> = spec.split("..").collect();
            if parts.len() == 2 {
                if let Ok(v) = parts[0].trim().parse::<usize>() { min = Some(v); }
                if let Ok(v) = parts[1].trim().parse::<usize>() { max = Some(v); }
            } else if parts.len() == 1 {
                if let Ok(v) = parts[0].trim().parse::<usize>() { min = Some(v); max = Some(v); }
            }
            (min, max)
        }

        fn parse_rules(cfg: &str) -> Vec<Rule> {
            let body = trim_wrapping_braces(cfg);
            let mut rules: Vec<Rule> = Vec::new();
            for raw_line in body.lines() {
                let line = raw_line.trim().trim_end_matches(',');
                if line.is_empty() { continue; }
                if let Some(colon_pos) = line.find(':') {
                    let field = line[..colon_pos].trim().to_string();
                    let rhs = line[colon_pos + 1..].trim();
                    if rhs.starts_with("string") {
                        let mut min = None; let mut max = None;
                        if let Some(lp) = rhs.find('(') { if let Some(rp) = rhs.rfind(')') { let inside = &rhs[lp + 1..rp]; let (mn, mx) = parse_range(inside); min = mn; max = mx; } }
                        rules.push(Rule { field, kind: "string".to_string(), min, max });
                    } else if rhs.starts_with("email") {
                        rules.push(Rule { field, kind: "email".to_string(), min: None, max: None });
                    } else {
                        continue;
                    }
                }
            }
            rules
        }

        fn make_error(field: &str, rule: &str, message: String) -> Value {
            serde_json::json!({
                "errors": [ { "type": "validationError", "field": field, "context": field, "rule": rule, "message": message } ]
            })
        }

        let rules = parse_rules(config);
        if rules.is_empty() { return Ok(input.clone()); }

        let body = input.get("body");
        let obj = match body.and_then(|b| b.as_object()) { Some(o) => o, None => { return Ok(input.clone()); } };

        for rule in rules {
            let val = obj.get(&rule.field);
            match rule.kind.as_str() {
                "string" => {
                    let s_owned: Option<String> = match val {
                        Some(Value::String(s)) => Some(s.clone()),
                        Some(Value::Number(n)) => Some(n.to_string()),
                        Some(Value::Bool(b)) => Some(b.to_string()),
                        Some(Value::Null) | None => None,
                        Some(_) => None,
                    };
                    let s = match s_owned.as_deref() {
                        Some(v) if !v.is_empty() => v,
                        _ => { let msg = format!("Field '{}' is required and must be a non-empty string", rule.field); return Ok(make_error(&rule.field, "required", msg)); }
                    };
                    let len = s.chars().count();
                    if let Some(min) = rule.min { if len < min { return Ok(make_error(&rule.field, "minLength", format!("Field '{}' must be at least {} characters", rule.field, min))); } }
                    if let Some(max) = rule.max { if len > max { return Ok(make_error(&rule.field, "maxLength", format!("Field '{}' must be at most {} characters", rule.field, max))); } }
                }
                "email" => {
                    let s = match val.and_then(|v| v.as_str()) {
                        Some(v) if !v.is_empty() => v,
                        _ => { let msg = format!("Field '{}' is required and must be a valid email", rule.field); return Ok(make_error(&rule.field, "required", msg)); }
                    };
                    let valid = s.contains('@') && s.contains('.') && !s.starts_with('@') && !s.ends_with('@');
                    if !valid { return Ok(make_error(&rule.field, "email", format!("Field '{}' must be a valid email address", rule.field))); }
                }
                _ => {}
            }
        }

        Ok(input.clone())
    }
}


#[cfg(test)]
mod tests {
    struct StubInvoker;
    #[async_trait::async_trait]
    impl crate::executor::MiddlewareInvoker for StubInvoker {
        async fn call(&self, _name: &str, _cfg: &str, _input: &Value, _env: &crate::executor::ExecutionEnv) -> Result<Value, WebPipeError> {
            Ok(Value::Null)
        }
    }
    fn dummy_env() -> crate::executor::ExecutionEnv {
        use crate::executor::{ExecutionEnv, AsyncTaskRegistry};
        use parking_lot::Mutex;
        use std::sync::Arc;
        use std::collections::HashMap;

        ExecutionEnv {
            variables: Arc::new(vec![]),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
            environment: None,
            async_registry: AsyncTaskRegistry::new(),
            flags: Arc::new(HashMap::new()),
            cache: None,
            deferred: Arc::new(Mutex::new(Vec::new())),
        }
    }
    use super::*;
    use crate::middleware::Middleware;

    #[tokio::test]
    async fn string_min_max_and_missing() {
        let mw = ValidateMiddleware;
        let env = dummy_env();
        let cfg = "{ name: string(2..4) }";
        // missing
        let out = mw.execute(cfg, &serde_json::json!({"body": {}}), &env).await.unwrap();
        assert_eq!(out["errors"][0]["type"], serde_json::json!("validationError"));
        // too short
        let out = mw.execute(cfg, &serde_json::json!({"body": {"name": "a"}}), &env).await.unwrap();
        assert_eq!(out["errors"][0]["rule"], serde_json::json!("minLength"));
        // too long
        let out = mw.execute(cfg, &serde_json::json!({"body": {"name": "abcde"}}), &env).await.unwrap();
        assert_eq!(out["errors"][0]["rule"], serde_json::json!("maxLength"));
        // ok
        let out = mw.execute(cfg, &serde_json::json!({"body": {"name": "abc"}}), &env).await.unwrap();
        assert!(out.get("errors").is_none());
    }

    #[tokio::test]
    async fn email_rule_accepts_and_rejects() {
        let mw = ValidateMiddleware;
        let cfg = "{ email: email }";
        // missing
        let out = mw.execute(cfg, &serde_json::json!({"body": {}}), &dummy_env()).await.unwrap();
        assert_eq!(out["errors"][0]["type"], serde_json::json!("validationError"));
        // invalid
        let out = mw.execute(cfg, &serde_json::json!({"body": {"email": "foo"}}), &dummy_env()).await.unwrap();
        assert_eq!(out["errors"][0]["rule"], serde_json::json!("email"));
        // valid
        let out = mw.execute(cfg, &serde_json::json!({"body": {"email": "a@b.com"}}), &dummy_env()).await.unwrap();
        assert!(out.get("errors").is_none());
    }
}

