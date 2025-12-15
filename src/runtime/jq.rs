use crate::error::WebPipeError;

use jaq_core::{
    load::{Arena, File, Loader},
    Compiler, Ctx, RcIter,
};
use jaq_json::Val;
use lru::LruCache;
use serde_json::Value;

use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::rc::Rc;

// We wrap the filter in Rc so we can clone it cheaply out of the cache 
// if we need to (though we mostly just borrow it).
// We do NOT need Arc or Mutex because this is strictly single-threaded.
struct CompiledFilter {
    #[allow(dead_code)]
    arena: Rc<Arena>,
    filter: jaq_core::Filter<jaq_core::Native<Val>>,
}

// Thread-Local Cache: No locks, just a RefCell for interior mutability.
thread_local! {
    static THREAD_JQ_CACHE: RefCell<LruCache<String, Rc<CompiledFilter>>> = RefCell::new(
        LruCache::new(NonZeroUsize::new(4096).expect("non-zero cache capacity"))
    );
}

/// Compile a JQ expression into a `CompiledFilter`.
fn compile_filter(expr: &str) -> Result<CompiledFilter, WebPipeError> {
    let loader = Loader::new(jaq_std::defs().chain(jaq_json::defs()));
    let arena = Rc::new(Arena::default());
    let program = File { code: expr, path: () };
    
    // Load into the arena
    let modules = loader.load(&arena, program).map_err(|e| {
        WebPipeError::MiddlewareExecutionError(format!("JQ parsing error: {e:?}"))
    })?;

    // Compile
    let filter = Compiler::default()
        .with_funs(jaq_std::funs().chain(jaq_json::funs()))
        .compile(modules)
        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ compilation error: {e:?}")))?;

    Ok(CompiledFilter { arena, filter })
}

/// Get a compiled filter from the thread-local cache, compiling on miss.
fn get_or_compile(expr: &str) -> Result<Rc<CompiledFilter>, WebPipeError> {
    THREAD_JQ_CACHE.with(|cache_cell| {
        let mut cache = cache_cell.borrow_mut();
        
        // Fast path: Hit
        if let Some(filter) = cache.get(expr) {
            return Ok(filter.clone());
        }

        // Slow path: Compile (happens once per thread per filter)
        let compiled = compile_filter(expr)?;
        let rc = Rc::new(compiled);
        
        cache.put(expr.to_string(), rc.clone());
        Ok(rc)
    })
}

/// Evaluate a JQ expression against a JSON value.
/// This function is synchronous and CPU-bound.
pub fn evaluate(expr: &str, input: &Value) -> Result<Value, WebPipeError> {
    let expr = expr.trim();
    
    // Retrieve the filter (this is lock-free, just a RefCell borrow)
    let entry = get_or_compile(expr)?;

    let jaq_input = Val::from(input.clone());
    let inputs = RcIter::new(core::iter::empty());
    let ctx = Ctx::new([], &inputs);

    // Execute
    // We run this without holding the cache RefCell borrow, thanks to Rc cloning above.
    let mut results = entry.filter.run((ctx, jaq_input));
    
    let first_val = results
        .next()
        .unwrap_or_else(|| Ok(Val::Null))
        .map_err(|e| {
            WebPipeError::MiddlewareExecutionError(format!("JQ execution error: {e}"))
        })?;

    Ok(Value::from(first_val))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_thread_local_caching() {
        let input = json!({"val": 10});
        
        // First call: Compiles and inserts into this thread's cache
        let res1 = evaluate(".val + 1", &input).unwrap();
        assert_eq!(res1, json!(11));

        // Second call: Hits the thread-local cache
        let res2 = evaluate(".val + 1", &input).unwrap();
        assert_eq!(res2, json!(11));
    }

    #[test]
    fn test_concurrent_threads() {
        // Verify that different threads work independently
        let handle = std::thread::spawn(|| {
            let input = json!({"a": 1});
            evaluate(".a", &input).unwrap()
        });
        
        let res = handle.join().unwrap();
        assert_eq!(res, json!(1));
    }
}