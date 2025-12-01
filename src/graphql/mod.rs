use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ast::{Pipeline, Program};
use crate::executor::ExecutionEnv;
use crate::error::WebPipeError;
use async_graphql_parser::{parse_query, types::*};
use async_graphql_value::Value as GraphQLValue;

/// Compiled GraphQL runtime - created ONCE at server startup
#[derive(Clone)]
pub struct GraphQLRuntime {
    /// The SDL schema definition (stored for reference/validation)
    pub schema_sdl: String,
    /// Registry mapping query field names to their pipeline implementations
    pub query_resolvers: Arc<HashMap<String, Arc<Pipeline>>>,
    /// Registry mapping mutation field names to their pipeline implementations
    pub mutation_resolvers: Arc<HashMap<String, Arc<Pipeline>>>,
}

impl GraphQLRuntime {
    /// Create a new GraphQL runtime from a WebPipe program
    /// This is called ONCE at server startup to compile the schema
    pub fn from_program(program: &Program) -> Result<Self, anyhow::Error> {
        // Get SDL schema
        let schema_sdl = program
            .graphql_schema
            .as_ref()
            .map(|s| s.sdl.clone())
            .ok_or_else(|| anyhow::anyhow!("No GraphQL schema defined"))?;

        // Build resolver registry for queries
        let mut query_resolvers_map = HashMap::new();
        for query in &program.queries {
            query_resolvers_map.insert(
                query.name.clone(),
                Arc::new(query.pipeline.clone())
            );
        }

        // Build resolver registry for mutations
        let mut mutation_resolvers_map = HashMap::new();
        for mutation in &program.mutations {
            mutation_resolvers_map.insert(
                mutation.name.clone(),
                Arc::new(mutation.pipeline.clone())
            );
        }

        tracing::info!(
            "GraphQL runtime initialized with {} queries, {} mutations",
            query_resolvers_map.len(),
            mutation_resolvers_map.len()
        );

        Ok(Self {
            schema_sdl,
            query_resolvers: Arc::new(query_resolvers_map),
            mutation_resolvers: Arc::new(mutation_resolvers_map),
        })
    }

    /// Execute a GraphQL query or mutation
    ///
    /// For WebPipe 2.0, this is a simplified implementation that:
    /// - Parses the GraphQL query to extract the operation type and field name
    /// - Executes the corresponding resolver pipeline
    /// - Returns the result in GraphQL response format
    ///
    /// Full async-graphql integration with nested resolvers will be added in 2.1
    pub async fn execute(
        &self,
        query: &str,
        variables: Value,
        pipeline_state: Value,
        env: &ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
    ) -> Result<Value, WebPipeError> {
        // Parse the GraphQL query to extract operation type and all fields
        let (operation_type, field_args_list) = self.parse_query_fields_with_variables(query, &variables)?;

        // Execute each field resolver and collect results
        let mut data = serde_json::Map::new();
        let mut errors = Vec::new();

        for (field_name, field_args) in field_args_list {
            // 1. Construct target key for logging and mocking
            let target_key = format!("{}.{}", operation_type, field_name);

            // 2. Log the call (Spying) - record arguments passed to this resolver
            ctx.call_log.entry(target_key.clone())
               .or_default()
               .push(field_args.clone());

            // 3. Check for Mock - if mocked, skip execution and use mock value
            if let Some(mock_val) = env.invoker.get_mock(&target_key) {
                data.insert(field_name.clone(), mock_val);
                continue;
            }

            // 4. Standard Execution (existing logic)
            // Get the resolver pipeline
            let pipeline = match operation_type.as_str() {
                "query" => {
                    self.query_resolvers.get(&field_name)
                }
                "mutation" => {
                    self.mutation_resolvers.get(&field_name)
                }
                _ => None
            };

            let Some(pipeline) = pipeline else {
                errors.push(json!({
                    "message": format!("No {} resolver found for field '{}'", operation_type, field_name),
                    "path": [field_name.clone()]
                }));
                continue;
            };

            // Clone pipeline state for each field execution
            let mut field_state = pipeline_state.clone();

            // Merge GraphQL variables and field arguments into pipeline state
            if let Some(state_obj) = field_state.as_object_mut() {
                // Add variables at root level
                if let Some(vars_obj) = variables.as_object() {
                    for (k, v) in vars_obj {
                        state_obj.insert(k.clone(), v.clone());
                    }
                }
                // Add field arguments at root level
                if let Some(args_obj) = field_args.as_object() {
                    for (k, v) in args_obj {
                        state_obj.insert(k.clone(), v.clone());
                    }
                }
            }

            // Execute the resolver pipeline (reuse existing context for call logging)
            let result = crate::executor::execute_pipeline_internal(
                env,
                pipeline,
                field_state,
                ctx
            ).await;

            match result {
                Ok((field_data, _, _)) => {
                    data.insert(field_name.clone(), field_data);
                }
                Err(e) => {
                    errors.push(json!({
                        "message": e.to_string(),
                        "path": [field_name.clone()]
                    }));
                    data.insert(field_name.clone(), Value::Null);
                }
            }
        }

        // Return GraphQL response format
        let mut response = json!({
            "data": data
        });

        if !errors.is_empty() {
            response["errors"] = json!(errors);
        }

        Ok(response)
    }

    /// Parse a GraphQL query and extract all root fields with variable resolution
    ///
    /// Returns (operation_type, vec![(field_name, field_args)])
    fn parse_query_fields_with_variables(&self, query: &str, variables: &Value) -> Result<(String, Vec<(String, Value)>), WebPipeError> {
        let (op_type, fields_with_var_refs) = self.parse_query_fields(query)?;

        // Resolve variable references in field arguments
        let resolved_fields = fields_with_var_refs
            .into_iter()
            .map(|(field_name, args)| {
                let resolved_args = self.resolve_variables_in_value(&args, variables);
                (field_name, resolved_args)
            })
            .collect();

        Ok((op_type, resolved_fields))
    }

    /// Resolve variable references in a JSON value
    /// Replaces {"__variable__": "varName"} with the actual value from variables
    fn resolve_variables_in_value(&self, value: &Value, variables: &Value) -> Value {
        match value {
            Value::Object(obj) => {
                // Check if this is a variable marker
                if obj.len() == 1 {
                    if let Some(var_name) = obj.get("__variable__") {
                        if let Some(var_name_str) = var_name.as_str() {
                            // Look up the variable value
                            return variables.get(var_name_str).cloned().unwrap_or(Value::Null);
                        }
                    }
                }

                // Otherwise, recursively resolve nested objects
                Value::Object(
                    obj.iter()
                        .map(|(k, v)| (k.clone(), self.resolve_variables_in_value(v, variables)))
                        .collect()
                )
            }
            Value::Array(arr) => {
                Value::Array(
                    arr.iter()
                        .map(|v| self.resolve_variables_in_value(v, variables))
                        .collect()
                )
            }
            _ => value.clone()
        }
    }

    /// Parse a GraphQL query and extract all root fields using async-graphql-parser
    ///
    /// Returns (operation_type, vec![(field_name, field_args)])
    fn parse_query_fields(&self, query: &str) -> Result<(String, Vec<(String, Value)>), WebPipeError> {
        // Use the proper GraphQL parser from async-graphql-parser
        let doc = parse_query(query)
            .map_err(|e| WebPipeError::ConfigError(format!("Invalid GraphQL query: {}", e)))?;

        // Get the first operation definition
        let operation = doc
            .operations
            .iter()
            .next()
            .ok_or_else(|| WebPipeError::ConfigError("No operation found in GraphQL query".into()))?;

        let (operation_type, selection_set) = match &operation.1.node.ty {
            OperationType::Query => ("query", &operation.1.node.selection_set),
            OperationType::Mutation => ("mutation", &operation.1.node.selection_set),
            OperationType::Subscription => {
                return Err(WebPipeError::ConfigError(
                    "Subscriptions are not supported in WebPipe 2.0".into()
                ));
            }
        };

        // Extract all root-level fields
        let mut fields = Vec::new();
        for selection in &selection_set.node.items {
            match &selection.node {
                Selection::Field(field) => {
                    let field_name = field.node.name.node.to_string();

                    // Store arguments as-is (including Variable references)
                    // We'll resolve them at execution time against the variables parameter
                    let args = Value::Object(
                        field.node.arguments
                            .iter()
                            .map(|(name, value)| {
                                (name.node.to_string(), self.graphql_value_to_json(&value.node).unwrap_or(Value::Null))
                            })
                            .collect()
                    );

                    fields.push((field_name, args));
                }
                Selection::FragmentSpread(_) | Selection::InlineFragment(_) => {
                    return Err(WebPipeError::ConfigError(
                        "Fragments are not supported in WebPipe 2.0".into()
                    ));
                }
            }
        }

        if fields.is_empty() {
            return Err(WebPipeError::ConfigError(
                "No fields found in GraphQL query".into()
            ));
        }

        Ok((operation_type.to_string(), fields))
    }

    /// Convert async-graphql Value to serde_json Value
    fn graphql_value_to_json(&self, value: &GraphQLValue) -> Result<Value, WebPipeError> {
        match value {
            GraphQLValue::Null => Ok(Value::Null),
            GraphQLValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(json!(i))
                } else if let Some(f) = n.as_f64() {
                    Ok(json!(f))
                } else {
                    Ok(json!(n.to_string()))
                }
            }
            GraphQLValue::String(s) => Ok(json!(s)),
            GraphQLValue::Boolean(b) => Ok(json!(b)),
            GraphQLValue::Enum(e) => Ok(json!(e.to_string())),
            GraphQLValue::List(list) => {
                let mut arr = Vec::new();
                for item in list {
                    arr.push(self.graphql_value_to_json(item)?);
                }
                Ok(Value::Array(arr))
            }
            GraphQLValue::Object(obj) => {
                let mut map = serde_json::Map::new();
                for (key, val) in obj {
                    map.insert(key.to_string(), self.graphql_value_to_json(val)?);
                }
                Ok(Value::Object(map))
            }
            GraphQLValue::Variable(var_name) => {
                // Return a special marker that we'll resolve later
                // Format: {"__variable__": "varName"}
                Ok(json!({"__variable__": var_name.to_string()}))
            }
            GraphQLValue::Binary(_) => {
                Err(WebPipeError::ConfigError("Binary values not supported".into()))
            }
        }
    }

}

impl std::fmt::Debug for GraphQLRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphQLRuntime")
            .field("queries", &self.query_resolvers.len())
            .field("mutations", &self.mutation_resolvers.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_query_without_args() {
        let runtime = GraphQLRuntime {
            schema_sdl: String::new(),
            query_resolvers: Arc::new(HashMap::new()),
            mutation_resolvers: Arc::new(HashMap::new()),
        };

        let (op, fields) = runtime.parse_query_fields("query { todos { id title } }").unwrap();
        assert_eq!(op, "query");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "todos");
        assert_eq!(fields[0].1, json!({}));
    }

    #[test]
    fn parse_simple_query_with_args() {
        let runtime = GraphQLRuntime {
            schema_sdl: String::new(),
            query_resolvers: Arc::new(HashMap::new()),
            mutation_resolvers: Arc::new(HashMap::new()),
        };

        let (op, fields) = runtime.parse_query_fields(
            "query { todos(limit: 10, completed: true) { id } }"
        ).unwrap();
        assert_eq!(op, "query");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "todos");
        assert_eq!(fields[0].1["limit"], 10);
        assert_eq!(fields[0].1["completed"], true);
    }

    #[test]
    fn parse_mutation() {
        let runtime = GraphQLRuntime {
            schema_sdl: String::new(),
            query_resolvers: Arc::new(HashMap::new()),
            mutation_resolvers: Arc::new(HashMap::new()),
        };

        let (op, fields) = runtime.parse_query_fields(
            r#"mutation { createTodo(title: "test") { id } }"#
        ).unwrap();
        assert_eq!(op, "mutation");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "createTodo");
        assert_eq!(fields[0].1["title"], "test");
    }

    #[test]
    fn parse_query_with_variables() {
        let runtime = GraphQLRuntime {
            schema_sdl: String::new(),
            query_resolvers: Arc::new(HashMap::new()),
            mutation_resolvers: Arc::new(HashMap::new()),
        };

        let (op, fields) = runtime.parse_query_fields(
            "query($limit: Int) { todos(limit: $limit) { id } }"
        ).unwrap();
        assert_eq!(op, "query");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "todos");
        // Variable references are marked for resolution
        assert_eq!(fields[0].1["limit"], json!({"__variable__": "limit"}));

        // Test variable resolution
        let variables = json!({"limit": 5});
        let (_, resolved_fields) = runtime.parse_query_fields_with_variables(
            "query($limit: Int) { todos(limit: $limit) { id } }",
            &variables
        ).unwrap();
        assert_eq!(resolved_fields[0].1["limit"], 5);
    }
}
