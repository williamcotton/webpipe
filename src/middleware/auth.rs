use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use rand::RngCore;
use argon2::{Argon2, PasswordVerifier, PasswordHasher};
use argon2::password_hash::{PasswordHash, SaltString, rand_core::OsRng};

#[derive(Debug)]
pub struct AuthMiddleware { pub(crate) ctx: Arc<Context> }

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
        "errors": [ { "type": "authError", "message": message } ]
    })
}

fn extract_body_field<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
    input.get("body")?.get(key)?.as_str()
}

async fn auth_login(pool: &PgPool, input: &Value) -> Result<Value, WebPipeError> {
    let login = extract_body_field(input, "login").or_else(|| extract_body_field(input, "username"));
    let password = extract_body_field(input, "password");
    if login.is_none() || password.is_none() {
        return Ok(build_auth_error_object("Missing login or password", "login"));
    }
    let login = login.unwrap();
    let password = password.unwrap();

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

    match PasswordHash::new(&password_hash)
        .ok()
        .and_then(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).ok())
    {
        Some(_) => {}
        None => return Ok(build_auth_error_object("Invalid credentials", "login")),
    }

    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);

    let ttl = get_auth_config().session_ttl;
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(ttl);

    let _ = sqlx::query("INSERT INTO sessions (user_id, token, expires_at) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(&token)
        .bind(expires_at)
        .execute(pool)
        .await
        .map_err(|e| WebPipeError::DatabaseError(e.to_string()))?;

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

async fn auth_register(pool: &PgPool, input: &Value) -> Result<Value, WebPipeError> {
    let login = extract_body_field(input, "login");
    let email = extract_body_field(input, "email");
    let password = extract_body_field(input, "password");
    if login.is_none() || email.is_none() || password.is_none() {
        return Ok(build_auth_error_object("Missing required fields: login, email, password", "register"));
    }
    let login = login.unwrap();
    let email = email.unwrap();
    let password = password.unwrap();

    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE login = $1 LIMIT 1")
        .bind(login)
        .fetch_optional(pool)
        .await
        .map_err(|e| WebPipeError::DatabaseError(e.to_string()))?;
    if exists.is_some() {
        return Ok(build_auth_error_object("User already exists", "register"));
    }

    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| WebPipeError::InternalError(e.to_string()))?
        .to_string();

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

async fn auth_required(pool: &PgPool, input: &Value) -> Result<Value, WebPipeError> {
    let cookie_name = get_auth_config().cookie_name;
    let token_opt = input
        .get("cookies")
        .and_then(|v| v.get(&cookie_name))
        .and_then(|v| v.as_str());
    let token = match token_opt { Some(t) if !t.is_empty() => t, _ => return Ok(build_auth_error_object("Authentication required", "required")) };

    let user_info = lookup_session_user(pool, token).await?;
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

async fn auth_optional(pool: &PgPool, input: &Value) -> Result<Value, WebPipeError> {
    let cookie_name = get_auth_config().cookie_name;
    let token_opt = input
        .get("cookies")
        .and_then(|v| v.get(&cookie_name))
        .and_then(|v| v.as_str());
    if let Some(token) = token_opt {
        if let Some((user_id, login, email, user_type)) = lookup_session_user(pool, token).await? {
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

async fn auth_logout(pool: &PgPool, input: &Value) -> Result<Value, WebPipeError> {
    let cookie_name = get_auth_config().cookie_name;
    let token_opt = input
        .get("cookies")
        .and_then(|v| v.get(&cookie_name))
        .and_then(|v| v.as_str());
    if let Some(token) = token_opt {
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

async fn auth_type_check(pool: &PgPool, input: &Value, required_type: &str) -> Result<Value, WebPipeError> {
    let with_user = auth_required(pool, input).await?;
    let user_type = with_user.get("user").and_then(|u| u.get("type")).and_then(|v| v.as_str());
    if user_type == Some(required_type) {
        Ok(with_user)
    } else {
        Ok(build_auth_error_object("Insufficient permissions", "type"))
    }
}

async fn lookup_session_user(pool: &PgPool, token: &str) -> Result<Option<(i32, String, String, String)>, WebPipeError> {
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
impl super::Middleware for AuthMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        let pool = self
            .ctx
            .pg
            .as_ref()
            .ok_or_else(|| WebPipeError::DatabaseError("database not configured".to_string()))?;
        let config = config.trim();
        match config {
            "login" => auth_login(pool, input).await,
            "logout" => auth_logout(pool, input).await,
            "register" => auth_register(pool, input).await,
            "required" => auth_required(pool, input).await,
            "optional" => auth_optional(pool, input).await,
            _ => {
                if let Some(rest) = config.strip_prefix("type:") {
                    auth_type_check(pool, input, rest.trim()).await
                } else {
                    Ok(build_auth_error_object("Invalid auth configuration", config))
                }
            }
        }
    }
}


