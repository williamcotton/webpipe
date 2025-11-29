use serde_json::Value;

/// Parses a selector string like "data.rows" or ".data.rows"
/// 
/// Supports:
/// - `data.rows` (no leading dot)
/// - `.data.rows` (jq-style with leading dot)
/// - `data.users.0` (array index access)
pub fn parse_path(path: &str) -> Vec<&str> {
    // 1. Trim leading dot if present (compatibility with JQ-style habits)
    let clean = path.trim_start_matches('.');
    
    // 2. Split by dots.
    // This supports "data.rows" and "users.0.id"
    clean.split('.').filter(|s| !s.is_empty()).collect()
}

/// Mutable access to a nested value (the engine for foreach)
/// 
/// Returns a mutable reference to the value at the given path, or None if the path
/// doesn't exist or traverses through a non-container type.
pub fn get_value_mut<'a>(root: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let parts = parse_path(path);
    let mut current = root;
    
    for part in parts {
        current = match current {
            Value::Object(map) => map.get_mut(part)?,
            Value::Array(arr) => {
                // Support "users.0" index syntax
                let idx = part.parse::<usize>().ok()?;
                arr.get_mut(idx)?
            },
            _ => return None,
        };
    }
    Some(current)
}

/// Immutable access to a nested value
/// 
/// Returns a reference to the value at the given path, or None if the path
/// doesn't exist or traverses through a non-container type.
pub fn get_value<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let parts = parse_path(path);
    let mut current = root;
    
    for part in parts {
        current = match current {
            Value::Object(map) => map.get(part)?,
            Value::Array(arr) => {
                // Support "users.0" index syntax
                let idx = part.parse::<usize>().ok()?;
                arr.get(idx)?
            },
            _ => return None,
        };
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_path_simple() {
        assert_eq!(parse_path("data"), vec!["data"]);
        assert_eq!(parse_path("data.rows"), vec!["data", "rows"]);
        assert_eq!(parse_path("a.b.c.d"), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn parse_path_with_leading_dot() {
        assert_eq!(parse_path(".data"), vec!["data"]);
        assert_eq!(parse_path(".data.rows"), vec!["data", "rows"]);
    }

    #[test]
    fn parse_path_with_array_index() {
        assert_eq!(parse_path("users.0"), vec!["users", "0"]);
        assert_eq!(parse_path("data.rows.0.id"), vec!["data", "rows", "0", "id"]);
    }

    #[test]
    fn parse_path_empty() {
        assert_eq!(parse_path(""), Vec::<&str>::new());
        assert_eq!(parse_path("."), Vec::<&str>::new());
    }

    #[test]
    fn get_value_simple() {
        let data = json!({
            "data": {
                "rows": [1, 2, 3]
            }
        });
        
        assert_eq!(get_value(&data, "data.rows"), Some(&json!([1, 2, 3])));
        assert_eq!(get_value(&data, ".data.rows"), Some(&json!([1, 2, 3])));
    }

    #[test]
    fn get_value_with_array_index() {
        let data = json!({
            "users": [
                {"name": "Alice"},
                {"name": "Bob"}
            ]
        });
        
        assert_eq!(get_value(&data, "users.0.name"), Some(&json!("Alice")));
        assert_eq!(get_value(&data, "users.1.name"), Some(&json!("Bob")));
    }

    #[test]
    fn get_value_not_found() {
        let data = json!({"a": 1});
        
        assert_eq!(get_value(&data, "b"), None);
        assert_eq!(get_value(&data, "a.b"), None);
    }

    #[test]
    fn get_value_mut_and_modify() {
        let mut data = json!({
            "data": {
                "rows": [1, 2, 3]
            }
        });
        
        if let Some(rows) = get_value_mut(&mut data, "data.rows") {
            *rows = json!([4, 5, 6]);
        }
        
        assert_eq!(data, json!({
            "data": {
                "rows": [4, 5, 6]
            }
        }));
    }

    #[test]
    fn get_value_mut_take_and_replace() {
        let mut data = json!({
            "data": {
                "rows": [1, 2, 3]
            }
        });
        
        // Simulate the surgical extraction pattern
        let taken = get_value_mut(&mut data, "data.rows")
            .map(|v| v.take());
        
        assert_eq!(taken, Some(json!([1, 2, 3])));
        // After take(), the slot contains null
        assert_eq!(data, json!({
            "data": {
                "rows": null
            }
        }));
        
        // Implant new value
        if let Some(slot) = get_value_mut(&mut data, "data.rows") {
            *slot = json!([10, 20, 30]);
        }
        
        assert_eq!(data, json!({
            "data": {
                "rows": [10, 20, 30]
            }
        }));
    }

    #[test]
    fn get_value_mut_array_index() {
        let mut data = json!({
            "users": [
                {"name": "Alice", "score": 10},
                {"name": "Bob", "score": 20}
            ]
        });
        
        if let Some(first_user) = get_value_mut(&mut data, "users.0") {
            if let Some(obj) = first_user.as_object_mut() {
                obj.insert("processed".to_string(), json!(true));
            }
        }
        
        assert_eq!(data["users"][0]["processed"], json!(true));
    }
}

