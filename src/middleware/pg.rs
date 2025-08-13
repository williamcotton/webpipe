use crate::error::WebPipeError;
use crate::runtime::Context;
use async_trait::async_trait;
use serde_json::Value;
use sqlx::{self};
use std::sync::Arc;

#[derive(Debug)]
pub struct PgMiddleware { pub(crate) ctx: Arc<Context> }

#[async_trait]
impl super::Middleware for PgMiddleware {
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        let sql = config.trim();
        if sql.is_empty() { return Err(WebPipeError::DatabaseError("Empty SQL config for pg middleware".to_string())); }

        // Check if we have a database connection (configured via config block)
        let pool = self.ctx.pg.as_ref().ok_or_else(||
            WebPipeError::DatabaseError("Database not configured. Please add a 'config pg {}' block with connection settings.".to_string())
        )?;

        let params: Vec<Value> = input.get("sqlParams").and_then(|v| v.as_array()).map(|arr| arr.to_vec()).unwrap_or_default();

        let lowered = sql.to_lowercase();
        let is_select = lowered.trim_start().starts_with("select");
        let has_returning = lowered.contains(" returning ");

        if is_select || has_returning {
            let wrapped_sql = format!(
                "WITH t AS ({}) SELECT COALESCE(json_agg(row_to_json(t)), '[]'::json) AS rows FROM t",
                sql
            );

            let mut query = sqlx::query_scalar::<_, sqlx::types::Json<Value>>(&wrapped_sql);
            for p in &params { query = bind_json_param_scalar(query, p); }

            let rows_json = match query.fetch_one(pool).await {
                Ok(json) => json.0,
                Err(e) => { return Ok(build_sql_error_value(&e, sql)); }
            };

            let row_count = rows_json.as_array().map(|a| a.len()).unwrap_or(0);

            let mut output = input.clone();
            let result_name = input.get("resultName").and_then(|v| v.as_str());
            let result_value = serde_json::json!({ "rows": rows_json, "rowCount": row_count });

            match result_name {
                Some(name) => {
                    let data_obj = output.as_object_mut().expect("input must be a JSON object for pg middleware");
                    let data_entry = data_obj.entry("data").or_insert_with(|| Value::Object(serde_json::Map::new()));
                    if !data_entry.is_object() { *data_entry = Value::Object(serde_json::Map::new()); }
                    if let Some(map) = data_entry.as_object_mut() { map.insert(name.to_string(), result_value); }
                    Ok(output)
                }
                None => {
                    if let Some(obj) = output.as_object_mut() { obj.insert("data".to_string(), result_value); }
                    Ok(output)
                }
            }
        } else {
            let mut query = sqlx::query(sql);
            for p in &params { query = bind_json_param(query, p); }

            let result = match query.execute(pool).await {
                Ok(res) => res,
                Err(e) => { return Ok(build_sql_error_value(&e, sql)); }
            };

            let mut output = input.clone();
            let result_value = serde_json::json!({ "rows": [], "rowCount": result.rows_affected() });
            if let Some(obj) = output.as_object_mut() { obj.insert("data".to_string(), result_value); }
            Ok(output)
        }
    }
}

fn bind_json_param<'q>(query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>, v: &'q Value)
    -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>
{
    match v {
        Value::Null => { let none: Option<i32> = None; query.bind(none) },
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
    match v {
        Value::Null => { let none: Option<i32> = None; query.bind(none) },
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
        if let Some(code) = db_err.code() { sqlstate = Some(code.to_string()); }
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


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_sql_error_object_shape() {
        // Build a synthetic sqlx::Error using a simple conversion from string via DatabaseError is non-trivial.
        // Instead, use a connection error to verify the envelope fields are present.
        let err = sqlx::Error::Configuration("bad config".into());
        let v = build_sql_error_value(&err, "SELECT 1");
        assert_eq!(v["errors"][0]["type"], serde_json::json!("sqlError"));
        assert_eq!(v["errors"][0]["query"], serde_json::json!("SELECT 1"));
    }
}

