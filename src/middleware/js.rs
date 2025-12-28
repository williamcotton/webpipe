use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use boa_engine::{Context as BoaContext, JsValue, Source, JsResult, NativeFunction, object::builtins::JsArray};
use serde_json::Value;
use sqlx::{self};
use std::sync::Arc;
use tokio::runtime::Handle;

#[derive(Debug)]
pub struct JsMiddleware {
    pub(crate) ctx: Arc<Context>,
}

impl JsMiddleware {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }

    fn load_scripts_from_dir(&self, boa_ctx: &mut BoaContext) -> Result<(), WebPipeError> {
        // Use preloaded scripts from Context (no blocking I/O!)
        for (module_name, source) in self.ctx.js_scripts.iter() {
            match boa_ctx.eval(Source::from_bytes(source.as_bytes())) {
                Ok(returned) => {
                    if let Err(e) = boa_ctx.global_object().set(
                        boa_engine::js_string!(module_name.as_str()),
                        returned,
                        false,
                        boa_ctx,
                    ) {
                        return Err(WebPipeError::MiddlewareExecutionError(
                            format!("Failed to register JavaScript module '{}' as global: {}", module_name, e)
                        ));
                    }
                }
                Err(e) => {
                    return Err(WebPipeError::MiddlewareExecutionError(
                        format!("JavaScript script '{}' execution error: {}", module_name, e)
                    ));
                }
            }
        }
        Ok(())
    }

    fn sandbox_context(&self, boa_ctx: &mut BoaContext) -> Result<(), WebPipeError> {
        // Disable dangerous globals
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
                    boa_engine::JsNativeError::typ()
                        .with_message("getEnv requires string argument")
                })?;

            match std::env::var(&key) {
                Ok(val) => Ok(JsValue::from(boa_engine::JsString::from(val))),
                Err(_) => Ok(JsValue::undefined()),
            }
        });

        let js_fn = boa_engine::object::FunctionObjectBuilder::new(
            boa_ctx.realm(),
            get_env_fn
        )
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

        let require_fn = unsafe { NativeFunction::from_closure(move |_this, args, context| {
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
        }) };

        let js_fn = boa_engine::object::FunctionObjectBuilder::new(
            boa_ctx.realm(),
            require_fn
        )
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

        let exec_sql_fn = unsafe { NativeFunction::from_closure(move |_this, args, context| {
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
                            .map_err(|e| boa_engine::JsNativeError::typ().with_message(e.to_string()))?;
                        let len = len_val
                            .to_u32(context)
                            .map_err(|e| boa_engine::JsNativeError::typ().with_message(e.to_string()))?;

                        let mut values = Vec::new();
                        for i in 0..len {
                            let elem = array_obj
                                .get(i, context)
                                .map_err(|e| boa_engine::JsNativeError::typ().with_message(e.to_string()))?;
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
                            query_builder = query_builder.bind(sqlx::types::Json(param.clone()));
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
                    let js_val = Self::json_to_js(context, &json_val)
                        .map_err(|e| boa_engine::JsNativeError::typ().with_message(e.to_string()))?;
                    Ok(js_val)
                }
                Err(err_msg) => Err(boa_engine::JsNativeError::typ()
                    .with_message(err_msg)
                    .into()),
            }
        }) };

        let js_fn = boa_engine::object::FunctionObjectBuilder::new(
            boa_ctx.realm(),
            exec_sql_fn
        )
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

    fn create_js_context(&self) -> Result<BoaContext, WebPipeError> {
        let mut boa_ctx = BoaContext::default();

        // 1. Disable dangerous globals
        self.sandbox_context(&mut boa_ctx)?;

        // 2. Register runtime functions
        self.register_get_env(&mut boa_ctx)?;
        self.register_require_script(&mut boa_ctx)?;
        self.register_execute_sql(&mut boa_ctx)?;

        // 3. Preload scripts
        self.load_scripts_from_dir(&mut boa_ctx)?;

        Ok(boa_ctx)
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
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(e.to_string()))? as u64;

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
        script: &str,
        input: &Value,
        ctx: &mut crate::executor::RequestContext,
        env: &crate::executor::ExecutionEnv,
    ) -> Result<Value, WebPipeError> {
        // Create a fresh Boa context for each request to avoid GC issues
        let mut boa_ctx = self.create_js_context()?;

        // Create isolated scope using IIFE
        let input_json = serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string());
        let context_json = serde_json::to_string(&ctx.to_value(env))
            .unwrap_or_else(|_| "{}".to_string());

        let wrapper = format!(
            r#"
            (function() {{
                // Inject request
                const request = {};

                // Inject context
                const context = {};

                // Inject flag functions
                const flags = {{}};
                const getFlag = function(key) {{ return flags[key] || false; }};
                const setFlag = function(key, val) {{ flags[key] = val; }};

                // Inject condition functions
                const conditions = {{}};
                const getWhen = function(key) {{ return conditions[key] || false; }};
                const setWhen = function(key, val) {{ conditions[key] = val; }};

                // User script
                {}
            }})()
        "#,
            input_json, context_json, script
        );

        let result = boa_ctx
            .eval(Source::from_bytes(wrapper.as_bytes()))
            .map_err(|e| {
                WebPipeError::MiddlewareExecutionError(format!(
                    "JavaScript execution error: {}",
                    e
                ))
            })?;

        Self::js_to_json_with_depth(result, &mut boa_ctx, 0)
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
    ) -> Result<(), WebPipeError> {
        let result = self.execute_js_script(config, &pipeline_ctx.state, ctx, env)?;
        pipeline_ctx.state = result;
        Ok(())
    }

    fn behavior(&self) -> super::StateBehavior {
        super::StateBehavior::Transform
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Context;
    use crate::runtime::context::{CacheStore, ConfigSnapshot, RateLimitStore};
    use reqwest::Client;
    use handlebars::Handlebars;
    use parking_lot::{Mutex, RwLock};

    fn ctx_no_db() -> Arc<Context> {
        Arc::new(Context {
            pg: None,
            http: Client::new(),
            cache: CacheStore::new(64, 1),
            rate_limit: RateLimitStore::new(1000),
            hb: Arc::new(Mutex::new(Handlebars::new())),
            cfg: ConfigSnapshot(serde_json::json!({})),
            lua_scripts: Arc::new(std::collections::HashMap::new()),
            js_scripts: Arc::new(std::collections::HashMap::new()),
            graphql: None,
            execution_env: Arc::new(RwLock::new(None)),
        })
    }

    struct StubInvoker;
    #[async_trait::async_trait]
    impl crate::executor::MiddlewareInvoker for StubInvoker {
        async fn call(
            &self,
            _name: &str,
            _args: &[String],
            _cfg: &str,
            _pipeline_ctx: &mut crate::runtime::PipelineContext,
            _env: &crate::executor::ExecutionEnv,
            _ctx: &mut crate::executor::RequestContext,
            _target_name: Option<&str>,
        ) -> Result<(), WebPipeError> {
            Ok(())
        }
    }

    fn dummy_env() -> crate::executor::ExecutionEnv {
        use crate::executor::ExecutionEnv;
        use crate::runtime::context::{CacheStore, RateLimitStore};
        let registry = Arc::new(crate::middleware::MiddlewareRegistry::empty());
        ExecutionEnv {
            variables: Arc::new(std::collections::HashMap::new()),
            named_pipelines: Arc::new(std::collections::HashMap::new()),
            invoker: Arc::new(StubInvoker),
            registry,
            environment: None,
            cache: CacheStore::new(8, 60),
            rate_limit: RateLimitStore::new(1000),
            #[cfg(feature = "debugger")]
            debugger: None,
        }
    }

    #[tokio::test]
    async fn json_js_roundtrip_primitives_and_objects() {
        use crate::middleware::Middleware;

        let mw = JsMiddleware::new(ctx_no_db());
        let input = serde_json::json!({"x": 1, "arr": [1, 2], "obj": {"a": true}});
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());

        mw.execute(
            &[],
            "return { x: request.x, arr: request.arr, obj: request.obj }",
            &mut pipeline_ctx,
            &env,
            &mut ctx,
            None,
        )
        .await
        .unwrap();

        assert_eq!(pipeline_ctx.state["x"], serde_json::json!(1));
        assert_eq!(pipeline_ctx.state["arr"].as_array().unwrap().len(), 2);
        assert_eq!(pipeline_ctx.state["obj"]["a"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn undefined_converts_to_null() {
        use crate::middleware::Middleware;

        let mw = JsMiddleware::new(ctx_no_db());
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({}));

        mw.execute(
            &[],
            "return { value: undefined }",
            &mut pipeline_ctx,
            &env,
            &mut ctx,
            None,
        )
        .await
        .unwrap();

        assert_eq!(pipeline_ctx.state["value"], serde_json::json!(null));
    }

    #[tokio::test]
    async fn sandbox_prevents_global_pollution() {
        use crate::middleware::Middleware;

        let mw = JsMiddleware::new(ctx_no_db());
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();

        // First request: accidentally write to global
        let script1 = "if (typeof counter === 'undefined') { counter = 0; } counter++; return { count: counter };";
        let mut pipeline_ctx1 = crate::runtime::PipelineContext::new(serde_json::json!({}));
        mw.execute(&[], script1, &mut pipeline_ctx1, &env, &mut ctx, None)
            .await
            .unwrap();
        assert_eq!(pipeline_ctx1.state["count"], serde_json::json!(1));

        // Second request: should return 1 again (not 2) because IIFE creates new scope
        let mut pipeline_ctx2 = crate::runtime::PipelineContext::new(serde_json::json!({}));
        mw.execute(&[], script1, &mut pipeline_ctx2, &env, &mut ctx, None)
            .await
            .unwrap();
        assert_eq!(
            pipeline_ctx2.state["count"],
            serde_json::json!(1),
            "IIFE should prevent global pollution"
        );
    }

    #[tokio::test]
    async fn get_env_returns_env_variables() {
        use crate::middleware::Middleware;

        std::env::set_var("TEST_VAR_JS", "test_value");
        let mw = JsMiddleware::new(ctx_no_db());
        let env = dummy_env();
        let mut ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(serde_json::json!({}));

        mw.execute(
            &[],
            "return { value: getEnv('TEST_VAR_JS') }",
            &mut pipeline_ctx,
            &env,
            &mut ctx,
            None,
        )
        .await
        .unwrap();

        assert_eq!(
            pipeline_ctx.state["value"],
            serde_json::json!("test_value")
        );
    }
}
