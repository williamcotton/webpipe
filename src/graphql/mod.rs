use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ast::{Pipeline, Program};
use crate::executor::ExecutionEnv;
use crate::error::WebPipeError;

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
        mut pipeline_state: Value,
        env: &ExecutionEnv,
    ) -> Result<Value, WebPipeError> {
        // Parse the GraphQL query to extract operation type and field name
        let (operation_type, field_name, field_args) = self.parse_simple_query(query)?;

        // Get the resolver pipeline
        let pipeline = match operation_type.as_str() {
            "query" => {
                self.query_resolvers
                    .get(&field_name)
                    .ok_or_else(|| WebPipeError::ConfigError(
                        format!("No query resolver found for field '{}'", field_name)
                    ))?
            }
            "mutation" => {
                self.mutation_resolvers
                    .get(&field_name)
                    .ok_or_else(|| WebPipeError::ConfigError(
                        format!("No mutation resolver found for field '{}'", field_name)
                    ))?
            }
            _ => {
                return Err(WebPipeError::ConfigError(
                    format!("Unsupported operation type: {}", operation_type)
                ));
            }
        };

        // Merge GraphQL variables and field arguments into pipeline state
        if let Some(state_obj) = pipeline_state.as_object_mut() {
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

        // Execute the resolver pipeline
        let result = crate::executor::execute_pipeline(
            env,
            pipeline,
            pipeline_state
        ).await;

        match result {
            Ok((data, _, _)) => {
                // Return GraphQL response format
                Ok(json!({
                    "data": { field_name: data }
                }))
            }
            Err(err) => {
                // Return GraphQL error format
                Ok(json!({
                    "data": null,
                    "errors": [{
                        "message": err.to_string(),
                        "path": [field_name]
                    }]
                }))
            }
        }
    }

    /// Simplified GraphQL query parser for WebPipe 2.0
    ///
    /// Extracts:
    /// - operation type ("query" or "mutation")
    /// - field name (e.g., "todos")
    /// - field arguments (e.g., { "limit": 10 })
    ///
    /// This handles simple single-field queries like:
    /// - query { todos { id title } }
    /// - mutation { createTodo(title: "test") { id } }
    /// - query($limit: Int) { todos(limit: $limit) { id } }
    ///
    /// Full GraphQL parsing with nested resolvers will be added in 2.1
    fn parse_simple_query(&self, query: &str) -> Result<(String, String, Value), WebPipeError> {
        let query = query.trim();

        // Determine operation type
        let operation_type = if query.starts_with("mutation") {
            "mutation"
        } else {
            "query" // default to query
        };

        // Extract field name and arguments
        // Look for pattern: fieldName(args) { or fieldName {
        let field_start = query.find('{')
            .ok_or_else(|| WebPipeError::ConfigError("Invalid GraphQL query: no opening brace".into()))?;

        let field_section = &query[..field_start];

        // Remove operation keyword and variable declarations
        let field_section = if let Some(vars_end) = field_section.rfind(')') {
            // Has variable declaration like query($limit: Int)
            let after_vars = &field_section[vars_end + 1..];
            after_vars.trim()
        } else {
            // Remove 'query' or 'mutation' keyword
            field_section
                .trim_start_matches("query")
                .trim_start_matches("mutation")
                .trim()
        };

        // Parse field name and arguments
        if let Some(args_start) = field_section.find('(') {
            // Has arguments: todos(limit: 10)
            let field_name = field_section[..args_start].trim().to_string();

            let args_end = field_section.find(')')
                .ok_or_else(|| WebPipeError::ConfigError("Invalid GraphQL query: unclosed argument list".into()))?;

            let args_str = &field_section[args_start + 1..args_end];
            let field_args = self.parse_arguments(args_str)?;

            Ok((operation_type.to_string(), field_name, field_args))
        } else {
            // No arguments: todos { id }
            let field_name = field_section.trim().to_string();
            Ok((operation_type.to_string(), field_name, json!({})))
        }
    }

    /// Parse GraphQL arguments into JSON
    /// Examples:
    /// - "limit: 10" -> { "limit": 10 }
    /// - "title: \"test\"" -> { "title": "test" }
    /// - "$limit" -> {} (variables are passed separately)
    fn parse_arguments(&self, args_str: &str) -> Result<Value, WebPipeError> {
        let mut args = serde_json::Map::new();

        for arg in args_str.split(',') {
            let arg = arg.trim();
            if arg.is_empty() || arg.starts_with('$') {
                // Skip variables (they're passed in variables object)
                continue;
            }

            if let Some(colon_pos) = arg.find(':') {
                let key = arg[..colon_pos].trim().to_string();
                let value_str = arg[colon_pos + 1..].trim();

                let value = if value_str.starts_with('"') && value_str.ends_with('"') {
                    // String value
                    Value::String(value_str[1..value_str.len() - 1].to_string())
                } else if value_str == "true" {
                    Value::Bool(true)
                } else if value_str == "false" {
                    Value::Bool(false)
                } else if value_str == "null" {
                    Value::Null
                } else if let Ok(num) = value_str.parse::<i64>() {
                    Value::Number(num.into())
                } else if let Ok(num) = value_str.parse::<f64>() {
                    Value::Number(serde_json::Number::from_f64(num).unwrap_or(0.into()))
                } else {
                    // Fallback to string
                    Value::String(value_str.to_string())
                };

                args.insert(key, value);
            }
        }

        Ok(Value::Object(args))
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

        let (op, field, args) = runtime.parse_simple_query("query { todos { id title } }").unwrap();
        assert_eq!(op, "query");
        assert_eq!(field, "todos");
        assert_eq!(args, json!({}));
    }

    #[test]
    fn parse_simple_query_with_args() {
        let runtime = GraphQLRuntime {
            schema_sdl: String::new(),
            query_resolvers: Arc::new(HashMap::new()),
            mutation_resolvers: Arc::new(HashMap::new()),
        };

        let (op, field, args) = runtime.parse_simple_query(
            "query { todos(limit: 10, completed: true) { id } }"
        ).unwrap();
        assert_eq!(op, "query");
        assert_eq!(field, "todos");
        assert_eq!(args["limit"], 10);
        assert_eq!(args["completed"], true);
    }

    #[test]
    fn parse_mutation() {
        let runtime = GraphQLRuntime {
            schema_sdl: String::new(),
            query_resolvers: Arc::new(HashMap::new()),
            mutation_resolvers: Arc::new(HashMap::new()),
        };

        let (op, field, args) = runtime.parse_simple_query(
            r#"mutation { createTodo(title: "test") { id } }"#
        ).unwrap();
        assert_eq!(op, "mutation");
        assert_eq!(field, "createTodo");
        assert_eq!(args["title"], "test");
    }

    #[test]
    fn parse_query_with_variables() {
        let runtime = GraphQLRuntime {
            schema_sdl: String::new(),
            query_resolvers: Arc::new(HashMap::new()),
            mutation_resolvers: Arc::new(HashMap::new()),
        };

        let (op, field, args) = runtime.parse_simple_query(
            "query($limit: Int) { todos(limit: $limit) { id } }"
        ).unwrap();
        assert_eq!(op, "query");
        assert_eq!(field, "todos");
        // Variable args are skipped in field args (they come from variables param)
        assert_eq!(args, json!({}));
    }
}
