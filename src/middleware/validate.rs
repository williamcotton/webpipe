use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::Value;

#[derive(Debug)]
pub struct ValidateMiddleware;

#[async_trait]
impl super::Middleware for ValidateMiddleware {
    async fn execute(
        &self,
        _args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<(), WebPipeError> {
        let input = &pipeline_ctx.state;
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
        if rules.is_empty() { return Ok(()); }

        let body = input.get("body");
        let obj = match body.and_then(|b| b.as_object()) { Some(o) => o, None => { return Ok(()); } };

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
                        _ => {
                            let msg = format!("Field '{}' is required and must be a non-empty string", rule.field);
                            pipeline_ctx.state = make_error(&rule.field, "required", msg);
                            return Ok(());
                        }
                    };
                    let len = s.chars().count();
                    if let Some(min) = rule.min {
                        if len < min {
                            pipeline_ctx.state = make_error(&rule.field, "minLength", format!("Field '{}' must be at least {} characters", rule.field, min));
                            return Ok(());
                        }
                    }
                    if let Some(max) = rule.max {
                        if len > max {
                            pipeline_ctx.state = make_error(&rule.field, "maxLength", format!("Field '{}' must be at most {} characters", rule.field, max));
                            return Ok(());
                        }
                    }
                }
                "email" => {
                    let s = match val.and_then(|v| v.as_str()) {
                        Some(v) if !v.is_empty() => v,
                        _ => {
                            let msg = format!("Field '{}' is required and must be a valid email", rule.field);
                            pipeline_ctx.state = make_error(&rule.field, "required", msg);
                            return Ok(());
                        }
                    };
                    let valid = s.contains('@') && s.contains('.') && !s.starts_with('@') && !s.ends_with('@');
                    if !valid {
                        pipeline_ctx.state = make_error(&rule.field, "email", format!("Field '{}' must be a valid email address", rule.field));
                        return Ok(());
                    }
                }
                _ => {}
            }
        }

        // Validation passed - state unchanged
        Ok(())
    }
}


#[cfg(test)]
mod tests {
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
        ) -> Result<(), WebPipeError> {
            Ok(())
        }
    }
    fn dummy_env() -> crate::executor::ExecutionEnv {
        use crate::executor::ExecutionEnv;
        use crate::runtime::context::{CacheStore, RateLimitStore};
        use std::sync::Arc;
        use std::collections::HashMap;

        let registry = Arc::new(crate::middleware::MiddlewareRegistry::empty());
        ExecutionEnv {
            variables: Arc::new(vec![]),
            named_pipelines: Arc::new(HashMap::new()),
            invoker: Arc::new(StubInvoker),
            registry,
            environment: None,
            cache: CacheStore::new(8, 60),
            rate_limit: RateLimitStore::new(1000),
        }
    }
    use super::*;
    use crate::middleware::Middleware;

    #[tokio::test]
    async fn string_min_max_and_missing() {
        let mw = ValidateMiddleware;
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let cfg = "{ name: string(2..4) }";
        // missing
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({"body": {}}));
        mw.execute(&[], cfg, &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx.state["errors"][0]["type"], serde_json::json!("validationError"));
        // too short
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({"body": {"name": "a"}}));
        mw.execute(&[], cfg, &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx.state["errors"][0]["rule"], serde_json::json!("minLength"));
        // too long
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({"body": {"name": "abcde"}}));
        mw.execute(&[], cfg, &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx.state["errors"][0]["rule"], serde_json::json!("maxLength"));
        // ok
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({"body": {"name": "abc"}}));
        mw.execute(&[], cfg, &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert!(pipeline_ctx.state.get("errors").is_none());
    }

    #[tokio::test]
    async fn email_rule_accepts_and_rejects() {
        let mw = ValidateMiddleware;
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let cfg = "{ email: email }";
        // missing
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({"body": {}}));
        mw.execute(&[], cfg, &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx.state["errors"][0]["type"], serde_json::json!("validationError"));
        // invalid
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({"body": {"email": "foo"}}));
        mw.execute(&[], cfg, &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert_eq!(pipeline_ctx.state["errors"][0]["rule"], serde_json::json!("email"));
        // valid
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({"body": {"email": "a@b.com"}}));
        mw.execute(&[], cfg, &mut pipeline_ctx, &env, &mut ctx, None).await.unwrap();
        assert!(pipeline_ctx.state.get("errors").is_none());
    }
}

