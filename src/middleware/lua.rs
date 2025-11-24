use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use mlua::{Lua, Value as LuaValue, Result as LuaResult};
use serde_json::Value;
use sqlx::{self};
use std::sync::Arc;
use tokio::runtime::Handle;

// Thread-local Lua state since Lua is not Send + Sync
thread_local! {
    static LUA_STATE: std::cell::RefCell<Option<Lua>> = const { std::cell::RefCell::new(None) };
    static LUA_INITIALIZED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[derive(Debug)]
pub struct LuaMiddleware { pub(crate) ctx: Arc<Context> }

impl LuaMiddleware {
    pub fn new(ctx: Arc<Context>) -> Self { Self { ctx } }

    fn load_scripts_from_dir(&self, lua: &Lua) -> Result<(), WebPipeError> {
        // Use preloaded scripts from Context (no blocking I/O!)
        for (module_name, source) in self.ctx.lua_scripts.iter() {
            let chunk = lua.load(source).set_name(format!("@{}", module_name));
            let returned: LuaValue = chunk.eval()
                .map_err(|e| WebPipeError::MiddlewareExecutionError(
                    format!("Lua script '{}' execution error: {}", module_name, e)
                ))?;
            lua.globals().set(module_name.as_str(), returned)
                .map_err(|e| WebPipeError::MiddlewareExecutionError(
                    format!("Failed to register Lua module as global: {}", e)
                ))?;
        }
        Ok(())
    }

    fn ensure_lua_initialized(&self) -> Result<(), WebPipeError> {
        LUA_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.is_none() {
                let lua = Lua::new();
                if let Err(e) = lua.load(r#"
                    os.execute = nil
                    os.exit = nil
                    io = nil
                    debug = nil
                "#).exec() { return Err(WebPipeError::MiddlewareExecutionError(format!("Failed to initialize Lua environment: {}", e))); }

                let get_env_fn = lua.create_function(|lua, key: String| {
                    match std::env::var(&key) { Ok(val) => Ok(LuaValue::String(lua.create_string(&val)?)), Err(_) => Ok(LuaValue::Nil) }
                }).map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Failed to create getEnv: {}", e)))?;
                if let Err(e) = lua.globals().set("getEnv", get_env_fn) { return Err(WebPipeError::MiddlewareExecutionError(format!("Failed to register getEnv: {}", e))); }

                // Clone Arc to move into closure (cheap - just increments ref count)
                let scripts = self.ctx.lua_scripts.clone();
                let require_fn = lua.create_function(move |lua, name: String| {
                    // Use preloaded scripts from Context (no blocking I/O!)
                    match scripts.get(&name) {
                        Some(src) => {
                            let chunk = lua.load(src).set_name(format!("@{}", name));
                            let val: LuaValue = chunk.eval().map_err(|e| mlua::Error::external(e.to_string()))?;
                            lua.globals().set(name, val.clone())?;
                            Ok(val)
                        },
                        None => Ok(LuaValue::Nil)
                    }
                }).map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Failed to create requireScript: {}", e)))?;
                if let Err(e) = lua.globals().set("requireScript", require_fn) { return Err(WebPipeError::MiddlewareExecutionError(format!("Failed to register requireScript: {}", e))); }

                self.load_scripts_from_dir(&lua)?;

                let pool_opt = self.ctx.pg.clone();
                let exec_sql_fn = lua.create_function(move |lua, (query, params): (String, Option<LuaValue>)| {
                    let pool = match pool_opt.clone() { Some(p) => p, None => { let err = lua.create_string("database not configured")?; return Ok((LuaValue::Nil, LuaValue::String(err))); } };

                    // Convert Lua params to JSON values for binding
                    let param_values: Vec<Value> = match params {
                        Some(LuaValue::Table(table)) => {
                            let mut values = Vec::new();
                            let mut i = 1;
                            loop {
                                match table.get::<i32, LuaValue>(i) {
                                    Ok(LuaValue::Nil) => break,
                                    Ok(val) => {
                                        match LuaMiddleware::lua_to_json_with_depth(val, 0) {
                                            Ok(json_val) => {
                                                values.push(json_val);
                                            }
                                            Err(_) => {
                                                break;
                                            }
                                        }
                                    }
                                    Err(_) => break,
                                }
                                i += 1;
                            }
                            values
                        }
                        Some(_) => {
                            let err = lua.create_string("params must be a table/array")?;
                            return Ok((LuaValue::Nil, LuaValue::String(err)));
                        }
                        None => Vec::new(),
                    };

                    let handle = Handle::current();
                    let fut = async move {
                        let wrapped_sql = format!("WITH t AS ({}) SELECT COALESCE(json_agg(row_to_json(t)), '[]'::json) AS rows FROM t", query);

                        // Build query with parameters
                        let mut query_builder = sqlx::query_scalar::<_, sqlx::types::Json<Value>>(&wrapped_sql);
                        for (_, param) in param_values.iter().enumerate() {
                            query_builder = match param {
                                Value::Null => query_builder.bind(None::<String>),
                                Value::Bool(b) => query_builder.bind(*b),
                                Value::Number(n) => {
                                    if let Some(i) = n.as_i64() {
                                        query_builder.bind(i)
                                    } else if let Some(f) = n.as_f64() {
                                        query_builder.bind(f)
                                    } else {
                                        query_builder.bind(0)
                                    }
                                }
                                Value::String(s) => query_builder.bind(s.clone()),
                                Value::Array(_) | Value::Object(_) => {
                                    // Bind complex types as JSON
                                    query_builder.bind(sqlx::types::Json(param.clone()))
                                }
                            };
                        }

                        let rows_json_res = query_builder.fetch_one(&pool).await;
                        match rows_json_res {
                            Ok(json) => { let rows = json.0; let row_count = rows.as_array().map(|a| a.len()).unwrap_or(0); let val = serde_json::json!({ "rows": rows, "rowCount": row_count }); Ok::<Value, String>(val) }
                            Err(e) => {
                                tracing::error!("SQL error: {}", e);
                                Err(e.to_string())
                            },
                        }
                    };
                    match tokio::task::block_in_place(|| handle.block_on(fut)) {
                        Ok(json_val) => {
                            let lua_val = LuaMiddleware::json_to_lua(lua, &json_val)?;
                            Ok((lua_val, LuaValue::Nil))
                        }
                        Err(err_msg) => {
                            let err = lua.create_string(&err_msg)?;
                            Ok((LuaValue::Nil, LuaValue::String(err)))
                        }
                    }
                }).map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Failed to create executeSql: {}", e)))?;
                if let Err(e) = lua.globals().set("executeSql", exec_sql_fn) { return Err(WebPipeError::MiddlewareExecutionError(format!("Failed to register executeSql: {}", e))); }

                *state = Some(lua);

                // Mark as initialized to avoid future checks
                LUA_INITIALIZED.with(|initialized| initialized.set(true));
            }
            Ok(())
        })
    }

    fn json_to_lua<'lua>(lua: &'lua Lua, value: &Value) -> LuaResult<LuaValue<'lua>> {
        match value {
            Value::Null => Ok(LuaValue::Nil),
            Value::Bool(b) => Ok(LuaValue::Boolean(*b)),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() { Ok(LuaValue::Integer(i)) }
                else if let Some(f) = n.as_f64() { Ok(LuaValue::Number(f)) }
                else { Ok(LuaValue::Number(0.0)) }
            },
            Value::String(s) => lua.create_string(s).map(LuaValue::String),
            Value::Array(arr) => { let table = lua.create_table()?; for (i, item) in arr.iter().enumerate() { table.set(i + 1, Self::json_to_lua(lua, item)?)?; } Ok(LuaValue::Table(table)) },
            Value::Object(obj) => { let table = lua.create_table()?; for (key, val) in obj { table.set(key.as_str(), Self::json_to_lua(lua, val)?)?; } Ok(LuaValue::Table(table)) }
        }
    }

    fn lua_to_json_with_depth(value: LuaValue, depth: u32) -> Result<Value, WebPipeError> {
        const MAX_DEPTH: u32 = 20;
        if depth > MAX_DEPTH { return Ok(Value::String("[max depth exceeded]".to_string())); }
        match value {
            LuaValue::Nil => Ok(Value::Null),
            LuaValue::Boolean(b) => Ok(Value::Bool(b)),
            LuaValue::Integer(i) => Ok(Value::Number(serde_json::Number::from(i))),
            LuaValue::Number(n) => { if let Some(num) = serde_json::Number::from_f64(n) { Ok(Value::Number(num)) } else { Ok(Value::Number(serde_json::Number::from(0))) } },
            LuaValue::String(s) => { match s.to_str() { Ok(str_val) => Ok(Value::String(str_val.to_string())), Err(_) => Ok(Value::String("[invalid utf8]".to_string())) } },
            LuaValue::Table(table) => {
                let pairs_result: Result<Vec<_>, _> = table.pairs::<LuaValue, LuaValue>().collect();
                let pairs = match pairs_result { Ok(p) => p, Err(_) => return Ok(Value::Object(serde_json::Map::new())) };
                let mut is_array = true; let mut max_index = 0i64; let mut count = 0;
                for (key, _) in &pairs {
                    match key { LuaValue::Integer(i) if *i > 0 => { max_index = max_index.max(*i); count += 1; }, _ => { is_array = false; break; } }
                }
                if is_array && count == max_index && count > 0 {
                    let mut array_items: Vec<(i64, LuaValue)> = pairs.into_iter().filter_map(|(key, value)| { if let LuaValue::Integer(i) = key { Some((i, value)) } else { None } }).collect();
                    array_items.sort_by_key(|(i, _)| *i);
                    let mut array = Vec::with_capacity(count as usize);
                    for (_, value) in array_items { array.push(Self::lua_to_json_with_depth(value, depth + 1)?); }
                    Ok(Value::Array(array))
                } else {
                    let mut object = serde_json::Map::new();
                    for (key, value) in pairs {
                        let key_str = match key { LuaValue::String(s) => s.to_str().unwrap_or("[invalid key]").to_string(), LuaValue::Integer(i) => i.to_string(), LuaValue::Number(n) => n.to_string(), _ => "[unsupported key]".to_string() };
                        object.insert(key_str, Self::lua_to_json_with_depth(value, depth + 1)?);
                    }
                    Ok(Value::Object(object))
                }
            },
            _ => Ok(Value::String("[unsupported lua type]".to_string()))
        }
    }

    fn execute_lua_script(&self, script: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Fast path: check if already initialized without function call overhead
        let initialized = LUA_INITIALIZED.with(|initialized| initialized.get());
        if !initialized {
            self.ensure_lua_initialized()?;
        }

        LUA_STATE.with(|state| {
            let state = state.borrow();
            let lua = state.as_ref().unwrap();
            let lua_input = Self::json_to_lua(lua, input).map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JSON to Lua conversion error: {}", e)))?;
            lua.globals().set("request", lua_input).map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Setting request variable error: {}", e)))?;
            let result: LuaValue = lua.load(script).eval().map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Lua execution error: {}", e)))?;
            Self::lua_to_json_with_depth(result, 0)
        })
    }
}

#[async_trait]
impl super::Middleware for LuaMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        self.execute_lua_script(config, input)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Context;
    use crate::runtime::context::{CacheStore, ConfigSnapshot};
    use crate::middleware::Middleware;

    fn ctx_no_db() -> Arc<Context> {
        Arc::new(Context {
            pg: None,
            http: reqwest::Client::builder().build().unwrap(),
            cache: CacheStore::new(64, 1),
            hb: std::sync::Arc::new(parking_lot::Mutex::new(handlebars::Handlebars::new())),
            cfg: ConfigSnapshot(serde_json::json!({})),
            lua_scripts: std::sync::Arc::new(std::collections::HashMap::new()),
            graphql: None,
            execution_env: None,
        })
    }

    fn ctx_with_scripts() -> Arc<Context> {
        let mut scripts = std::collections::HashMap::new();
        // Try to load scripts from the scripts directory for testing
        let scripts_dir = std::env::var("WEBPIPE_SCRIPTS_DIR").unwrap_or_else(|_| "scripts".to_string());
        if let Ok(entries) = std::fs::read_dir(&scripts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()).map(|e| e.eq_ignore_ascii_case("lua")).unwrap_or(false) {
                    if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            scripts.insert(name.to_string(), content);
                        }
                    }
                }
            }
        }

        Arc::new(Context {
            pg: None,
            http: reqwest::Client::builder().build().unwrap(),
            cache: CacheStore::new(64, 1),
            hb: std::sync::Arc::new(parking_lot::Mutex::new(handlebars::Handlebars::new())),
            cfg: ConfigSnapshot(serde_json::json!({})),
            lua_scripts: std::sync::Arc::new(scripts),
            graphql: None,
            execution_env: None,
        })
    }

    #[tokio::test]
    async fn json_lua_roundtrip_primitives_and_tables() {
        let mw = LuaMiddleware::new(ctx_no_db());
        let input = serde_json::json!({"x": 1, "arr": [1,2], "obj": {"a": true}});
        let out = mw.execute("return { x = request.x, arr = request.arr, obj = request.obj }", &input).await.unwrap();
        assert_eq!(out["x"], serde_json::json!(1));
        assert_eq!(out["arr"].as_array().unwrap().len(), 2);
        assert_eq!(out["obj"]["a"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn require_script_and_get_env_registered() {
        std::env::set_var("WEBPIPE_SCRIPTS_DIR", "scripts");
        let mw = LuaMiddleware::new(ctx_with_scripts());
        let out = mw.execute("local c = requireScript('dateFormatter'); return type(c) ~= 'nil'", &serde_json::json!({})).await.unwrap();
        assert!(out.as_bool().unwrap() || out.is_string());
        let out2 = mw.execute("return getEnv('PATH') ~= nil", &serde_json::json!({})).await.unwrap();
        assert!(out2.as_bool().unwrap());
    }
}

