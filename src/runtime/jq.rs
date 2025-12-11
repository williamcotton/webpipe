use crate::error::WebPipeError;
use serde_json::Value;
use parking_lot::Mutex;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::LazyLock;

use jaq_core::{Compiler, Ctx, RcIter, load::{Arena, File, Loader}};
use jaq_json::Val;

/// Compiled JQ filter with its arena (arena must outlive filter)
struct CompiledFilter {
    #[allow(dead_code)]
    arena: Arena,
    filter: jaq_core::Filter<jaq_core::Native<Val>>,
}

/// Global cache for compiled JQ programs
/// Using LazyLock for thread-safe one-time initialization
static JQ_FILTER_CACHE: LazyLock<Mutex<LruCache<String, CompiledFilter>>> =
    LazyLock::new(|| {
        Mutex::new(LruCache::new(NonZeroUsize::new(100).unwrap()))
    });

/// Compile a JQ expression into a cached Filter
fn compile_filter(expr: &str) -> Result<CompiledFilter, WebPipeError> {
    // Create loader with standard library
    let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));

    // Create arena for this compilation
    let arena = Arena::default();

    // Load and parse the program
    let program = File { code: expr, path: () };
    let modules = loader.load(&arena, program)
        .map_err(|e| WebPipeError::MiddlewareExecutionError(
            format!("JQ parsing error: {:?}", e)
        ))?;

    // Compile with standard functions
    let filter = Compiler::default()
        .with_funs(jaq_std::funs().chain(jaq_json::funs()))
        .compile(modules)
        .map_err(|e| WebPipeError::MiddlewareExecutionError(
            format!("JQ compilation error: {:?}", e)
        ))?;

    Ok(CompiledFilter { arena, filter })
}

/// Evaluate a JQ expression against a JSON value and return the first result.
///
/// This is the shared runtime function used by all middleware that need to evaluate
/// JQ expressions. It uses a global cache with mutex for optimal performance.
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
/// The first result of evaluating the JQ expression as a JSON value.
/// If the filter produces no results, returns `null`.
pub fn evaluate(expr: &str, input: &Value) -> Result<Value, WebPipeError> {
    // Lock cache and check for existing compiled filter
    let mut cache = JQ_FILTER_CACHE.lock();

    // Get or compile filter
    if !cache.contains(expr) {
        let compiled = compile_filter(expr)?;
        cache.put(expr.to_string(), compiled);
    }

    // Get filter from cache (safe unwrap - we just inserted if missing)
    let compiled = cache.get_mut(expr).unwrap();

    // Convert serde_json::Value -> jaq_json::Val
    let jaq_input = Val::from(input.clone());

    // Create execution context with no inputs iterator
    let inputs = RcIter::new(core::iter::empty());
    let ctx = Ctx::new([], &inputs);

    // Execute filter and take first result
    let mut results = compiled.filter.run((ctx, jaq_input));

    let first_result = results.next()
        .unwrap_or_else(|| Ok(Val::Null))
        .map_err(|e| WebPipeError::MiddlewareExecutionError(
            format!("JQ execution error: {}", e)
        ))?;

    // Convert jaq_json::Val -> serde_json::Value
    Ok(Value::from(first_result))
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

    #[test]
    fn test_evaluate_first_result_only() {
        // Test that we only get first result from iterator
        let input = json!([1, 2, 3]);
        let result = evaluate(".[]", &input).unwrap();
        assert_eq!(result, json!(1)); // Only first element
    }

    #[test]
    fn test_evaluate_no_results_returns_null() {
        let input = json!([]);
        let result = evaluate(".[] | select(. > 10)", &input).unwrap();
        assert_eq!(result, json!(null));
    }
}
