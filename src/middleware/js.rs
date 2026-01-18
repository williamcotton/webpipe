use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use boa_engine::{
    object::builtins::JsArray, Context as BoaContext, JsResult, JsValue, NativeFunction, Script,
    Source,
};
use lru::LruCache;
use serde_json::Value;
use sqlx::{self};
use std::cell::RefCell;
use std::collections::HashSet;
use std::mem::ManuallyDrop;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::runtime::Handle;

// Container struct for the JS runtime state.
// We use ManuallyDrop to intentionally LEAK the context and cache on thread destruction.
// This is necessary because Boa's thread-local Garbage Collector state may be dropped
// before this struct, causing a panic (subtract with overflow) in Boa's destructor
// if we try to clean up normally.
struct JsRuntimeState {
    cache: ManuallyDrop<LruCache<String, Script>>,
    context: ManuallyDrop<BoaContext>,
    // Store the list of global keys that are allowed to exist (captured at startup)
    initial_keys: HashSet<String>,
}

thread_local! {
    static RUNTIME_STATE: RefCell<Option<JsRuntimeState>> = RefCell::new(None);
}

#[derive(Debug)]
pub struct JsMiddleware {
    pub(crate) ctx: Arc<Context>,
}

impl JsMiddleware {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }

    /// Initializes a fresh context, loads helpers, locks down intrinsics,
    /// and captures the "clean" state of the global object.
    fn initialize_runtime(&self) -> Result<JsRuntimeState, WebPipeError> {
        let mut boa_ctx = BoaContext::default();

        // 1. Disable dangerous globals
        self.sandbox_context(&mut boa_ctx)?;

        // 2. Register runtime functions
        self.register_get_env(&mut boa_ctx)?;
        self.register_require_script(&mut boa_ctx)?;
        self.register_execute_sql(&mut boa_ctx)?;

        // 3. Preload helper scripts
        self.load_scripts_from_dir(&mut boa_ctx)?;

        // 4. Lockdown: Freeze standard library to prevent prototype pollution
        self.lockdown_intrinsics(&mut boa_ctx)?;

        // 5. Snapshot: Record the "clean" list of global keys
        let initial_keys = self.get_global_keys(&mut boa_ctx)?;

        let cache = LruCache::new(NonZeroUsize::new(64).unwrap());

        Ok(JsRuntimeState {
            context: ManuallyDrop::new(boa_ctx),
            cache: ManuallyDrop::new(cache),
            initial_keys,
        })
    }

    /// Helper to get all current string keys from the global object
    fn get_global_keys(&self, boa_ctx: &mut BoaContext) -> Result<HashSet<String>, WebPipeError> {
        let global = boa_ctx.global_object();
        let keys = global
            .own_property_keys(boa_ctx)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?;

        let mut key_set = HashSet::new();
        for key in keys {
            // FIX: Convert PropertyKey to JsValue first to stringify properly
            let js_val = JsValue::from(key);
            if let Ok(js_string) = js_val.to_string(boa_ctx) {
                if let Ok(s) = js_string.to_std_string() {
                    key_set.insert(s);
                }
            }
        }
        Ok(key_set)
    }

    /// Freezes standard intrinsic objects to prevent modification
    fn lockdown_intrinsics(&self, boa_ctx: &mut BoaContext) -> Result<(), WebPipeError> {
        let script = r#"
            (function() {
                const objects = [
                    Object, Array, String, Number, Boolean, Date, RegExp, Error, 
                    Math, JSON, Promise, Map, Set, WeakMap, WeakSet
                ];
                
                objects.forEach(obj => {
                    if (obj) {
                        // Freeze the prototype if it exists (prevents Array.prototype.push = ...)
                        if (obj.prototype) Object.freeze(obj.prototype);
                        // Freeze the static object itself (prevents Array.from = ...)
                        Object.freeze(obj);
                    }
                });
            })();
        "#;
        boa_ctx
            .eval(Source::from_bytes(script.as_bytes()))
            .map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!("Lockdown failed: {}", e))
            })?;
        Ok(())
    }

    /// Deletes any global keys that weren't present at initialization
    // FIX: Pass mutable context and immutable keys separately to avoid double borrow of `state`
    fn cleanup_environment(
        &self,
        boa_ctx: &mut BoaContext,
        initial_keys: &HashSet<String>,
    ) -> Result<(), WebPipeError> {
        let global = boa_ctx.global_object();

        // Get current keys
        let current_keys = global
            .own_property_keys(boa_ctx)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?;

        // Identify and delete pollution
        for key in current_keys {
            // FIX: Convert PropertyKey to JsValue first
            let js_val = JsValue::from(key.clone());
            if let Ok(js_string) = js_val.to_string(boa_ctx) {
                if let Ok(s) = js_string.to_std_string() {
                    // If this key wasn't in our initial allowlist, DELETE IT
                    if !initial_keys.contains(&s) {
                        // FIX: Use delete_property
                        global.delete_property_or_throw(key, boa_ctx).map_err(|e| {
                            WebPipeError::MiddlewareExecutionError(e.to_string())
                        })?;
                    }
                }
            }
        }
        Ok(())
    }

    fn load_scripts_from_dir(&self, boa_ctx: &mut BoaContext) -> Result<(), WebPipeError> {
        for (module_name, source) in self.ctx.js_scripts.iter() {
            match boa_ctx.eval(Source::from_bytes(source.as_bytes())) {
                Ok(returned) => {
                    if let Err(e) = boa_ctx.global_object().set(
                        boa_engine::js_string!(module_name.as_str()),
                        returned,
                        false,
                        boa_ctx,
                    ) {
                        return Err(WebPipeError::MiddlewareExecutionError(format!(
                            "Failed to register JavaScript module '{}' as global: {}",
                            module_name, e
                        )));
                    }
                }
                Err(e) => {
                    return Err(WebPipeError::MiddlewareExecutionError(format!(
                        "JavaScript script '{}' execution error: {}",
                        module_name, e
                    )));
                }
            }
        }
        Ok(())
    }

    fn sandbox_context(&self, boa_ctx: &mut BoaContext) -> Result<(), WebPipeError> {
        let code = r#"
            eval = undefined;
            Function = undefined;
            setTimeout = undefined;
            setInterval = undefined;
            setImmediate = undefined;
        "#;

        boa_ctx
            .eval(Source::from_bytes(code.as_bytes()))
            .map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!(
                    "Failed to sandbox JavaScript environment: {}",
                    e
                ))
            })?;

        Ok(())
    }

    fn register_get_env(&self, boa_ctx: &mut BoaContext) -> Result<(), WebPipeError> {
        let get_env_fn = NativeFunction::from_fn_ptr(|_this, args, _context| {
            let key = args
                .get(0)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_std_string().ok())
                .ok_or_else(|| {
                    boa_engine::JsNativeError::typ().with_message("getEnv requires string argument")
                })?;

            match std::env::var(&key) {
                Ok(val) => Ok(JsValue::from(boa_engine::JsString::from(val))),
                Err(_) => Ok(JsValue::undefined()),
            }
        });

        let js_fn = boa_engine::object::FunctionObjectBuilder::new(boa_ctx.realm(), get_env_fn)
            .name("getEnv")
            .length(1)
            .build();

        boa_ctx
            .register_global_property(
                boa_engine::js_string!("getEnv"),
                js_fn,
                boa_engine::property::Attribute::all(),
            )
            .map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!("Failed to register getEnv: {}", e))
            })?;
        Ok(())
    }

    fn register_require_script(&self, boa_ctx: &mut BoaContext) -> Result<(), WebPipeError> {
        let scripts = self.ctx.js_scripts.clone();

        let require_fn = unsafe {
            NativeFunction::from_closure(move |_this, args, context| {
                let name = args
                    .get(0)
                    .and_then(|v| v.as_string())
                    .and_then(|s| s.to_std_string().ok())
                    .ok_or_else(|| {
                        boa_engine::JsNativeError::typ()
                            .with_message("requireScript requires string argument")
                    })?;

                match scripts.get(&name) {
                    Some(source) => {
                        // Check if already loaded as global
                        if let Ok(existing) = context.global_object().get(
                            boa_engine::js_string!(name.as_str()),
                            context,
                        ) {
                            if !existing.is_undefined() {
                                return Ok(existing);
                            }
                        }

                        // Execute and cache as global
                        let result = context.eval(Source::from_bytes(source.as_bytes()))?;
                        context.global_object().set(
                            boa_engine::js_string!(name.as_str()),
                            result.clone(),
                            false,
                            context,
                        )?;
                        Ok(result)
                    }
                    None => Ok(JsValue::undefined()),
                }
            })
        };

        let js_fn = boa_engine::object::FunctionObjectBuilder::new(boa_ctx.realm(), require_fn)
            .name("requireScript")
            .length(1)
            .build();

        boa_ctx
            .register_global_property(
                boa_engine::js_string!("requireScript"),
                js_fn,
                boa_engine::property::Attribute::all(),
            )
            .map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!(
                    "Failed to register requireScript: {}",
                    e
                ))
            })?;
        Ok(())
    }

    fn register_execute_sql(&self, boa_ctx: &mut BoaContext) -> Result<(), WebPipeError> {
        let pool_opt = self.ctx.pg.clone();

        let exec_sql_fn = unsafe {
            NativeFunction::from_closure(move |_this, args, context| {
                let pool = match pool_opt.clone() {
                    Some(p) => p,
                    None => {
                        return Err(boa_engine::JsNativeError::typ()
                            .with_message("database not configured")
                            .into());
                    }
                };

                // Parse query string
                let query = args
                    .get(0)
                    .and_then(|v| v.as_string())
                    .and_then(|s| s.to_std_string().ok())
                    .ok_or_else(|| {
                        boa_engine::JsNativeError::typ()
                            .with_message("executeSql requires query string")
                    })?;

                // Parse params array
                let param_values: Vec<Value> = match args.get(1) {
                    Some(val) if val.is_object() => {
                        let array_obj = val.as_object().unwrap();
                        if array_obj.is_array() {
                            let len_val = array_obj
                                .get(boa_engine::js_string!("length"), context)
                                .map_err(|e| {
                                    boa_engine::JsNativeError::typ().with_message(e.to_string())
                                })?;
                            let len = len_val.to_u32(context).map_err(|e| {
                                boa_engine::JsNativeError::typ().with_message(e.to_string())
                            })?;

                            let mut values = Vec::new();
                            for i in 0..len {
                                let elem = array_obj.get(i, context).map_err(|e| {
                                    boa_engine::JsNativeError::typ().with_message(e.to_string())
                                })?;
                                match Self::js_to_json_with_depth(elem, context, 0) {
                                    Ok(json_val) => values.push(json_val),
                                    Err(e) => {
                                        return Err(boa_engine::JsNativeError::typ()
                                            .with_message(format!("Failed to convert param: {}", e))
                                            .into());
                                    }
                                }
                            }
                            values
                        } else {
                            return Err(boa_engine::JsNativeError::typ()
                                .with_message("params must be an array")
                                .into());
                        }
                    }
                    Some(val) if val.is_undefined() || val.is_null() => Vec::new(),
                    Some(_) => {
                        return Err(boa_engine::JsNativeError::typ()
                            .with_message("params must be an array")
                            .into());
                    }
                    None => Vec::new(),
                };

                // Execute async SQL query synchronously
                let handle = Handle::current();
                let fut = async move {
                    // Same SQL execution logic as Lua middleware
                    let wrapped_sql = format!(
                    "WITH t AS ({}) SELECT COALESCE(json_agg(row_to_json(t)), '[]'::json) AS rows FROM t",
                    query
                );

                    let mut query_builder =
                        sqlx::query_scalar::<_, sqlx::types::Json<Value>>(&wrapped_sql);
                    for param in param_values.iter() {
                        match param {
                            Value::Null => {
                                query_builder = query_builder.bind(None::<String>);
                            }
                            Value::Bool(b) => {
                                query_builder = query_builder.bind(*b);
                            }
                            Value::Number(n) => {
                                if let Some(i) = n.as_i64() {
                                    query_builder = query_builder.bind(i);
                                } else if let Some(f) = n.as_f64() {
                                    query_builder = query_builder.bind(f);
                                } else {
                                    query_builder = query_builder.bind(0i64);
                                }
                            }
                            Value::String(s) => {
                                query_builder = query_builder.bind(s);
                            }
                            Value::Array(_) | Value::Object(_) => {
                                query_builder =
                                    query_builder.bind(sqlx::types::Json(param.clone()));
                            }
                        }
                    }

                    let rows_json_res = query_builder.fetch_one(&pool).await;
                    match rows_json_res {
                        Ok(json) => {
                            let rows = json.0;
                            let row_count = rows.as_array().map(|a| a.len()).unwrap_or(0);
                            Ok(serde_json::json!({ "rows": rows, "rowCount": row_count }))
                        }
                        Err(e) => Err(e.to_string()),
                    }
                };

                match tokio::task::block_in_place(|| handle.block_on(fut)) {
                    Ok(json_val) => {
                        let js_val = Self::json_to_js(context, &json_val).map_err(|e| {
                            boa_engine::JsNativeError::typ().with_message(e.to_string())
                        })?;
                        Ok(js_val)
                    }
                    Err(err_msg) => Err(boa_engine::JsNativeError::typ()
                        .with_message(err_msg)
                        .into()),
                }
            })
        };

        let js_fn = boa_engine::object::FunctionObjectBuilder::new(boa_ctx.realm(), exec_sql_fn)
            .name("executeSql")
            .length(2)
            .build();

        boa_ctx
            .register_global_property(
                boa_engine::js_string!("executeSql"),
                js_fn,
                boa_engine::property::Attribute::all(),
            )
            .map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!(
                    "Failed to register executeSql: {}",
                    e
                ))
            })?;
        Ok(())
    }

    fn json_to_js(boa_ctx: &mut BoaContext, value: &Value) -> JsResult<JsValue> {
        match value {
            Value::Null => Ok(JsValue::null()),
            Value::Bool(b) => Ok(JsValue::from(*b)),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(JsValue::from(i))
                } else if let Some(f) = n.as_f64() {
                    Ok(JsValue::from(f))
                } else {
                    Ok(JsValue::from(0))
                }
            }
            Value::String(s) => Ok(JsValue::from(boa_engine::JsString::from(s.as_str()))),
            Value::Array(arr) => {
                let js_array = JsArray::new(boa_ctx);
                for (idx, item) in arr.iter().enumerate() {
                    let js_val = Self::json_to_js(boa_ctx, item)?;
                    js_array.set(idx, js_val, false, boa_ctx)?;
                }
                Ok(JsValue::from(js_array))
            }
            Value::Object(obj) => {
                let js_obj = boa_engine::object::JsObject::with_object_proto(boa_ctx.intrinsics());
                for (key, val) in obj {
                    let js_val = Self::json_to_js(boa_ctx, val)?;
                    js_obj.set(
                        boa_engine::js_string!(key.as_str()),
                        js_val,
                        false,
                        boa_ctx,
                    )?;
                }
                Ok(JsValue::from(js_obj))
            }
        }
    }

    fn js_to_json_with_depth(
        value: JsValue,
        boa_ctx: &mut BoaContext,
        depth: u32,
    ) -> Result<Value, WebPipeError> {
        const MAX_DEPTH: u32 = 20;
        if depth > MAX_DEPTH {
            return Ok(Value::String("[max depth exceeded]".to_string()));
        }

        match value {
            JsValue::Null | JsValue::Undefined => Ok(Value::Null),
            JsValue::Boolean(b) => Ok(Value::Bool(b)),
            JsValue::Integer(i) => Ok(Value::Number(serde_json::Number::from(i))),
            JsValue::Rational(f) => {
                if let Some(num) = serde_json::Number::from_f64(f) {
                    Ok(Value::Number(num))
                } else {
                    Ok(Value::Number(serde_json::Number::from(0)))
                }
            }
            JsValue::String(s) => match s.to_std_string() {
                Ok(str_val) => Ok(Value::String(str_val)),
                Err(_) => Ok(Value::String("[invalid utf8]".to_string())),
            },
            JsValue::BigInt(_) => {
                // BigInt conversion - convert to string
                Ok(Value::String(
                    value
                        .to_string(boa_ctx)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?
                        .to_std_string()
                        .unwrap_or("[bigint]".to_string()),
                ))
            }
            JsValue::Object(obj) => {
                // Check if array
                if obj.is_array() {
                    let len_val = obj
                        .get(boa_engine::js_string!("length"), boa_ctx)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?;
                    let len = len_val
                        .to_u32(boa_ctx)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?
                        as u64;

                    let mut json_array = Vec::with_capacity(len as usize);
                    for i in 0..len {
                        let elem = obj
                            .get(i, boa_ctx)
                            .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?;
                        json_array.push(Self::js_to_json_with_depth(elem, boa_ctx, depth + 1)?);
                    }
                    Ok(Value::Array(json_array))
                } else {
                    // Regular object
                    let mut json_obj = serde_json::Map::new();
                    let keys = obj
                        .own_property_keys(boa_ctx)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?;

                    for key in keys {
                        // Convert PropertyKey to string
                        let key_val = JsValue::from(key.clone());
                        let key_js_str = key_val
                            .to_string(boa_ctx)
                            .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?;
                        let key_str = key_js_str
                            .to_std_string()
                            .unwrap_or("[invalid key]".to_string());

                        let val = obj
                            .get(key.clone(), boa_ctx)
                            .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?;

                        json_obj.insert(
                            key_str,
                            Self::js_to_json_with_depth(val, boa_ctx, depth + 1)?,
                        );
                    }
                    Ok(Value::Object(json_obj))
                }
            }
            JsValue::Symbol(_) => Ok(Value::String("[symbol]".to_string())),
        }
    }

    fn execute_js_script(
        &self,
        script_src: &str,
        input: &Value,
        ctx: &mut crate::executor::RequestContext,
        env: &crate::executor::ExecutionEnv,
    ) -> Result<Value, WebPipeError> {
        // Access the Thread Local Context
        RUNTIME_STATE.with(|state_cell| {
            let mut state_opt = state_cell.borrow_mut();

            if state_opt.is_none() {
                *state_opt = Some(self.initialize_runtime()?);
            }
            let state = state_opt.as_mut().unwrap();
            
            // FIX: Borrow fields independently to satisfy borrow checker later
            let boa_ctx = &mut state.context;
            let initial_keys = &state.initial_keys;

            // 1. Inject data as globals
            let js_input = Self::json_to_js(boa_ctx, input)
                .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?;

            // We use .set() to overwrite existing globals from previous runs
            boa_ctx
                .global_object()
                .set(
                    boa_engine::js_string!("__req"),
                    js_input,
                    false,
                    boa_ctx,
                )
                .map_err(|e| {
                    WebPipeError::MiddlewareExecutionError(format!("Failed to set __req: {}", e))
                })?;

            let js_context = Self::json_to_js(boa_ctx, &ctx.to_value(env))
                .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))?;

            boa_ctx
                .global_object()
                .set(
                    boa_engine::js_string!("__ctx"),
                    js_context,
                    false,
                    boa_ctx,
                )
                .map_err(|e| {
                    WebPipeError::MiddlewareExecutionError(format!("Failed to set __ctx: {}", e))
                })?;

            // 2. Wrap the user's code with "use strict"
            // "use strict" prevents accidental creation of globals (e.g. 'x = 1' throws error)
            let wrapped_src = format!(
                r#"
                (function(request, context) {{
                    "use strict";
                    
                    // Inject helper functions
                    const flags = {{}};
                    const getFlag = function(key) {{ return flags[key] || false; }};
                    const setFlag = function(key, val) {{ flags[key] = val; }};
                    
                    const conditions = {{}};
                    const getWhen = function(key) {{ return conditions[key] || false; }};
                    const setWhen = function(key, val) {{ conditions[key] = val; }};
                    
                    {}
                }})(__req, __ctx)
            "#,
                script_src
            );

            // 3. Execute with Cache
            let execution_result = if !state.cache.contains(script_src) {
                let script =
                    Script::parse(Source::from_bytes(wrapped_src.as_bytes()), None, boa_ctx)
                        .map_err(|e| {
                            WebPipeError::MiddlewareExecutionError(format!(
                                "JS Compilation error: {}",
                                e
                            ))
                        })?;
                state.cache.put(script_src.to_string(), script);
                state.cache.get(script_src).unwrap().evaluate(boa_ctx)
            } else {
                state.cache.get(script_src).unwrap().evaluate(boa_ctx)
            };

            // 4. CLEANUP: IMPORTANT!
            // Regardless of success or failure, we try to clean up the environment
            // to prevent data leaking to the next request.
            // We ignore errors here to prioritize returning the actual execution result (or error).
            // FIX: Pass separated references to appease borrow checker
            let _ = self.cleanup_environment(boa_ctx, initial_keys);

            let result_val = execution_result.map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!("JS Runtime error: {}", e))
            })?;

            Self::js_to_json_with_depth(result_val, boa_ctx, 0)
        })
    }
}

#[async_trait]
impl super::Middleware for JsMiddleware {
    async fn execute(
        &self,
        _args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        env: &crate::executor::ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
        _target_name: Option<&str>,
    ) -> Result<super::MiddlewareOutput, WebPipeError> {
        let result = self.execute_js_script(config, &pipeline_ctx.state, ctx, env)?;
        pipeline_ctx.state = result;
        Ok(super::MiddlewareOutput::default())
    }

    fn behavior(&self) -> super::StateBehavior {
        super::StateBehavior::Transform
    }
}