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
        let req_clone = obj.clone();
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



#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, Method};
    use std::collections::HashMap;

    #[test]
    fn test_string_to_number_or_string_variants() {
        assert_eq!(string_to_number_or_string("42"), serde_json::json!(42));
        // f64 may have precision, but 3.14 should parse
        assert_eq!(string_to_number_or_string("3.1").as_f64().unwrap(), 3.1);
        assert_eq!(string_to_number_or_string("foo"), serde_json::json!("foo"));
    }

    #[test]
    fn test_string_map_to_json_with_number_coercion() {
        let mut m = HashMap::new();
        m.insert("a".to_string(), "1".to_string());
        m.insert("b".to_string(), "1.5".to_string());
        m.insert("c".to_string(), "x".to_string());
        let v = string_map_to_json_with_number_coercion(&m);
        assert_eq!(v["a"], serde_json::json!(1));
        assert_eq!(v["b"].as_f64().unwrap(), 1.5);
        assert_eq!(v["c"], serde_json::json!("x"));
    }

    #[test]
    fn test_parse_cookies_from_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", "a=1; b=two; c=".parse().unwrap());
        let cookies = parse_cookies_from_headers(&headers);
        assert_eq!(cookies.get("a").unwrap(), "1");
        assert_eq!(cookies.get("b").unwrap(), "two");
        assert_eq!(cookies.get("c").unwrap(), "");
    }

    #[test]
    fn test_parse_body_from_content_type() {
        // empty -> {}
        let empty = Bytes::from_static(b"");
        assert_eq!(parse_body_from_content_type(&empty, "application/json"), serde_json::json!({}));

        // json valid
        let body = Bytes::from_static(br#"{"x":1}"#);
        assert_eq!(parse_body_from_content_type(&body, "application/json"), serde_json::json!({"x":1}));

        // json invalid -> Null
        let body = Bytes::from_static(br#"{invalid}"#);
        assert_eq!(parse_body_from_content_type(&body, "application/json"), serde_json::Value::Null);

        // form
        let body = Bytes::from_static(b"a=1&b=two&c=&d=3.5");
        let v = parse_body_from_content_type(&body, "application/x-www-form-urlencoded");
        assert_eq!(v["a"], serde_json::json!(1));
        assert_eq!(v["b"], serde_json::json!("two"));
        assert_eq!(v["c"], serde_json::json!(""));
        assert_eq!(v["d"].as_f64().unwrap(), 3.5);

        // other -> raw string
        let body = Bytes::from_static(b"hello");
        assert_eq!(parse_body_from_content_type(&body, "text/plain"), serde_json::json!("hello"));
    }

    #[test]
    fn test_build_request_from_axum_defaults_and_snapshot() {
        let method = Method::POST;
        let mut headers = HeaderMap::new();
        // Intentionally omit content-type to trigger default
        headers.insert("cookie", "sid=abc".parse().unwrap());
        let mut path_params = HashMap::new();
        path_params.insert("id".to_string(), "42".to_string());
        let mut query_params = HashMap::new();
        query_params.insert("q".to_string(), "term".to_string());
        let body = Bytes::from_static(br#"{"a":1}"#);

        let (req, ct) = build_request_from_axum(&method, &headers, &path_params, &query_params, &body);
        assert_eq!(ct, "application/json");
        assert_eq!(req["method"], serde_json::json!("POST"));
        assert_eq!(req["cookies"]["sid"], serde_json::json!("abc"));
        assert_eq!(req["originalRequest"]["method"], serde_json::json!("POST"));
        assert_eq!(req["originalRequest"]["params"]["id"], serde_json::json!(42));
        assert_eq!(req["originalRequest"]["query"]["q"], serde_json::json!("term"));
    }

    #[test]
    fn test_build_minimal_request_for_tests_shape() {
        let mut path_params = HashMap::new();
        path_params.insert("id".to_string(), "7".to_string());
        let mut query_params = HashMap::new();
        query_params.insert("q".to_string(), "a".to_string());
        let v = build_minimal_request_for_tests("GET", "/u/7", &path_params, &query_params);
        assert_eq!(v["method"], serde_json::json!("GET"));
        assert_eq!(v["content_type"], serde_json::json!("application/json"));
        assert_eq!(v["originalRequest"]["params"]["id"], serde_json::json!(7));
        assert_eq!(v["originalRequest"]["query"]["q"], serde_json::json!("a"));
    }
}

