use async_trait::async_trait;
use std::sync::Arc;

use crate::error::WebPipeError;
use crate::middleware::Middleware;
use crate::runtime::Context;

/// GraphQL middleware for executing GraphQL queries and mutations
///
/// This middleware:
/// 1. Reads the GraphQL query from the config (the query string)
/// 2. Extracts variables from input.graphqlParams
/// 3. Passes the entire pipeline state to the GraphQL runtime
/// 4. Returns the result in GraphQL response format: { data, errors }
/// 5. Supports resultName pattern for multiple queries
///
/// Example usage:
/// ```webpipe
/// GET /todos
///   |> auth: "required"
///   |> jq: `.graphqlParams = { limit: 10 }`
///   |> graphql: `query($limit: Int) { todos(limit: $limit) { id title } }`
///   |> jq: `{ todos: .data.todos }`
/// ```
///
/// With resultName for multiple queries:
/// ```webpipe
/// GET /dashboard
///   |> auth: "required"
///   |> jq: `.resultName = "todos"`
///   |> graphql: `query { todos { id title } }`
///   |> jq: `.resultName = "user"`
///   |> graphql: `query { currentUser { name email } }`
///   |> jq: `{ todos: .data.todos.data.todos, user: .data.user.data.currentUser }`
/// ```
#[derive(Debug)]
pub struct GraphQLMiddleware {
    ctx: Arc<Context>,
}

impl GraphQLMiddleware {
    pub fn new(ctx: Arc<Context>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl Middleware for GraphQLMiddleware {
    async fn execute(
        &self,
        args: &[String],
        config: &str,
        pipeline_ctx: &mut crate::runtime::PipelineContext,
        _env: &crate::executor::ExecutionEnv,
        _ctx: &mut crate::executor::RequestContext,
        target_name: Option<&str>,
    ) -> Result<(), WebPipeError> {
        // Get the pre-compiled GraphQL runtime
        let runtime = self.ctx.graphql
            .as_ref()
            .ok_or_else(|| WebPipeError::ConfigError(
                "No GraphQL schema defined in program".into()
            ))?;

        // Determine variables based on inline args vs fallback state
        let variables = if !args.is_empty() {
            // New syntax: graphql(variables_expr)
            // Evaluate arg[0] for variables object
            let variables_value = crate::runtime::jq::evaluate(&args[0], &pipeline_ctx.state)?;
            if !variables_value.is_object() {
                return Err(WebPipeError::MiddlewareExecutionError(
                    format!("graphql argument 0 must evaluate to an object, got: {:?}", variables_value)
                ));
            }
            variables_value
        } else {
            // Old syntax: fallback to graphqlParams from state
            pipeline_ctx.state
                .get("graphqlParams")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}))
        };

        // Execute the GraphQL query
        let query = config;
        let response = runtime.execute(
            query,
            variables,
            pipeline_ctx.state.clone(),
            _env,
            _ctx
        ).await?;

        // Smart unwrapping logic (from spec)
        // If target_name is present and matches a single field in the response,
        // unwrap it to avoid double nesting like .data.users.users
        if let Some(name) = target_name {
            if let Some(data) = response.get("data").and_then(|d| d.as_object()) {
                // If response is {"data": {"users": [...]}} and target is "users"
                // Unwrap to just [...] so executor wrapping creates {"data": {"users": [...]}}
                if data.len() == 1 && data.contains_key(name) {
                    pipeline_ctx.state = data.get(name).unwrap().clone();
                    return Ok(());
                }
            }
        }

        // Default: return full GraphQL response
        pipeline_ctx.state = response;
        Ok(())
    }

    fn behavior(&self) -> super::StateBehavior {
        // GraphQL acts as a Transform middleware - it replaces the entire state
        // with the query result, but should preserve system context when not terminal
        super::StateBehavior::Transform
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn middleware_requires_graphql_runtime() {
        // Create a context without GraphQL runtime
        let ctx = Arc::new(Context {
            pg: None,
            http: reqwest::Client::new(),
            cache: crate::runtime::context::CacheStore::new(8, 60),
            rate_limit: crate::runtime::context::RateLimitStore::new(1000),
            hb: Arc::new(parking_lot::Mutex::new(handlebars::Handlebars::new())),
            cfg: crate::runtime::context::ConfigSnapshot(json!({})),
            lua_scripts: Arc::new(std::collections::HashMap::new()),
            graphql: None,
            execution_env: Arc::new(parking_lot::RwLock::new(None)),
        });

        let middleware = GraphQLMiddleware::new(ctx);
        let input = json!({});

        let rt = tokio::runtime::Runtime::new().unwrap();
        let registry = Arc::new(crate::middleware::MiddlewareRegistry::default());
        let env = crate::executor::ExecutionEnv {
            variables: Arc::new(vec![]),
            named_pipelines: Arc::new(std::collections::HashMap::new()),
            invoker: Arc::new(crate::executor::RealInvoker::new(registry.clone())),
            registry: registry.clone(),
            environment: None,
            cache: crate::runtime::context::CacheStore::new(8, 60),
            rate_limit: crate::runtime::context::RateLimitStore::new(1000),
        };
        let mut req_ctx = crate::executor::RequestContext::new();
        let mut pipeline_ctx = crate::runtime::PipelineContext::new(input.clone());
        let result = rt.block_on(middleware.execute(&[], "query { test }", &mut pipeline_ctx, &env, &mut req_ctx, None));

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No GraphQL schema"));
    }
}
