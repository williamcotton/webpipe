use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use reqwest::{self, Method};
use reqwest::header::{HeaderMap as ReqwestHeaderMap, HeaderName, HeaderValue, USER_AGENT};
use serde_json::Value;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug)]
pub struct FetchMiddleware { pub(crate) ctx: Arc<Context> }

fn lookup_path_string(input: &Value, path: &str) -> String {
    let mut cur = input;
    for seg in path.split('.') {
        if seg.is_empty() { continue; }
        match cur {
            Value::Object(map) => { cur = map.get(seg).unwrap_or(&Value::Null); }
            _ => { cur = &Value::Null; }
        }
    }
    match cur {
        Value::Null => "".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn render_key_template(template: &str, input: &Value) -> String {
    let mut out = String::with_capacity(template.len());
    let mut i = 0; let chars: Vec<char> = template.chars().collect();
    while i < chars.len() {
        if chars[i] == '{' {
            let mut j = i + 1; while j < chars.len() && chars[j] != '}' { j += 1; }
            if j < chars.len() && chars[j] == '}' {
                let key: String = chars[i+1..j].iter().collect();
                let val = lookup_path_string(input, key.trim());
                out.push_str(&val); i = j + 1; continue;
            }
        }
        out.push(chars[i]); i += 1;
    }
    out
}

fn cache_enabled_and_ttl(input: &Value) -> (bool, u64, Option<String>) {
    let meta = input.get("_metadata").and_then(|m| m.get("cache"));
    let enabled = meta.and_then(|m| m.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(true);
    let ttl = meta.and_then(|m| m.get("ttl")).and_then(|v| v.as_u64()).unwrap_or(60);
    let key_tpl = meta.and_then(|m| m.get("keyTemplate")).and_then(|v| v.as_str()).map(|s| s.to_string());
    (enabled, ttl, key_tpl)
}

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
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        let url = input.get("fetchUrl").and_then(|v| v.as_str()).unwrap_or(config).trim().to_string();
        if url.is_empty() {
            return Ok(build_fetch_error_object("networkError", serde_json::json!({ "message": "Missing URL for fetch middleware" })));
        }

        let method_str = input.get("fetchMethod").and_then(|v| v.as_str()).unwrap_or("GET");
        let method = Method::from_bytes(method_str.as_bytes()).unwrap_or(Method::GET);

        let mut req_builder = self.ctx.http.request(method, &url);
        if let Some(timeout_secs) = input.get("fetchTimeout").and_then(|v| v.as_u64()) {
            req_builder = req_builder.timeout(Duration::from_secs(timeout_secs));
        }

        let mut headers = ReqwestHeaderMap::new();
        if let Some(headers_obj) = input.get("fetchHeaders").and_then(|v| v.as_object()) {
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

        if let Some(body) = input.get("fetchBody") { req_builder = req_builder.json(body); }

        let (cache_enabled, cache_ttl, key_template_opt) = cache_enabled_and_ttl(input);
        let cache_key = if let Some(tpl) = key_template_opt.as_deref() {
            Some(render_key_template(tpl, input))
        } else {
            let mut hasher = DefaultHasher::new();
            method_str.hash(&mut hasher); url.hash(&mut hasher);
            if let Some(h) = input.get("fetchHeaders") { if let Ok(s) = serde_json::to_string(h) { s.hash(&mut hasher); } }
            if let Some(b) = input.get("fetchBody") { if let Ok(s) = serde_json::to_string(b) { s.hash(&mut hasher); } }
            Some(format!("fetch:{:x}", hasher.finish()))
        };

        if cache_enabled {
            if let Some(key) = cache_key.as_ref() {
                if let Some(cached_payload) = self.ctx.cache.get(key) {
                    let mut output = input.clone();
                    let result_name_opt = input.get("resultName").and_then(|v| v.as_str());
                    match result_name_opt {
                        Some(name) => {
                            if let Some(obj) = output.as_object_mut() {
                                let data_entry = obj.entry("data").or_insert_with(|| Value::Object(serde_json::Map::new()));
                                if !data_entry.is_object() { *data_entry = Value::Object(serde_json::Map::new()); }
                                if let Some(data_obj) = data_entry.as_object_mut() { data_obj.insert(name.to_string(), cached_payload); }
                            }
                        }
                        None => { if let Some(obj) = output.as_object_mut() { obj.insert("data".to_string(), cached_payload); } }
                    }
                    return Ok(output);
                }
            }
        }

        let response = match req_builder.send().await {
            Ok(resp) => resp,
            Err(err) => {
                if err.is_timeout() {
                    return Ok(build_fetch_error_object("timeoutError", serde_json::json!({ "message": err.to_string() })));
                } else {
                    return Ok(build_fetch_error_object("networkError", serde_json::json!({ "message": err.to_string(), "url": url })));
                }
            }
        };

        let status = response.status();
        let status_code = status.as_u16();
        let headers_map: HashMap<String, String> = response.headers().iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())).collect();

        let body_text = match response.text().await {
            Ok(t) => t,
            Err(err) => {
                return Ok(build_fetch_error_object("networkError", serde_json::json!({ "message": format!("Failed to read response body: {}", err), "url": url })));
            }
        };

        if !status.is_success() {
            return Ok(build_fetch_error_object("httpError", serde_json::json!({ "status": status_code, "message": body_text, "url": url })));
        }

        let response_body: Value = serde_json::from_str(&body_text).unwrap_or(Value::String(body_text));

        let mut output = input.clone();
        let result_name_opt = input.get("resultName").and_then(|v| v.as_str());
        let result_payload = serde_json::json!({ "response": response_body, "status": status_code, "headers": headers_map });

        match result_name_opt {
            Some(name) => {
                if let Some(obj) = output.as_object_mut() {
                    let data_entry = obj.entry("data").or_insert_with(|| Value::Object(serde_json::Map::new()));
                    if !data_entry.is_object() { *data_entry = Value::Object(serde_json::Map::new()); }
                    if let Some(data_obj) = data_entry.as_object_mut() { data_obj.insert(name.to_string(), result_payload.clone()); }
                }
            }
            None => { if let Some(obj) = output.as_object_mut() { obj.insert("data".to_string(), result_payload.clone()); } }
        }

        if cache_enabled { if let Some(key) = cache_key { self.ctx.cache.put(key, result_payload, Some(cache_ttl)); } }

        Ok(output)
    }
}


