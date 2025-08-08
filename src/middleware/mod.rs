use crate::error::WebPipeError;
use crate::ast::Variable;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::cell::RefCell;
use std::sync::{Arc, Mutex};
use lru::LruCache;
use handlebars::Handlebars;
use mlua::{Lua, Value as LuaValue, Result as LuaResult};
use once_cell::sync::OnceCell;
use sqlx::{self, postgres::PgPoolOptions, PgPool};
use std::time::Duration;
use reqwest::{self, Client, Method};
use reqwest::header::{HeaderMap as ReqwestHeaderMap, HeaderName, HeaderValue, USER_AGENT};
use rand::RngCore;
use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use argon2::password_hash::{SaltString, PasswordHash, rand_core::OsRng};
// no regex needed now

static HANDLEBARS_PARTIALS: OnceCell<std::collections::HashMap<String, String>> = OnceCell::new();

pub fn configure_handlebars_partials(variables: &[Variable]) {
    let mut map = std::collections::HashMap::new();
    for v in variables {
        if v.var_type == "mustache" || v.var_type == "handlebars" {
            map.insert(v.name.clone(), v.value.clone());
        }
    }
    let _ = HANDLEBARS_PARTIALS.set(map);
}

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
         registry.register("debug", Box::new(DebugMiddleware));
        
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
    // Register global partials only once per process
    partials_registered: Arc<Mutex<bool>>,
    // Track templates we've compiled/registered by a stable id
    registered_templates: Arc<Mutex<HashSet<String>>>,
}

impl HandlebarsMiddleware {
    pub fn new() -> Self {
        Self {
            handlebars: Arc::new(Mutex::new(Handlebars::new())),
            partials_registered: Arc::new(Mutex::new(false)),
            registered_templates: Arc::new(Mutex::new(HashSet::new())),
        }
    }
    
    fn render_template(&self, template: &str, data: &Value) -> Result<String, WebPipeError> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut handlebars = self.handlebars.lock().unwrap();

        // One-time registration of global partials
        if let Ok(mut flag) = self.partials_registered.lock() {
            if !*flag {
                if let Some(partials) = HANDLEBARS_PARTIALS.get() {
                    for (name, tpl) in partials {
                        let n = name.trim();
                        let _ = handlebars.register_partial(n, tpl);
                    }
                }
                *flag = true;
            }
        }

        // Compile/register this template string only once, render by id thereafter
        let tpl = template.trim().to_string();
        let mut hasher = DefaultHasher::new();
        tpl.hash(&mut hasher);
        let id = format!("tpl_{:x}", hasher.finish());

        let mut compiled = self.registered_templates.lock().unwrap();
        if !compiled.contains(&id) {
            if let Err(e) = handlebars.register_template_string(&id, &tpl) {
                return Err(WebPipeError::MiddlewareExecutionError(format!("Handlebars template compile error: {}", e)));
            }
            compiled.insert(id.clone());
        }

        handlebars
            .render(&id, data)
            .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Handlebars render error: {}", e)))
    }
}

// Preprocessor removed: templates are now authored in native Handlebars
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
                
                // Inject getEnv function to read environment variables
                let get_env_fn = lua
                    .create_function(|lua, key: String| {
                        match std::env::var(&key) {
                            Ok(val) => Ok(LuaValue::String(lua.create_string(&val)?)),
                            Err(_) => Ok(LuaValue::Nil),
                        }
                    })
                    .map_err(|e| WebPipeError::MiddlewareExecutionError(format!("Failed to create getEnv: {}", e)))?;
                if let Err(e) = lua.globals().set("getEnv", get_env_fn) {
                    return Err(WebPipeError::MiddlewareExecutionError(format!("Failed to register getEnv: {}", e)));
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
#[derive(Debug)]
pub struct DebugMiddleware;

// --- Auth helpers and implementation ---

#[derive(Debug, Clone)]
struct AuthConfig {
    session_ttl: i64,
    cookie_name: String,
    cookie_secure: bool,
    cookie_http_only: bool,
    cookie_same_site: String,
    cookie_path: String,
}

fn get_auth_config() -> AuthConfig {
    // In this Rust runtime we do not yet wire global config per README; use sensible defaults matching README examples
    AuthConfig {
        session_ttl: 604800,
        cookie_name: "wp_session".to_string(),
        cookie_secure: false,
        cookie_http_only: true,
        cookie_same_site: "Lax".to_string(),
        cookie_path: "/".to_string(),
    }
}

fn build_set_cookie_header(token: &str) -> String {
    let cfg = get_auth_config();
    let mut parts = vec![format!("{}={}", cfg.cookie_name, token)];
    if cfg.cookie_http_only { parts.push("HttpOnly".to_string()); }
    // For local dev over http, do not force Secure or the browser will drop it
    if cfg.cookie_secure { parts.push("Secure".to_string()); }
    parts.push(format!("SameSite={}", cfg.cookie_same_site));
    parts.push(format!("Path={}", cfg.cookie_path));
    parts.push(format!("Max-Age={}", cfg.session_ttl));
    parts.join("; ")
}

fn build_clear_cookie_header() -> String {
    let cfg = get_auth_config();
    let mut parts = vec![format!("{}=", cfg.cookie_name)];
    parts.push("HttpOnly".to_string());
    if cfg.cookie_secure { parts.push("Secure".to_string()); }
    parts.push("SameSite=Strict".to_string());
    parts.push(format!("Path={}", cfg.cookie_path));
    parts.push("Max-Age=0".to_string());
    parts.join("; ")
}

fn build_auth_error_object(message: &str, _context: &str) -> Value {
    serde_json::json!({
        "errors": [
            { "type": "authError", "message": message }
        ]
    })
}

fn extract_body_field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get("body")?.get(key)?.as_str()
}

async fn auth_login(input: &Value) -> Result<Value, WebPipeError> {
    let login = extract_body_field(input, "login").or_else(|| extract_body_field(input, "username"));
    let password = extract_body_field(input, "password");
    if login.is_none() || password.is_none() {
        return Ok(build_auth_error_object("Missing login or password", "login"));
    }
    let login = login.unwrap();
    let password = password.unwrap();

    let pool = get_pg_pool().await?;
    // fetch user by login
    let user_row: Option<(i32, String, String, String, String)> = sqlx::query_as(
        "SELECT id, login, password_hash, email, COALESCE(type,'local') as type FROM users WHERE login = $1 AND COALESCE(status,'active') = 'active' LIMIT 1"
    )
        .bind(login)
        .fetch_optional(pool)
        .await
        .map_err(|e| WebPipeError::DatabaseError(e.to_string()))?;

    let (user_id, _login, password_hash, email, user_type) = match user_row {
        Some(row) => row,
        None => return Ok(build_auth_error_object("Invalid credentials", "login")),
    };

    // verify password
    match PasswordHash::new(&password_hash)
        .ok()
        .and_then(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).ok())
    {
        Some(_) => {}
        None => return Ok(build_auth_error_object("Invalid credentials", "login")),
    }

    // create session token
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);

    // expires_at now + ttl
    let ttl = get_auth_config().session_ttl;
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(ttl);

    // store session
    let _ = sqlx::query("INSERT INTO sessions (user_id, token, expires_at) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(&token)
        .bind(expires_at)
        .execute(pool)
        .await
        .map_err(|e| WebPipeError::DatabaseError(e.to_string()))?;

    // build response: clone input and set user + setCookies
    let mut result = input.clone();
    if let Some(obj) = result.as_object_mut() {
        obj.insert(
            "user".to_string(),
            serde_json::json!({
                "id": user_id,
                "login": login,
                "email": email,
                "type": user_type,
                "status": "active"
            }),
        );
        let cookie = build_set_cookie_header(&token);
        if let Some(existing) = obj.get_mut("setCookies") {
            if let Some(arr) = existing.as_array_mut() {
                arr.push(Value::String(cookie));
            }
        } else {
            obj.insert("setCookies".to_string(), Value::Array(vec![Value::String(cookie)]));
        }
    }
    Ok(result)
}

async fn auth_register(input: &Value) -> Result<Value, WebPipeError> {
    let login = extract_body_field(input, "login");
    let email = extract_body_field(input, "email");
    let password = extract_body_field(input, "password");
    if login.is_none() || email.is_none() || password.is_none() {
        return Ok(build_auth_error_object("Missing required fields: login, email, password", "register"));
    }
    let login = login.unwrap();
    let email = email.unwrap();
    let password = password.unwrap();

    let pool = get_pg_pool().await?;

    // check if exists
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE login = $1 LIMIT 1")
        .bind(login)
        .fetch_optional(pool)
        .await
        .map_err(|e| WebPipeError::DatabaseError(e.to_string()))?;
    if exists.is_some() {
        return Ok(build_auth_error_object("User already exists", "register"));
    }

    // hash password
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| WebPipeError::InternalError(e.to_string()))?
        .to_string();

    // create user
    let row: (i32, String, String) = sqlx::query_as(
        "INSERT INTO users (login, email, password_hash) VALUES ($1, $2, $3) RETURNING id, login, email"
    )
        .bind(login)
        .bind(email)
        .bind(password_hash)
        .fetch_one(pool)
        .await
        .map_err(|e| WebPipeError::DatabaseError(e.to_string()))?;

    let (id, login_ret, email_ret) = row;
    let mut result = input.clone();
    if let Some(obj) = result.as_object_mut() {
        obj.insert(
            "user".to_string(),
            serde_json::json!({
                "id": id,
                "login": login_ret,
                "email": email_ret,
                "type": "local",
                "status": "active"
            }),
        );
        obj.insert("message".to_string(), Value::String("User registration successful".to_string()));
    }
    Ok(result)
}

async fn auth_required(input: &Value) -> Result<Value, WebPipeError> {
    let cookie_name = get_auth_config().cookie_name;
    let token_opt = input
        .get("cookies")
        .and_then(|v| v.get(&cookie_name))
        .and_then(|v| v.as_str());
    let token = match token_opt { Some(t) if !t.is_empty() => t, _ => return Ok(build_auth_error_object("Authentication required", "required")) };

    let user_info = lookup_session_user(token).await?;
    match user_info {
        Some((user_id, login, email, user_type)) => {
            let mut result = input.clone();
            if let Some(obj) = result.as_object_mut() {
                obj.insert(
                    "user".to_string(),
                    serde_json::json!({
                        "id": user_id, "login": login, "email": email, "type": user_type
                    }),
                );
            }
            Ok(result)
        }
        None => Ok(build_auth_error_object("Invalid session", "required")),
    }
}

async fn auth_optional(input: &Value) -> Result<Value, WebPipeError> {
    let cookie_name = get_auth_config().cookie_name;
    let token_opt = input
        .get("cookies")
        .and_then(|v| v.get(&cookie_name))
        .and_then(|v| v.as_str());
    if let Some(token) = token_opt {
        if let Some((user_id, login, email, user_type)) = lookup_session_user(token).await? {
            let mut result = input.clone();
            if let Some(obj) = result.as_object_mut() {
                obj.insert(
                    "user".to_string(),
                    serde_json::json!({
                        "id": user_id, "login": login, "email": email, "type": user_type
                    }),
                );
            }
            return Ok(result);
        }
    }
    Ok(input.clone())
}

async fn auth_logout(input: &Value) -> Result<Value, WebPipeError> {
    let cookie_name = get_auth_config().cookie_name;
    let token_opt = input
        .get("cookies")
        .and_then(|v| v.get(&cookie_name))
        .and_then(|v| v.as_str());
    if let Some(token) = token_opt {
        let pool = get_pg_pool().await?;
        let _ = sqlx::query("DELETE FROM sessions WHERE token = $1")
            .bind(token)
            .execute(pool)
            .await;
    }
    let mut result = input.clone();
    if let Some(obj) = result.as_object_mut() {
        let clear_cookie = build_clear_cookie_header();
        obj.insert("setCookies".to_string(), Value::Array(vec![Value::String(clear_cookie)]));
    }
    Ok(result)
}

async fn auth_type_check(input: &Value, required_type: &str) -> Result<Value, WebPipeError> {
    let with_user = auth_required(input).await?;
    let user_type = with_user.get("user").and_then(|u| u.get("type")).and_then(|v| v.as_str());
    if user_type == Some(required_type) {
        Ok(with_user)
    } else {
        Ok(build_auth_error_object("Insufficient permissions", "type"))
    }
}

async fn lookup_session_user(token: &str) -> Result<Option<(i32, String, String, String)>, WebPipeError> {
    let pool = get_pg_pool().await?;
    let row: Option<(i32, String, String, String)> = sqlx::query_as(
        "SELECT u.id, u.login, u.email, COALESCE(u.type,'local') as type FROM sessions s JOIN users u ON s.user_id = u.id WHERE s.token = $1 AND s.expires_at > NOW() LIMIT 1"
    )
        .bind(token)
        .fetch_optional(pool)
        .await
        .map_err(|e| WebPipeError::DatabaseError(e.to_string()))?;
    Ok(row)
}

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
        let config = config.trim();
        match config {
            "login" => auth_login(input).await,
            "logout" => auth_logout(input).await,
            "register" => auth_register(input).await,
            "required" => auth_required(input).await,
            "optional" => auth_optional(input).await,
            _ => {
                if let Some(rest) = config.strip_prefix("type:") {
                    auth_type_check(input, rest.trim()).await
                } else {
                    Ok(build_auth_error_object("Invalid auth configuration", config))
                }
            }
        }
    }
}

#[async_trait]
impl Middleware for ValidateMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder: pass-through to avoid clobbering accumulated data
        let _ = config; // suppress unused warning
        Ok(input.clone())
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

        // Build parameter list from input.sqlParams (preserve native types)
        let params: Vec<Value> = input
            .get("sqlParams")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().cloned().collect())
            .unwrap_or_default();

        // Detect SELECT queries (very simple heuristic)
        let lowered = sql.to_lowercase();
        let is_select = lowered.trim_start().starts_with("select");
        let has_returning = lowered.contains(" returning ");

        if is_select || has_returning {
            // Use CTE to support SELECT and DML with RETURNING
            let wrapped_sql = format!(
                "WITH t AS ({}) SELECT COALESCE(json_agg(row_to_json(t)), '[]'::json) AS rows FROM t",
                sql
            );

            let mut query = sqlx::query_scalar::<_, sqlx::types::Json<Value>>(&wrapped_sql);
            for p in &params { query = bind_json_param_scalar(query, p); }

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
            for p in &params { query = bind_json_param(query, p); }

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

    // Read resolved pg config directly (handles $ENV || default)
    let cm = crate::config::global();
    let pg_cfg = cm.resolve_config_as_json("pg")?;

    let host = pg_cfg.get("host").and_then(|v| v.as_str()).unwrap_or("localhost");
    let port = pg_cfg.get("port").and_then(|v| v.as_i64()).unwrap_or(5432);
    let database = pg_cfg.get("database").and_then(|v| v.as_str()).unwrap_or("postgres");
    let user = pg_cfg.get("user").and_then(|v| v.as_str()).unwrap_or("postgres");
    let password = pg_cfg.get("password").and_then(|v| v.as_str()).unwrap_or("postgres");

    // Pool sizing from config with sensible defaults
    let max_conns: u32 = pg_cfg.get("maxPoolSize").and_then(|v| v.as_i64()).unwrap_or(20).max(1) as u32;
    let mut min_conns: u32 = pg_cfg.get("initialPoolSize").and_then(|v| v.as_i64()).unwrap_or(5).max(1) as u32;
    if min_conns > max_conns { min_conns = max_conns; }

    let database_url = format!(
        "postgres://{}:{}@{}:{}/{}",
        urlencode(user),
        urlencode(password),
        host,
        port,
        database
    );

    let pool = PgPoolOptions::new()
        .max_connections(max_conns)
        .min_connections(min_conns)
        .acquire_timeout(Duration::from_secs(3))
        .connect(&database_url)
        .await
        .map_err(|e| WebPipeError::DatabaseError(format!("Failed to connect to database: {}", e)))?;

    let _ = PG_POOL.set(pool);
    Ok(PG_POOL.get().expect("PG_POOL just set"))
}

// old env-based URL builder removed; config-driven URL is constructed in get_pg_pool

fn urlencode(input: &str) -> String {
    // Basic percent-encoding for URL components
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    utf8_percent_encode(input, NON_ALPHANUMERIC).to_string()
}

fn bind_json_param<'q>(query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>, v: &'q Value)
    -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>
{
    let mut query = query;
    match v {
        Value::Null => {
            let none: Option<i32> = None; query.bind(none)
        },
        Value::Bool(b) => query.bind(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() { query.bind(i) }
            else if let Some(u) = n.as_u64() { query.bind(u as i64) }
            else if let Some(f) = n.as_f64() { query.bind(f) }
            else { query.bind(n.to_string()) }
        },
        Value::String(s) => query.bind(s.as_str()),
        Value::Array(_) | Value::Object(_) => query.bind(sqlx::types::Json(v.clone())),
    }
}

fn bind_json_param_scalar<'q>(query: sqlx::query::QueryScalar<'q, sqlx::Postgres, sqlx::types::Json<Value>, sqlx::postgres::PgArguments>, v: &'q Value)
    -> sqlx::query::QueryScalar<'q, sqlx::Postgres, sqlx::types::Json<Value>, sqlx::postgres::PgArguments>
{
    let mut query = query;
    match v {
        Value::Null => {
            let none: Option<i32> = None; query.bind(none)
        },
        Value::Bool(b) => query.bind(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() { query.bind(i) }
            else if let Some(u) = n.as_u64() { query.bind(u as i64) }
            else if let Some(f) = n.as_f64() { query.bind(f) }
            else { query.bind(n.to_string()) }
        },
        Value::String(s) => query.bind(s.as_str()),
        Value::Array(_) | Value::Object(_) => query.bind(sqlx::types::Json(v.clone())),
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
        // Determine URL (input.fetchUrl overrides config)
        let url = input
            .get("fetchUrl")
            .and_then(|v| v.as_str())
            .unwrap_or(config)
            .trim()
            .to_string();

        if url.is_empty() {
            return Ok(build_fetch_error_object(
                "networkError",
                serde_json::json!({
                    "message": "Missing URL for fetch middleware",
                }),
            ));
        }

        // HTTP method (default GET)
        let method_str = input
            .get("fetchMethod")
            .and_then(|v| v.as_str())
            .unwrap_or("GET");
        let method = Method::from_bytes(method_str.as_bytes())
            .unwrap_or(Method::GET);

        // Build client and request
        let client = get_http_client();
        let mut req_builder = client.request(method, &url);

        // Per-request timeout
        if let Some(timeout_secs) = input.get("fetchTimeout").and_then(|v| v.as_u64()) {
            req_builder = req_builder.timeout(Duration::from_secs(timeout_secs));
        }

        // Headers (ensure a default User-Agent)
        let mut headers = ReqwestHeaderMap::new();
        if let Some(headers_obj) = input.get("fetchHeaders").and_then(|v| v.as_object()) {
            for (k, v) in headers_obj {
                if let Some(val_str) = v.as_str() {
                    if let (Ok(name), Ok(value)) = (
                        HeaderName::from_bytes(k.as_bytes()),
                        HeaderValue::from_str(val_str),
                    ) {
                        headers.insert(name, value);
                    }
                }
            }
        }
        if !headers.contains_key(USER_AGENT) {
            headers.insert(USER_AGENT, HeaderValue::from_static("WebPipe/1.0"));
        }
        req_builder = req_builder.headers(headers);

        // Body
        if let Some(body) = input.get("fetchBody") {
            req_builder = req_builder.json(body);
        }

        // Execute request
        let response = match req_builder.send().await {
            Ok(resp) => resp,
            Err(err) => {
                if err.is_timeout() {
                    return Ok(build_fetch_error_object(
                        "timeoutError",
                        serde_json::json!({
                            "message": err.to_string(),
                        }),
                    ));
                } else {
                    return Ok(build_fetch_error_object(
                        "networkError",
                        serde_json::json!({
                            "message": err.to_string(),
                            "url": url,
                        }),
                    ));
                }
            }
        };

        let status = response.status();
        let status_code = status.as_u16();
        let headers_map: HashMap<String, String> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();

        let body_text = match response.text().await {
            Ok(t) => t,
            Err(err) => {
                return Ok(build_fetch_error_object(
                    "networkError",
                    serde_json::json!({
                        "message": format!("Failed to read response body: {}", err),
                        "url": url,
                    }),
                ));
            }
        };

        if !status.is_success() {
            // HTTP error path
            return Ok(build_fetch_error_object(
                "httpError",
                serde_json::json!({
                    "status": status_code,
                    "message": body_text,
                    "url": url,
                }),
            ));
        }

        // Parse response body as JSON, fallback to string
        let response_body: Value = serde_json::from_str(&body_text)
            .unwrap_or(Value::String(body_text));

        // Prepare output format
        let mut output = input.clone();
        let result_name_opt = input.get("resultName").and_then(|v| v.as_str());
        let result_payload = serde_json::json!({
            "response": response_body,
            "status": status_code,
            "headers": headers_map
        });

        match result_name_opt {
            Some(name) => {
                // Ensure .data is an object
                if let Some(obj) = output.as_object_mut() {
                    let data_entry = obj.entry("data").or_insert_with(|| Value::Object(serde_json::Map::new()));
                    if !data_entry.is_object() {
                        *data_entry = Value::Object(serde_json::Map::new());
                    }
                    if let Some(data_obj) = data_entry.as_object_mut() {
                        data_obj.insert(name.to_string(), result_payload);
                    }
                }
            }
            None => {
                if let Some(obj) = output.as_object_mut() {
                    obj.insert("data".to_string(), result_payload);
                }
            }
        }

        Ok(output)
    }
}

// --- Fetch helpers ---

fn build_fetch_error_object(error_type: &str, mut details: Value) -> Value {
    // Ensure details is an object to which we can add fields
    if !details.is_object() {
        details = serde_json::json!({ "message": details.to_string() });
    }
    let mut err_obj = serde_json::Map::new();
    err_obj.insert("type".to_string(), Value::String(error_type.to_string()));
    if let Some(map) = details.as_object() {
        for (k, v) in map {
            err_obj.insert(k.clone(), v.clone());
        }
    }

    Value::Object(
        [
            (
                "errors".to_string(),
                Value::Array(vec![Value::Object(err_obj)]),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

fn get_http_client() -> &'static Client {
    static CLIENT: OnceCell<Client> = OnceCell::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            // Do not set a global timeout; allow per-request overrides
            .build()
            .expect("failed to build reqwest client")
    })
}

#[async_trait]
impl Middleware for CacheMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Placeholder: pass-through to avoid clobbering accumulated data
        let _ = config; // suppress unused warning
        Ok(input.clone())
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

#[async_trait]
impl Middleware for DebugMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        let label = {
            let trimmed = config.trim();
            if trimmed.is_empty() { "debug" } else { trimmed }
        };
        println!("{}", label);
        match serde_json::to_string_pretty(input) {
            Ok(pretty) => println!("{}", pretty),
            Err(_) => println!("{}", input),
        }
        Ok(input.clone())
    }
}