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
    async fn execute(
        &self,
        args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
        target_name: Option<&str>,
    ) -> Result<(), WebPipeError> {
        let sql = config.trim();
        if sql.is_empty() { return Err(WebPipeError::DatabaseError("Empty SQL config for pg middleware".to_string())); }

        // Check if we have a database connection (configured via config block)
        let pool = self.ctx.pg.as_ref().ok_or_else(||
            WebPipeError::DatabaseError("Database not configured. Please add a 'config pg {}' block with connection settings.".to_string())
        )?;

        // Determine parameters based on inline args vs fallback state
        let params: Vec<Value> = if !args.is_empty() {
            // New syntax: pg(params_expr)
            // Evaluate arg[0] for parameters array
            let params_value = crate::runtime::jq::evaluate(&args[0], &pipeline_ctx.state)?;
            params_value.as_array()
                .ok_or_else(|| WebPipeError::MiddlewareExecutionError(
                    format!("pg argument 0 must evaluate to an array, got: {:?}", params_value)
                ))?
                .to_vec()
        } else {
            // Old syntax: fallback to sqlParams from state
            pipeline_ctx.state.get("sqlParams").and_then(|v| v.as_array()).map(|arr| arr.to_vec()).unwrap_or_default()
        };

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
                Err(e) => {
                    pipeline_ctx.state = build_sql_error_value(&e, sql);
                    return Ok(());
                }
            };

            let row_count = rows_json.as_array().map(|a| a.len()).unwrap_or(0);

            let result_value = serde_json::json!({ "rows": rows_json, "rowCount": row_count });

            // Return raw data when target_name present (executor wraps it)
            // Otherwise, use legacy format for backwards compatibility
            if target_name.is_some() {
                pipeline_ctx.state = result_value;
            } else {
                pipeline_ctx.state = serde_json::json!({
                    "data": result_value
                });
            }
            Ok(())
        } else {
            let mut query = sqlx::query(sql);
            for p in &params { query = bind_json_param(query, p); }

            let result = match query.execute(pool).await {
                Ok(res) => res,
                Err(e) => {
                    pipeline_ctx.state = build_sql_error_value(&e, sql);
                    return Ok(());
                }
            };

            let result_value = serde_json::json!({ "rows": [], "rowCount": result.rows_affected() });

            if target_name.is_some() {
                pipeline_ctx.state = result_value;
            } else {
                pipeline_ctx.state = serde_json::json!({
                    "data": result_value
                });
            }
            Ok(())
        }
    }

    fn behavior(&self) -> super::StateBehavior {
        // PG middleware acts as Transform - it replaces state with query results
        // When not terminal, context is preserved. When terminal, clean output.
        super::StateBehavior::Transform
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

