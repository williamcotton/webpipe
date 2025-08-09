use axum::body::Bytes;
use axum::http::{HeaderMap, Method};
use serde_json::Value;
use std::collections::HashMap;

fn string_to_number_or_string(s: &str) -> Value {
    if let Ok(i) = s.parse::<i64>() {
        return Value::Number(i.into());
    }
    if let Ok(f) = s.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return Value::Number(n);
        }
    }
    Value::String(s.to_string())
}

fn string_map_to_json_with_number_coercion(map: &HashMap<String, String>) -> Value {
    let mut obj = serde_json::Map::new();
    for (k, v) in map {
        obj.insert(k.clone(), string_to_number_or_string(v));
    }
    Value::Object(obj)
}

fn parse_cookies_from_headers(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .filter_map(|cookie| {
            let mut parts = cookie.trim().split('=');
            let key = parts.next()?.to_string();
            let value = parts.next().unwrap_or("").to_string();
            Some((key, value))
        })
        .collect()
}

fn parse_body_from_content_type(body: &Bytes, content_type: &str) -> Value {
    if body.is_empty() {
        return Value::Object(serde_json::Map::new());
    }
    if content_type.starts_with("application/json") {
        serde_json::from_slice(body).unwrap_or(Value::Null)
    } else if content_type.starts_with("application/x-www-form-urlencoded") {
        let form_str = String::from_utf8_lossy(body);
        let form_data: HashMap<String, String> = form_str
            .split('&')
            .filter_map(|pair| {
                let mut parts = pair.split('=');
                let key = parts.next()?.to_string();
                let value = parts.next().unwrap_or("").to_string();
                Some((key, value))
            })
            .collect();
        string_map_to_json_with_number_coercion(&form_data)
    } else {
        Value::String(String::from_utf8_lossy(body).to_string())
    }
}

pub fn build_request_from_axum(
    method: &Method,
    headers: &HeaderMap,
    path_params: &HashMap<String, String>,
    query_params: &HashMap<String, String>,
    body: &Bytes,
) -> (Value, String) {
    // Convert headers to string map
    let headers_map: HashMap<String, String> = headers
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    // Determine content type
    let content_type = headers_map
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| "application/json".to_string());

    // Parse body
    let parsed_body = parse_body_from_content_type(body, &content_type);

    // Cookies
    let cookies = parse_cookies_from_headers(headers);

    // Build base request object
    let mut req_obj = serde_json::Map::new();
    req_obj.insert("method".to_string(), Value::String(method.to_string()));
    req_obj.insert("path".to_string(), Value::String("".to_string()));
    req_obj.insert(
        "query".to_string(),
        string_map_to_json_with_number_coercion(query_params),
    );
    req_obj.insert(
        "params".to_string(),
        string_map_to_json_with_number_coercion(path_params),
    );
    req_obj.insert(
        "headers".to_string(),
        {
            let mut hmap = serde_json::Map::new();
            for (k, v) in headers_map {
                hmap.insert(k, Value::String(v));
            }
            Value::Object(hmap)
        },
    );
    req_obj.insert(
        "cookies".to_string(),
        {
            let mut cmap = serde_json::Map::new();
            for (k, v) in cookies {
                cmap.insert(k, Value::String(v));
            }
            Value::Object(cmap)
        },
    );
    req_obj.insert("body".to_string(), parsed_body);
    req_obj.insert(
        "content_type".to_string(),
        Value::String(content_type.clone()),
    );

    // Original request snapshot
    if let Some(obj) = Value::Object(req_obj).as_object() {
        let mut req_clone = obj.clone();
        let method_str = req_clone
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut orig = serde_json::Map::new();
        orig.insert("method".to_string(), Value::String(method_str));
        if let Some(params) = req_clone.get("params").cloned() {
            orig.insert("params".to_string(), params);
        }
        if let Some(query) = req_clone.get("query").cloned() {
            orig.insert("query".to_string(), query);
        }
        let mut final_obj = req_clone;
        final_obj.insert("originalRequest".to_string(), Value::Object(orig));
        return (Value::Object(final_obj), content_type);
    }

    (Value::Object(serde_json::Map::new()), content_type)
}

pub fn build_minimal_request_for_tests(
    method: &str,
    path: &str,
    path_params: &HashMap<String, String>,
    query_params: &HashMap<String, String>,
) -> Value {
    let mut request_obj = serde_json::Map::new();
    request_obj.insert("method".to_string(), Value::String(method.to_string()));
    request_obj.insert("path".to_string(), Value::String(path.to_string()));
    request_obj.insert(
        "query".to_string(),
        string_map_to_json_with_number_coercion(query_params),
    );
    request_obj.insert(
        "params".to_string(),
        string_map_to_json_with_number_coercion(path_params),
    );
    request_obj.insert("headers".to_string(), Value::Object(serde_json::Map::new()));
    request_obj.insert("cookies".to_string(), Value::Object(serde_json::Map::new()));
    request_obj.insert("body".to_string(), Value::Object(serde_json::Map::new()));
    request_obj.insert(
        "content_type".to_string(),
        Value::String("application/json".to_string()),
    );

    // originalRequest snapshot
    let mut orig = serde_json::Map::new();
    orig.insert("method".to_string(), Value::String(method.to_string()));
    orig.insert(
        "params".to_string(),
        string_map_to_json_with_number_coercion(path_params),
    );
    orig.insert(
        "query".to_string(),
        string_map_to_json_with_number_coercion(query_params),
    );
    request_obj.insert("originalRequest".to_string(), Value::Object(orig));

    Value::Object(request_obj)
}


