use async_trait::async_trait;
use serde_json::Value;
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
    async fn execute(&self, config: &str, input: &Value) -> Result<Value, WebPipeError> {
        // Get the pre-compiled GraphQL runtime
        let runtime = self.ctx.graphql
            .as_ref()
            .ok_or_else(|| WebPipeError::ConfigError(
                "No GraphQL schema defined in program".into()
            ))?;

        // Get the execution environment from context
        let env = self.ctx.execution_env
            .read()
            .as_ref()
            .ok_or_else(|| WebPipeError::ConfigError(
                "No execution environment available".into()
            ))?
            .clone();

        // Extract GraphQL variables from input.graphqlParams
        let variables = input
            .get("graphqlParams")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        // Pass the pipeline state to GraphQL, but remove resultName
        // resultName is only for the GraphQL middleware, not for resolvers
        let mut pipeline_state = input.clone();
        if let Some(obj) = pipeline_state.as_object_mut() {
            obj.remove("resultName");
        }

        // Execute the GraphQL query
        let query = config;
        let response = runtime.execute(
            query,
            variables,
            pipeline_state,
            &env
        ).await?;

        // Check for resultName pattern (auto-naming)
        if let Some(result_name) = input.get("resultName").and_then(|v| v.as_str()) {
            // DEBUG: Log the GraphQL response
            eprintln!("GraphQL response for resultName '{}': {}", result_name, serde_json::to_string_pretty(&response).unwrap_or_else(|_| "invalid json".to_string()));

            // Extract the data from GraphQL response and store under .data.<resultName>
            let mut output = input.clone();
            if let Some(obj) = output.as_object_mut() {
                let data_obj = obj.entry("data")
                    .or_insert_with(|| Value::Object(serde_json::Map::new()))
                    .as_object_mut()
                    .unwrap();

                // Extract just the data portion from the GraphQL response
                // GraphQL response format is: { data: { fieldName: value }, errors?: [...] }
                // We want to store the entire data object under the result name
                if let Some(graphql_data) = response.get("data") {
                    eprintln!("Storing graphql_data under '{}': {}", result_name, serde_json::to_string_pretty(&graphql_data).unwrap_or_else(|_| "invalid".to_string()));
                    data_obj.insert(result_name.to_string(), graphql_data.clone());
                } else {
                    eprintln!("No 'data' field in GraphQL response for '{}'", result_name);
                    data_obj.insert(result_name.to_string(), Value::Null);
                }

                // If there were errors, add them to the output
                if let Some(errors) = response.get("errors") {
                    obj.insert("graphqlErrors".to_string(), errors.clone());
                }

                obj.remove("resultName");
            }
            Ok(output)
        } else {
            // Default: replace state with GraphQL response
            Ok(response)
        }
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
            hb: Arc::new(parking_lot::Mutex::new(handlebars::Handlebars::new())),
            cfg: crate::runtime::context::ConfigSnapshot(json!({})),
            lua_scripts: Arc::new(std::collections::HashMap::new()),
            graphql: None,
            execution_env: Arc::new(parking_lot::RwLock::new(None)),
        });

        let middleware = GraphQLMiddleware::new(ctx);
        let input = json!({});

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(middleware.execute("query { test }", &input));

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No GraphQL schema"));
    }
}
