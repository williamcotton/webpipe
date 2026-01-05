#![allow(unused)]
use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;

use webpipe::ast::{parse_program, Pipeline};
use webpipe::executor::{execute_pipeline, ExecutionEnv, ModuleRegistry, RealInvoker};
use webpipe::middleware::MiddlewareRegistry;
use webpipe::runtime::Context;
use webpipe::ast::Variable;

fn pg_test_program(table_name: &str) -> String {
    let host = std::env::var("PGHOST").unwrap_or_else(|_| "localhost".to_string());
    let port = std::env::var("PGPORT").unwrap_or_else(|_| "5432".to_string());
    let db = std::env::var("PGDATABASE").unwrap_or_else(|_| "postgres".to_string());
    let user = std::env::var("PGUSER").unwrap_or_else(|_| "postgres".to_string());
    let pass = std::env::var("PGPASSWORD").unwrap_or_else(|_| "postgres".to_string());
    format!(
        r#"
config pg {{
  host: "{host}"
  port: {port}
  database: "{db}"
  user: "{user}"
  password: "{pass}"
}}

pipeline doSelect =
  |> pg: "SELECT 1 AS one"

pipeline doInsertReturning =
  |> pg: "INSERT INTO {table} (name) VALUES ('a') RETURNING id, name"

pipeline doUpdate =
  |> pg: "UPDATE {table} SET name = 'b' WHERE name = 'a'"

pipeline doAuthRegister =
  |> auth: "register"

pipeline doAuthLogin =
  |> auth: "login"

pipeline doAuthRequired =
  |> auth: "required"

pipeline doAuthLogout =
  |> auth: "logout"
"#,
        host = host,
        port = port,
        db = db,
        user = user,
        pass = pass,
        table = table_name
    )
}

async fn build_env_with_ctx(program: &webpipe::ast::Program) -> (ExecutionEnv, Arc<Context>) {
    let ctx = Arc::new(
        Context::from_program_configs(program.configs.clone(), &program.variables)
            .await
            .expect("context"),
    );
    let registry = Arc::new(MiddlewareRegistry::with_builtins(ctx.clone()));

    let named: HashMap<(Option<usize>, String), Arc<Pipeline>> = program
        .pipelines
        .iter()
        .map(|p| ((None, p.name.clone()), Arc::new(p.pipeline.clone())))
        .collect();

    let variables_map: HashMap<(Option<usize>, String, String), Variable> = program.variables
        .iter()
        .map(|v| ((None, v.var_type.clone(), v.name.clone()), v.clone()))
        .collect();

    let env = ExecutionEnv {
        variables: Arc::new(variables_map),
        named_pipelines: Arc::new(named),
            imports: Arc::new(std::collections::HashMap::new()),
        invoker: Arc::new(RealInvoker::new(registry.clone())),
        registry: registry.clone(),
        environment: None,
        cache: ctx.cache.clone(),
        rate_limit: ctx.rate_limit.clone(),
        module_registry: Arc::new(ModuleRegistry::new()),
        #[cfg(feature = "debugger")]
        debugger: None,
    };
    (env, ctx)
}

async fn ensure_schema(ctx: &Context, table_name: &str) {
    let pool = ctx.pg.as_ref().expect("pg configured");
    // Ensure clean start to avoid sequence conflicts across runs
    let drop_stmt = format!("DROP TABLE IF EXISTS {} CASCADE", table_name);
    sqlx::query(&drop_stmt)
        .execute(pool)
        .await
        .unwrap();
    // Create simple table for pg tests using identity to avoid SERIAL sequence conflicts
    let create_stmt = format!(
        "CREATE TABLE {}(id INTEGER GENERATED ALWAYS AS IDENTITY PRIMARY KEY, name TEXT)",
        table_name
    );
    sqlx::query(&create_stmt)
    .execute(pool)
    .await
    .unwrap();

    // Create users/sessions tables for auth tests
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS users(
            id SERIAL PRIMARY KEY,
            login TEXT UNIQUE,
            email TEXT,
            password_hash TEXT,
            type TEXT,
            status TEXT DEFAULT 'active'
        )"#,
    )
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS sessions(
            user_id INT REFERENCES users(id),
            token TEXT PRIMARY KEY,
            expires_at TIMESTAMPTZ
        )"#,
    )
    .execute(pool)
    .await
    .unwrap();

    // Clean tables
    let _ = sqlx::query("DELETE FROM sessions").execute(pool).await;
    let _ = sqlx::query("DELETE FROM users").execute(pool).await;
    let _ = sqlx::query("DELETE FROM wp_test").execute(pool).await;
}

#[tokio::test]
#[cfg_attr(not(feature = "pg_tests"), ignore)]
async fn pg_middleware_select_update_insert() {
    let table = format!(
        "wp_test_{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")
    );
    let src = pg_test_program(&table);
    let (_rest, program) = parse_program(&src).unwrap();
    let (env, ctx) = build_env_with_ctx(&program).await;
    ensure_schema(&ctx, &table).await;

    // SELECT
    let p_select = program
        .pipelines
        .iter()
        .find(|p| p.name == "doSelect")
        .unwrap()
        .pipeline
        .clone();
    let (out, _ct, _st, _ctx) = execute_pipeline(&env, &p_select, serde_json::json!({"resultName":"s"}), webpipe::executor::RequestContext::new())
        .await
        .unwrap();
    assert_eq!(out["data"]["s"]["rowCount"], serde_json::json!(1));
    assert_eq!(out["data"]["s"]["rows"][0]["one"], serde_json::json!(1));

    // INSERT ... RETURNING
    let p_insert = program
        .pipelines
        .iter()
        .find(|p| p.name == "doInsertReturning")
        .unwrap()
        .pipeline
        .clone();
    let (out, _ct, _st, _ctx) = execute_pipeline(&env, &p_insert, serde_json::json!({"resultName":"ins"}), webpipe::executor::RequestContext::new())
        .await
        .unwrap();
    assert_eq!(out["data"]["ins"]["rowCount"], serde_json::json!(1));

    // UPDATE (no RETURNING) -> rowCount in `data`
    let p_update = program
        .pipelines
        .iter()
        .find(|p| p.name == "doUpdate")
        .unwrap()
        .pipeline
        .clone();
    let (out, _ct, _st, _ctx) = execute_pipeline(&env, &p_update, serde_json::json!({}), webpipe::executor::RequestContext::new())
        .await
        .unwrap();
    assert!(out["data"]["rowCount"].as_u64().unwrap() >= 1);
}

fn extract_token_from_set_cookie(header: &str) -> Option<String> {
    // Expect format like: wp_session=<token>; ...
    header
        .split(';')
        .next()
        .and_then(|kv| kv.split_once('='))
        .map(|(_, v)| v.to_string())
}

#[tokio::test]
#[cfg_attr(not(feature = "pg_tests"), ignore)]
async fn auth_register_login_required_logout_flow() {
    // table only used for wp_test in this program; users/sessions are global tables
    let table = format!(
        "wp_test_{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")
    );
    let src = pg_test_program(&table);
    let (_rest, program) = parse_program(&src).unwrap();
    let (env, ctx) = build_env_with_ctx(&program).await;
    ensure_schema(&ctx, &table).await;

    // Register
    let p_reg = program
        .pipelines
        .iter()
        .find(|p| p.name == "doAuthRegister")
        .unwrap()
        .pipeline
        .clone();
    let body = serde_json::json!({"body": {"login": "alice", "email": "alice@example.com", "password": "pw"}});
    let (reg_out, _ct, _st, _ctx) = execute_pipeline(&env, &p_reg, body, webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(reg_out["user"]["login"], serde_json::json!("alice"));

    // Login
    let p_login = program
        .pipelines
        .iter()
        .find(|p| p.name == "doAuthLogin")
        .unwrap()
        .pipeline
        .clone();
    let body = serde_json::json!({"body": {"login": "alice", "password": "pw"}});
    let (login_out, _ct, _st, _ctx) = execute_pipeline(&env, &p_login, body, webpipe::executor::RequestContext::new()).await.unwrap();
    let set_cookies = login_out["setCookies"].as_array().unwrap();
    let token = extract_token_from_set_cookie(set_cookies[0].as_str().unwrap()).expect("token");

    // Required
    let p_req = program
        .pipelines
        .iter()
        .find(|p| p.name == "doAuthRequired")
        .unwrap()
        .pipeline
        .clone();
    let req_input = serde_json::json!({"cookies": {"wp_session": token}});
    let (required_out, _ct, _st, _ctx) = execute_pipeline(&env, &p_req, req_input, webpipe::executor::RequestContext::new()).await.unwrap();
    assert_eq!(required_out["user"]["login"], serde_json::json!("alice"));

    // Logout
    let p_logout = program
        .pipelines
        .iter()
        .find(|p| p.name == "doAuthLogout")
        .unwrap()
        .pipeline
        .clone();
    let (logout_out, _ct, _st, _ctx) = execute_pipeline(&env, &p_logout, serde_json::json!({"cookies": {"wp_session": token}}), webpipe::executor::RequestContext::new()).await.unwrap();
    let clear = logout_out["setCookies"][0].as_str().unwrap();
    assert!(clear.contains("Max-Age=0"));
}


