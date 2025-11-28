use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use reqwest::{self, Method};
use reqwest::header::{HeaderMap as ReqwestHeaderMap, HeaderName, HeaderValue, USER_AGENT};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
pub struct FetchMiddleware { pub(crate) ctx: Arc<Context> }

fn build_fetch_error_object(error_type: &str, mut details: Value) -> Value {
    if !details.is_object() { details = serde_json::json!({ "message": details.to_string() }); }
    let mut err_obj = serde_json::Map::new();
    err_obj.insert("type".to_string(), Value::String(error_type.to_string()));
    if let Some(map) = details.as_object() {
        for (k, v) in map { err_obj.insert(k.clone(), v.clone()); }
    }
    Value::Object([ ("errors".to_string(), Value::Array(vec![Value::Object(err_obj)])) ].into_iter().collect())
}

#[async_trait]
impl super::Middleware for FetchMiddleware {
    async fn execute(
        &self,
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
    ) -> Result<(), WebPipeError> {
        // Extract values we need from state before mutating
        let url = pipeline_ctx.state.get("fetchUrl").and_then(|v| v.as_str()).unwrap_or(config).trim().to_string();
        if url.is_empty() {
            pipeline_ctx.state = build_fetch_error_object("networkError", serde_json::json!({ "message": "Missing URL for fetch middleware" }));
            return Ok(());
        }

        let method_str = pipeline_ctx.state.get("fetchMethod").and_then(|v| v.as_str()).unwrap_or("GET");
        let method = Method::from_bytes(method_str.as_bytes()).unwrap_or(Method::GET);

        let mut req_builder = self.ctx.http.request(method, &url);
        if let Some(timeout_secs) = pipeline_ctx.state.get("fetchTimeout").and_then(|v| v.as_u64()) {
            req_builder = req_builder.timeout(Duration::from_secs(timeout_secs));
        }

        let mut headers = ReqwestHeaderMap::new();
        if let Some(headers_obj) = pipeline_ctx.state.get("fetchHeaders").and_then(|v| v.as_object()) {
            for (k, v) in headers_obj {
                if let Some(val_str) = v.as_str() {
                    if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(val_str)) {
                        headers.insert(name, value);
                    }
                }
            }
        }
        if !headers.contains_key(USER_AGENT) { headers.insert(USER_AGENT, HeaderValue::from_static("WebPipe/1.0")); }
        req_builder = req_builder.headers(headers);

        if let Some(body) = pipeline_ctx.state.get("fetchBody") { req_builder = req_builder.json(body); }

        let result_name = pipeline_ctx.state.get("resultName").and_then(|v| v.as_str()).map(|s| s.to_string());

        let response = match req_builder.send().await {
            Ok(resp) => resp,
            Err(err) => {
                if err.is_timeout() {
                    pipeline_ctx.state = build_fetch_error_object("timeoutError", serde_json::json!({ "message": err.to_string() }));
                } else {
                    pipeline_ctx.state = build_fetch_error_object("networkError", serde_json::json!({ "message": err.to_string(), "url": url }));
                }
                return Ok(());
            }
        };

        let status = response.status();
        let status_code = status.as_u16();
        let headers_map: HashMap<String, String> = response.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())).collect();

        let body_text = match response.text().await {
            Ok(t) => t,
            Err(err) => {
                pipeline_ctx.state = build_fetch_error_object("networkError", serde_json::json!({ "message": format!("Failed to read response body: {}", err), "url": url }));
                return Ok(());
            }
        };

        if !status.is_success() {
            pipeline_ctx.state = build_fetch_error_object("httpError", serde_json::json!({ "status": status_code, "message": body_text, "url": url }));
            return Ok(());
        }

        let response_body: Value = serde_json::from_str(&body_text).unwrap_or(Value::String(body_text));

        let result_payload = serde_json::json!({ "response": response_body, "status": status_code, "headers": headers_map });

        // Mutate state in place
        if let Some(obj) = pipeline_ctx.state.as_object_mut() {
            match result_name.as_deref() {
                Some(name) => {
                    let data_entry = obj.entry("data").or_insert_with(|| Value::Object(serde_json::Map::new()));
                    if !data_entry.is_object() { *data_entry = Value::Object(serde_json::Map::new()); }
                    if let Some(data_obj) = data_entry.as_object_mut() { data_obj.insert(name.to_string(), result_payload); }
                }
                None => { obj.insert("data".to_string(), result_payload); }
            }
        }

        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Context;
    use crate::runtime::context::{CacheStore, ConfigSnapshot};
    use crate::middleware::Middleware;
    use reqwest::Client;
    use std::time::Duration;

    fn ctx_with_client() -> Arc<Context> {
        Arc::new(Context {
            pg: None,
            http: Client::builder().timeout(Duration::from_secs(1)).build().unwrap(),
            cache: CacheStore::new(64, 1),
            rate_limit: crate::runtime::context::RateLimitStore::new(1000),
            hb: std::sync::Arc::new(parking_lot::Mutex::new(handlebars::Handlebars::new())),
            cfg: ConfigSnapshot(serde_json::json!({})),
            lua_scripts: std::sync::Arc::new(std::collections::HashMap::new()),
            graphql: None,
            execution_env: Arc::new(parking_lot::RwLock::new(None)),
        })
    }

    struct StubInvoker;
    #[async_trait::async_trait]
    impl crate::executor::MiddlewareInvoker for StubInvoker {
        async fn call(
            &self,
            _name: &str,
            _cfg: &str,
            _pipeline_ctx: &mut crate::runtime::PipelineContext,
            _env: &crate::executor::ExecutionEnv,
            _ctx: &mut crate::executor::RequestContext,
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

    #[tokio::test]
    async fn missing_url_yields_network_error_object() {
        let mw = FetchMiddleware { ctx: ctx_with_client() };
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({}));
        mw.execute("", &mut pipeline_ctx, &env, &mut ctx).await.unwrap();
        assert_eq!(pipeline_ctx.state["errors"][0]["type"], serde_json::json!("networkError"));
    }
}

