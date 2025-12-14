use crate::error::WebPipeError;

use jaq_core::{
    load::{Arena, File, Loader},
    Compiler, Ctx, RcIter,
};
use jaq_json::Val;
use lru::LruCache;
use parking_lot::Mutex;
use serde_json::Value;

use std::num::NonZeroUsize;
use std::sync::{Arc, LazyLock};

/// Compiled JQ filter with its arena (arena must outlive filter).
///
/// `Arena` must be stored alongside the compiled filter because the filter may
/// reference allocations within the arena.
struct CompiledFilter {
    #[allow(dead_code)]
    arena: Arena,
    filter: jaq_core::Filter<jaq_core::Native<Val>>,
}

/// Cache entries are individually locked so JQ execution does not serialize
/// across *all* expressions (only per-expression).
type CacheEntry = Arc<Mutex<CompiledFilter>>;

/// Global LRU cache for compiled JQ programs.
///
/// IMPORTANT: This lock is held only for cache bookkeeping (lookup/insert/LRU update).
/// JQ execution happens outside the global cache lock.
static JQ_FILTER_CACHE: LazyLock<Mutex<LruCache<String, CacheEntry>>> = LazyLock::new(|| {
    Mutex::new(LruCache::new(
        NonZeroUsize::new(100).expect("non-zero cache capacity"),
    ))
});

/// Compile a JQ expression into a `CompiledFilter`.
fn compile_filter(expr: &str) -> Result<CompiledFilter, WebPipeError> {
    // Create loader with standard library.
    let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));

    // Arena must outlive compiled modules/filter.
    let arena = Arena::default();

    // Load and parse the program.
    let program = File { code: expr, path: () };
    let modules = loader.load(&arena, program).map_err(|e| {
        WebPipeError::MiddlewareExecutionError(format!("JQ parsing error: {e:?}"))
    })?;

    // Compile with standard functions.
    let filter = Compiler::default()
        .with_funs(jaq_std::funs().chain(jaq_json::funs()))
        .compile(modules)
        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ compilation error: {e:?}")))?;

    Ok(CompiledFilter { arena, filter })
}

/// Get a cached compiled filter for `expr`, compiling and caching on miss.
///
/// The global cache lock is held only briefly. On cache miss, compilation occurs
/// outside the cache lock.
///
/// Note: Under high concurrency, the same expression can be compiled more than once
/// if two threads miss simultaneously; the second inserter will reuse the cached entry.
fn get_or_compile(expr: &str) -> Result<CacheEntry, WebPipeError> {
    // Fast path: try cache hit (and update LRU order).
    if let Some(entry) = {
        let mut cache = JQ_FILTER_CACHE.lock();
        cache.get(expr).cloned()
    } {
        return Ok(entry);
    }

    // Miss: compile outside global cache lock.
    let compiled = compile_filter(expr)?;
    let new_entry: CacheEntry = Arc::new(Mutex::new(compiled));

    // Insert (or reuse if another thread inserted meanwhile).
    let mut cache = JQ_FILTER_CACHE.lock();
    if let Some(existing) = cache.get(expr).cloned() {
        Ok(existing)
    } else {
        cache.put(expr.to_string(), Arc::clone(&new_entry));
        Ok(new_entry)
    }
}

/// Evaluate a JQ expression against a JSON value and return the first result.
///
/// This is the shared runtime function used by all middleware that need to evaluate
/// JQ expressions. It uses a global LRU cache for compiled programs, and per-entry
/// locks to avoid serializing execution across unrelated expressions.
///
/// # Semantics
/// - Returns the **first** output value produced by the filter.
/// - If the filter produces no outputs, returns `null`.
///
/// # Performance
/// - Cache Hit: O(1) cache lookup + JQ execution
/// - Cache Miss: compilation + JQ execution, then cached for future use
///
/// # Arguments
/// * `expr` - The JQ expression to evaluate
/// * `input` - The JSON value to use as input to the JQ expression
///
/// # Returns
/// The first result of evaluating the JQ expression as a JSON value.
/// If the filter produces no results, returns `null`.
pub fn evaluate(expr: &str, input: &Value) -> Result<Value, WebPipeError> {
    // Keep behavior stable, but reduce cache misses from surrounding whitespace.
    let expr = expr.trim();

    let entry = get_or_compile(expr)?;

    // Convert serde_json::Value -> jaq_json::Val.
    // (This clones the JSON input; jaq_json::Val is an owned representation.)
    let jaq_input = Val::from(input.clone());

    // Create execution context with no inputs iterator.
    let inputs = RcIter::new(core::iter::empty());
    let ctx = Ctx::new([], &inputs);

    // IMPORTANT: The iterator returned by `run(...)` can borrow from the compiled filter.
    // Therefore we must keep the entry lock alive until we've pulled the first result.
    let first_val: Val = {
        let compiled = entry.lock();
        let mut results = compiled.filter.run((ctx, jaq_input));

        results
            .next()
            .unwrap_or_else(|| Ok(Val::Null))
            .map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!("JQ execution error: {e}"))
            })?
    };

    // Convert jaq_json::Val -> serde_json::Value.
    Ok(Value::from(first_val))
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
        // First call - will compile and cache.
        let result1 = evaluate(".x + 5", &input).unwrap();
        assert_eq!(result1, json!(15));

        // Second call - should hit cache.
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
        // Test that we only get first result from iterator.
        let input = json!([1, 2, 3]);
        let result = evaluate(".[]", &input).unwrap();
        assert_eq!(result, json!(1)); // Only first element.
    }

    #[test]
    fn test_evaluate_no_results_returns_null() {
        let input = json!([]);
        let result = evaluate(".[] | select(. > 10)", &input).unwrap();
        assert_eq!(result, json!(null));
    }
}
