use crate::error::WebPipeError;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use lru::LruCache;
use handlebars::Handlebars;
use mlua::{Lua, Value as LuaValue, Result as LuaResult};
use once_cell::sync::OnceCell;
use sqlx::{self, postgres::PgPoolOptions, PgPool};
use std::env;

#[async_trait]
pub trait Middleware: Send + Sync + std::fmt::Debug {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError>;
}

#[derive(Debug)]
pub struct MiddlewareRegistry {
    middlewares: HashMap<String, Box<dyn Middleware>>,
}

impl MiddlewareRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            middlewares: HashMap::new(),
        };
        
        // Register built-in middleware
        registry.register("jq", Box::new(JqMiddleware::new()));
        registry.register("auth", Box::new(AuthMiddleware));
        registry.register("validate", Box::new(ValidateMiddleware));
        registry.register("pg", Box::new(PgMiddleware));
        registry.register("handlebars", Box::new(HandlebarsMiddleware::new()));
        registry.register("fetch", Box::new(FetchMiddleware));
        registry.register("cache", Box::new(CacheMiddleware));
        registry.register("lua", Box::new(LuaMiddleware::new()));
        registry.register("log", Box::new(LogMiddleware));
        
        registry
    }

    pub fn register(&mut self, name: &str, middleware: Box<dyn Middleware>) {
        self.middlewares.insert(name.to_string(), middleware);
    }

    pub async fn execute(
        &self,
        name: &str,
        config: &str,
        input: &Value,
    ) -> Result<Value, WebPipeError> {
        let middleware = self.middlewares.get(name)
            .ok_or_else(|| WebPipeError::MiddlewareNotFound(name.to_string()))?;
        
        middleware.execute(config, input).await
    }
}

// Thread-local cache for JQ programs since JqProgram is not Send + Sync
thread_local! {
    static JQ_PROGRAM_CACHE: RefCell<LruCache<String, jq_rs::JqProgram>> = RefCell::new(
        LruCache::new(std::num::NonZeroUsize::new(100).unwrap())
    );
}

// Thread-local Lua state since Lua is not Send + Sync
thread_local! {
    static LUA_STATE: RefCell<Option<Lua>> = RefCell::new(None);
}

// Placeholder middleware implementations
#[derive(Debug)]
pub struct JqMiddleware;

impl JqMiddleware {
    pub fn new() -> Self {
        Self
    }
    
    fn execute_jq(&self, filter: &str, input_json: &str) -> Result<String, WebPipeError> {
        JQ_PROGRAM_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            
            // Check if we have a compiled program for this filter
            if let Some(program) = cache.get_mut(filter) {
                // Use the cached compiled program
                return program.run(input_json)
                    .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ execution error: {}", e)));
            }
            
            // Not in cache, compile a new program
            match jq_rs::compile(filter) {
                Ok(mut program) => {
                    let result = program.run(input_json)
                        .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ execution error: {}", e)));
                    
                    // If successful, cache the compiled program for future use
                    if result.is_ok() {
                        cache.put(filter.to_string(), program);
                    }
                    
                    result
                },
                Err(e) => Err(WebPipeError::MiddlewareExecutionError(format!("JQ compilation error: {}", e)))
            }
        })
    }
}

impl Default for JqMiddleware {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Debug)]
pub struct AuthMiddleware;
#[derive(Debug)]
pub struct ValidateMiddleware;
#[derive(Debug)]
pub struct PgMiddleware;
#[derive(Debug)]
pub struct HandlebarsMiddleware {
    handlebars: Arc<Mutex<Handlebars<'static>>>,
}

impl HandlebarsMiddleware {
    pub fn new() -> Self {
        Self {
            handlebars: Arc::new(Mutex::new(Handlebars::new())),
        }
    }
    
    fn render_template(&self, template: &str, data: &Value) -> Result<String, WebPipeError> {
        let handlebars = self.handlebars.lock().unwrap();
        
        // Render the template directly without registering it
        handlebars.render_template(template, data)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Handlebars render error: {}", e)))
    }
}
#[derive(Debug)]
pub struct FetchMiddleware;
#[derive(Debug)]
pub struct CacheMiddleware;
#[derive(Debug)]
pub struct LuaMiddleware;

impl LuaMiddleware {
    pub fn new() -> Self {
        Self
    }
    
    fn ensure_lua_initialized() -> Result<(), WebPipeError> {
        LUA_STATE.with(|state| {
            let mut state = state.borrow_mut();
            if state.is_none() {
                let lua = Lua::new();
                
                // Set up safe environment by removing dangerous functions
                if let Err(e) = lua.load(r#"
                    -- Remove dangerous functions
                    os.execute = nil
                    os.exit = nil
                    io = nil
                    debug = nil
                "#).exec() {
                    return Err(WebPipeError::MiddlewareExecutionError(format!("Failed to initialize Lua environment: {}", e)));
                }
                
                *state = Some(lua);
            }
            Ok(())
        })
    }
    
    fn json_to_lua<'lua>(lua: &'lua Lua, value: &Value) -> LuaResult<LuaValue<'lua>> {
        match value {
            Value::Null => Ok(LuaValue::Nil),
            Value::Bool(b) => Ok(LuaValue::Boolean(*b)),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(LuaValue::Integer(i))
                } else if let Some(f) = n.as_f64() {
                    Ok(LuaValue::Number(f))
                } else {
                    Ok(LuaValue::Number(0.0))
                }
            },
            Value::String(s) => lua.create_string(s).map(LuaValue::String),
            Value::Array(arr) => {
                let table = lua.create_table()?;
                for (i, item) in arr.iter().enumerate() {
                    table.set(i + 1, Self::json_to_lua(lua, item)?)?; // 1-indexed
                }
                Ok(LuaValue::Table(table))
            },
            Value::Object(obj) => {
                let table = lua.create_table()?;
                for (key, val) in obj {
                    table.set(key.as_str(), Self::json_to_lua(lua, val)?)?;
                }
                Ok(LuaValue::Table(table))
            }
        }
    }
    
    fn lua_to_json_with_depth(value: LuaValue, depth: u32) -> Result<Value, WebPipeError> {
        const MAX_DEPTH: u32 = 20;
        
        if depth > MAX_DEPTH {
            return Ok(Value::String("[max depth exceeded]".to_string()));
        }
        
        match value {
            LuaValue::Nil => Ok(Value::Null),
            LuaValue::Boolean(b) => Ok(Value::Bool(b)),
            LuaValue::Integer(i) => Ok(Value::Number(serde_json::Number::from(i))),
            LuaValue::Number(n) => {
                if let Some(num) = serde_json::Number::from_f64(n) {
                    Ok(Value::Number(num))
                } else {
                    Ok(Value::Number(serde_json::Number::from(0)))
                }
            },
            LuaValue::String(s) => {
                match s.to_str() {
                    Ok(str_val) => Ok(Value::String(str_val.to_string())),
                    Err(_) => Ok(Value::String("[invalid utf8]".to_string()))
                }
            },
            LuaValue::Table(table) => {
                // Collect all pairs first to avoid ownership issues
                let pairs_result: Result<Vec<_>, _> = table.pairs::<LuaValue, LuaValue>().collect();
                let pairs = match pairs_result {
                    Ok(p) => p,
                    Err(_) => return Ok(Value::Object(serde_json::Map::new()))
                };
                
                // Determine if table is array-like or object-like
                let mut is_array = true;
                let mut max_index = 0i64;
                let mut count = 0;
                
                for (key, _) in &pairs {
                    match key {
                        LuaValue::Integer(i) if *i > 0 => {
                            max_index = max_index.max(*i);
                            count += 1;
                        },
                        _ => {
                            is_array = false;
                            break;
                        }
                    }
                }
                
                if is_array && count == max_index && count > 0 {
                    // Create array - extract values from pairs we already collected
                    let mut array_items: Vec<(i64, LuaValue)> = pairs.into_iter()
                        .filter_map(|(key, value)| {
                            if let LuaValue::Integer(i) = key {
                                Some((i, value))
                            } else {
                                None
                            }
                        })
                        .collect();
                    
                    array_items.sort_by_key(|(i, _)| *i);
                    
                    let mut array = Vec::with_capacity(count as usize);
                    for (_, value) in array_items {
                        array.push(Self::lua_to_json_with_depth(value, depth + 1)?);
                    }
                    Ok(Value::Array(array))
                } else {
                    // Create object
                    let mut object = serde_json::Map::new();
                    for (key, value) in pairs {
                        let key_str = match key {
                            LuaValue::String(s) => s.to_str().unwrap_or("[invalid key]").to_string(),
                            LuaValue::Integer(i) => i.to_string(),
                            LuaValue::Number(n) => n.to_string(),
                            _ => "[unsupported key]".to_string()
                        };
                        object.insert(key_str, Self::lua_to_json_with_depth(value, depth + 1)?);
                    }
                    Ok(Value::Object(object))
                }
            },
            _ => Ok(Value::String("[unsupported lua type]".to_string()))
        }
    }
    
    fn execute_lua_script(&self, script: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Ensure Lua is initialized in this thread
        Self::ensure_lua_initialized()?;
        
        LUA_STATE.with(|state| {
            let state = state.borrow();
            let lua = state.as_ref().unwrap();
            
            // Convert input to Lua value
            let lua_input = Self::json_to_lua(lua, input)
                .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JSON to Lua conversion error: {}", e)))?;
            
            // Set request as global variable
            lua.globals().set("request", lua_input)
                .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Setting request variable error: {}", e)))?;
            
            // Execute the script
            let result: LuaValue = lua.load(script).eval()
                .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Lua execution error: {}", e)))?;
            
            // Convert result back to JSON
            Self::lua_to_json_with_depth(result, 0)
        })
    }
}
#[derive(Debug)]
pub struct LogMiddleware;

#[async_trait]
impl Middleware for JqMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Convert input to JSON string for jq processing
        let input_json = serde_json::to_string(input)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Input serialization error: {}", e)))?;
        
        // Execute the jq program
        let result_json = self.execute_jq(config, &input_json)?;
        
        // Parse the result back to serde_json::Value
        serde_json::from_str(&result_json)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("JQ result parse error: {}", e)))
    }
}

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "auth",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for ValidateMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "validate",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for PgMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        let sql = config.trim();
        if sql.is_empty() {
            return Err(WebPipeError::DatabaseError("Empty SQL config for pg middleware".to_string()));
        }

        let pool = get_pg_pool().await?;

        // Build parameter list from input.sqlParams
        let params: Vec<String> = input
            .get("sqlParams")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(json_value_to_bind_string).collect())
            .unwrap_or_default();

        // Detect SELECT queries (very simple heuristic)
        let is_select = sql.to_lowercase().trim_start().starts_with("select");

        if is_select {
            // Wrap query to get JSON array result using row_to_json/json_agg
            let wrapped_sql = format!(
                "SELECT COALESCE(json_agg(row_to_json(t)), '[]'::json) AS rows FROM ({}) t",
                sql
            );

            let mut query = sqlx::query_scalar::<_, sqlx::types::Json<Value>>(&wrapped_sql);
            for p in &params {
                query = query.bind(p);
            }

            let rows_json = match query.fetch_one(pool).await {
                Ok(json) => json.0,
                Err(e) => {
                    return Ok(build_sql_error_value(&e, sql));
                }
            };

            let row_count = rows_json.as_array().map(|a| a.len()).unwrap_or(0);

            let mut output = input.clone();
            let result_name = input.get("resultName").and_then(|v| v.as_str());
            let result_value = serde_json::json!({
                "rows": rows_json,
                "rowCount": row_count,
            });

            match result_name {
                Some(name) => {
                    // Ensure .data is an object
                    let data_obj = output
                        .as_object_mut()
                        .expect("input must be a JSON object for pg middleware");
                    let data_entry = data_obj.entry("data").or_insert_with(|| Value::Object(serde_json::Map::new()));
                    if !data_entry.is_object() {
                        *data_entry = Value::Object(serde_json::Map::new());
                    }
                    if let Some(map) = data_entry.as_object_mut() {
                        map.insert(name.to_string(), result_value);
                    }
                    Ok(output)
                }
                None => {
                    // Set/replace top-level data
                    if let Some(obj) = output.as_object_mut() {
                        obj.insert("data".to_string(), result_value);
                    }
                    Ok(output)
                }
            }
        } else {
            // Non-SELECT: execute and return affected row count
            let mut query = sqlx::query(sql);
            for p in &params {
                query = query.bind(p);
            }

            let result = match query.execute(pool).await {
                Ok(res) => res,
                Err(e) => {
                    return Ok(build_sql_error_value(&e, sql));
                }
            };

            let mut output = input.clone();
            let result_value = serde_json::json!({
                "rows": [],
                "rowCount": result.rows_affected(),
            });
            if let Some(obj) = output.as_object_mut() {
                obj.insert("data".to_string(), result_value);
            }
            Ok(output)
        }
    }
}

// --- PG pool management and helpers ---

static PG_POOL: OnceCell<PgPool> = OnceCell::new();

async fn get_pg_pool() -> Result<&'static PgPool, WebPipeError> {
    if let Some(pool) = PG_POOL.get() {
        return Ok(pool);
    }

    let database_url = build_database_url_from_env().map_err(|e| WebPipeError::ConfigError(e))?;

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await
        .map_err(|e| WebPipeError::DatabaseError(format!("Failed to connect to database: {}", e)))?;

    let _ = PG_POOL.set(pool);
    Ok(PG_POOL.get().expect("PG_POOL just set"))
}

fn build_database_url_from_env() -> Result<String, String> {
    if let Ok(url) = env::var("DATABASE_URL") {
        return Ok(url);
    }

    let host = env::var("WP_PG_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port = env::var("WP_PG_PORT").unwrap_or_else(|_| "5432".to_string());
    let database = env::var("WP_PG_DATABASE").unwrap_or_else(|_| "postgres".to_string());
    let user = env::var("WP_PG_USER").unwrap_or_else(|_| "postgres".to_string());
    let password = env::var("WP_PG_PASSWORD").unwrap_or_else(|_| "postgres".to_string());

    Ok(format!(
        "postgres://{}:{}@{}:{}/{}",
        urlencode(&user),
        urlencode(&password),
        host,
        port,
        database
    ))
}

fn urlencode(input: &str) -> String {
    // Basic percent-encoding for URL components
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    utf8_percent_encode(input, NON_ALPHANUMERIC).to_string()
}

fn json_value_to_bind_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(v).unwrap_or_default(),
    }
}

fn build_sql_error_value(e: &sqlx::Error, query: &str) -> Value {
    use serde_json::Map;
    let mut err_obj = Map::new();
    err_obj.insert("type".to_string(), Value::String("sqlError".to_string()));

    let mut message = e.to_string();
    let mut sqlstate: Option<String> = None;
    let mut severity: Option<String> = None;

    if let Some(db_err) = e.as_database_error() {
        message = db_err.message().to_string();
        if let Some(code) = db_err.code() {
            sqlstate = Some(code.to_string());
        }
        // Try to get severity for Postgres
        #[allow(unused_imports)]
        use sqlx::postgres::PgDatabaseError;
        if let Some(pg_err) = db_err.try_downcast_ref::<PgDatabaseError>() {
            severity = Some(format!("{:?}", pg_err.severity()));
        }
    }

    err_obj.insert("message".to_string(), Value::String(message));
    if let Some(code) = sqlstate { err_obj.insert("sqlstate".to_string(), Value::String(code)); }
    if let Some(sev) = severity { err_obj.insert("severity".to_string(), Value::String(sev)); }
    err_obj.insert("query".to_string(), Value::String(query.to_string()));

    let errors = Value::Array(vec![Value::Object(err_obj)]);
    let mut root = Map::new();
    root.insert("errors".to_string(), errors);
    Value::Object(root)
}

#[async_trait]
impl Middleware for HandlebarsMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Render the handlebars template with the input data
        let rendered = self.render_template(config, input)?;
        
        // Return the rendered HTML as a JSON string value
        Ok(Value::String(rendered))
    }
}

#[async_trait]
impl Middleware for FetchMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "fetch",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for CacheMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        Ok(serde_json::json!({
            "middleware": "cache",
            "config": config,
            "input": input
        }))
    }
}

#[async_trait]
impl Middleware for LuaMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        self.execute_lua_script(config, input)
    }
}

#[async_trait]
impl Middleware for LogMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder implementation
        println!("LOG: {} -> {:?}", config, input);
        Ok(input.clone())
    }
}