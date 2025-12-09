use crate::error::WebPipeError;
use serde_json::Value;
use std::cell::RefCell;
use lru::LruCache;
use std::num::NonZeroUsize;

// Thread-local cache for JQ programs since jq_rs::JqProgram is not Send + Sync
thread_local! {
    static JQ_PROGRAM_CACHE: RefCell<LruCache<String, jq_rs::JqProgram>> = RefCell::new(
        LruCache::new(NonZeroUsize::new(100).unwrap())
    );
}

/// Evaluate a JQ expression against a JSON value and return the result.
///
/// This is the shared runtime function used by all middleware that need to evaluate
/// JQ expressions. It uses a thread-local cache for optimal performance.
///
/// # Performance
/// - Cache Hit: O(1) program lookup + O(n) JQ execution
/// - Cache Miss: O(n) compilation + O(n) execution, then cached for future use
///
/// # Arguments
/// * `expr` - The JQ expression to evaluate
/// * `input` - The JSON value to use as input to the JQ expression
///
/// # Returns
/// The result of evaluating the JQ expression as a JSON value
pub fn evaluate(expr: &str, input: &Value) -> Result<Value, WebPipeError> {
    JQ_PROGRAM_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();

        // Check cache first
        if let Some(program) = cache.get_mut(expr) {
            return program
                .run_json(input)
                .map_err(|e| WebPipeError::MiddlewareExecutionError(
                    format!("JQ execution error: {}", e)
                ));
        }

        // Cache miss - compile the program
        match jq_rs::compile(expr) {
            Ok(mut program) => {
                let result = program
                    .run_json(input)
                    .map_err(|e| WebPipeError::MiddlewareExecutionError(
                        format!("JQ execution error: {}", e)
                    ))?;

                // Store in cache for future use
                cache.put(expr.to_string(), program);

                Ok(result)
            }
            Err(e) => Err(WebPipeError::MiddlewareExecutionError(
                format!("JQ compilation error: {}", e)
            )),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_evaluate_simple_expression() {
        let input = json!({"a": 1, "b": 2});
        let result = evaluate(".a", &input).unwrap();
        assert_eq!(result, json!(1));
    }

    #[test]
    fn test_evaluate_complex_expression() {
        let input = json!({"items": [1, 2, 3]});
        let result = evaluate(".items | map(. * 2)", &input).unwrap();
        assert_eq!(result, json!([2, 4, 6]));
    }

    #[test]
    fn test_evaluate_caching() {
        let input = json!({"x": 10});
        // First call - will compile and cache
        let result1 = evaluate(".x + 5", &input).unwrap();
        assert_eq!(result1, json!(15));

        // Second call - should hit cache
        let result2 = evaluate(".x + 5", &input).unwrap();
        assert_eq!(result2, json!(15));
    }

    #[test]
    fn test_evaluate_compilation_error() {
        let input = json!({});
        let result = evaluate(".[] | ", &input);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("JQ"));
    }
}
