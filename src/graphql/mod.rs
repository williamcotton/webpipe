use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;

use async_graphql::{Request, Variables};
use async_graphql::dynamic::{
    Field, FieldFuture, FieldValue, InputValue, Object, Scalar, Schema, TypeRef
};
use async_graphql_parser::{parse_schema, types as ast};
use tokio::sync::Mutex;

use crate::ast::Program;
use crate::executor::ExecutionEnv;
use crate::error::WebPipeError;

pub mod loader;
pub use loader::{LoaderKey, PipelineLoader};

pub struct GraphQLRuntime {
    pub schema: Schema,
}

impl GraphQLRuntime {
    pub fn from_program(program: &Program) -> Result<Self, anyhow::Error> {
        let sdl = program.graphql_schema.as_ref()
            .map(|s| s.sdl.as_str())
            .unwrap_or("");

        let doc = parse_schema(sdl)?;

        let mut pipelines = HashMap::new();
        for q in &program.queries { pipelines.insert(format!("Query.{}", q.name), q.pipeline.clone()); }
        for m in &program.mutations { pipelines.insert(format!("Mutation.{}", m.name), m.pipeline.clone()); }
        for r in &program.resolvers { pipelines.insert(format!("{}.{}", r.type_name, r.field_name), r.pipeline.clone()); }
        let pipeline_registry = Arc::new(pipelines);

        let mut schema_builder = Schema::build("Query", Some("Mutation"), None)
            .register(Scalar::new("JSON"));

        fn convert_type(ty: &ast::Type) -> TypeRef {
            match &ty.base {
                ast::BaseType::Named(name) => {
                    let type_name = name.as_str();
                    let t = match type_name {
                        "ID" => TypeRef::named(TypeRef::ID),
                        "String" => TypeRef::named(TypeRef::STRING),
                        "Int" => TypeRef::named(TypeRef::INT),
                        "Float" => TypeRef::named(TypeRef::FLOAT),
                        "Boolean" => TypeRef::named(TypeRef::BOOLEAN),
                        other => TypeRef::named(other),
                    };
                    if !ty.nullable { TypeRef::NonNull(Box::new(t)) } else { t }
                },
                ast::BaseType::List(inner) => {
                    let inner_ref = convert_type(inner);
                    let t = TypeRef::List(Box::new(inner_ref));
                    if !ty.nullable { TypeRef::NonNull(Box::new(t)) } else { t }
                }
            }
        }

        for def in &doc.definitions {
            if let ast::TypeSystemDefinition::Type(type_def) = def {
                if let ast::TypeKind::Object(obj_def) = &type_def.node.kind {
                    let type_name = type_def.node.name.node.as_str();
                    let mut obj = Object::new(type_name);

                    for field_def in &obj_def.fields {
                        let field_name = field_def.node.name.node.as_str().to_string();
                        let field_name_clone = field_name.clone(); 
                        let type_name_clone = type_name.to_string();
                        let p_reg = pipeline_registry.clone();
                        let field_type = convert_type(&field_def.node.ty.node);

                        let mut field = Field::new(field_name.clone(), field_type, move |ctx| {
                            let p_reg = p_reg.clone();
                            let field_name = field_name_clone.clone();
                            let type_name = type_name_clone.clone();

                            FieldFuture::new(async move {
                                let key = format!("{}.{}", type_name, field_name);

                                if let Some(pipeline) = p_reg.get(&key) {
                                    // Check if this is a TypeResolver using DataLoader
                                    // Pattern: resolver Type.field = |> debug |> loader(keyExpr): PipelineName
                                    // Find the loader step anywhere in the pipeline
                                    let loader_step = pipeline.steps.iter().find(|step| {
                                        if let crate::ast::PipelineStep::Regular { name, .. } = step {
                                            name == "loader"
                                        } else {
                                            false
                                        }
                                    });

                                    if let Some(loader_step) = loader_step {
                                        if type_name != "Query" && type_name != "Mutation" {
                                            // === DATALOADER RESOLVER (Batched with Pipeline Composition) ===
                                            use async_graphql::dataloader::DataLoader;

                                            let env = ctx.data::<ExecutionEnv>().unwrap();
                                            let req_ctx_mutex = ctx.data::<Arc<Mutex<crate::executor::RequestContext>>>().unwrap();

                                            let loader = ctx.data::<DataLoader<PipelineLoader>>()
                                                .map_err(|_| async_graphql::Error::new("DataLoader not registered"))?;

                                            // Find the loader step index
                                            let loader_index = pipeline.steps.iter().position(|step| {
                                                if let crate::ast::PipelineStep::Regular { name, .. } = step {
                                                    name == "loader"
                                                } else {
                                                    false
                                                }
                                            }).unwrap();

                                            // Extract loader config
                                            let (loader_pipeline_name, key_expr) = if let crate::ast::PipelineStep::Regular { config, args, .. } = loader_step {
                                                let key_expr = args.first()
                                                    .ok_or_else(|| async_graphql::Error::new("loader requires a key expression"))?
                                                    .clone();
                                                (config.clone(), key_expr)
                                            } else {
                                                return Err(async_graphql::Error::new("Invalid loader configuration"));
                                            };

                                            // Get parent context
                                            let parent_json = ctx.parent_value.as_value()
                                                .map(async_graphql_value_to_json)
                                                .ok_or_else(|| async_graphql::Error::new("loader requires parent context"))?;

                                            // Build initial input with parent context
                                            let mut current_value = serde_json::json!({ "parent": parent_json });

                                            // Execute steps BEFORE the loader
                                            if loader_index > 0 {
                                                let before_pipeline = crate::ast::Pipeline {
                                                    steps: pipeline.steps[..loader_index].to_vec(),
                                                };

                                                let mut req_ctx = req_ctx_mutex.lock().await;
                                                let (result, _, _) = crate::executor::execute_pipeline_internal(
                                                    env,
                                                    &before_pipeline,
                                                    current_value,
                                                    &mut req_ctx
                                                ).await.map_err(|e| async_graphql::Error::new(e.to_string()))?;
                                                current_value = result;
                                            }

                                            // Evaluate key expression using jq from current pipeline state
                                            let key_value = crate::runtime::jq::evaluate(&key_expr, &current_value)
                                                .map_err(|e| async_graphql::Error::new(format!("Failed to evaluate key expression: {}", e)))?;

                                            // Load using DataLoader (this batches automatically!)
                                            let loader_key = LoaderKey(loader_pipeline_name, key_value);
                                            let mut result_value = loader.load_one(loader_key).await
                                                .map_err(|e| async_graphql::Error::new(format!("DataLoader error: {}", e)))?
                                                .unwrap_or(Value::Null);

                                            // Execute steps AFTER the loader
                                            if loader_index < pipeline.steps.len() - 1 {
                                                let after_pipeline = crate::ast::Pipeline {
                                                    steps: pipeline.steps[loader_index + 1..].to_vec(),
                                                };

                                                let mut req_ctx = req_ctx_mutex.lock().await;
                                                let (result, _, _) = crate::executor::execute_pipeline_internal(
                                                    env,
                                                    &after_pipeline,
                                                    result_value,
                                                    &mut req_ctx
                                                ).await.map_err(|e| async_graphql::Error::new(e.to_string()))?;
                                                result_value = result;
                                            }

                                            // Convert to async_graphql::Value
                                            let result_val = json_to_async_graphql_value(result_value);

                                            // Wrap correctly based on type
                                            match result_val {
                                                async_graphql::Value::List(items) => {
                                                    let iter = items.into_iter().map(|item| {
                                                        FieldValue::value(item)
                                                    });
                                                    Ok(Some(FieldValue::list(iter)))
                                                }
                                                async_graphql::Value::Null => Ok(None),
                                                other => Ok(Some(FieldValue::value(other)))
                                            }
                                        } else {
                                            Err(async_graphql::Error::new("loader can only be used in type resolvers, not Query/Mutation"))
                                        }
                                    } else {
                                        // === PIPELINE RESOLVER (Root Queries/Mutations and Non-Loader Resolvers) ===
                                        let env = ctx.data::<ExecutionEnv>().unwrap();
                                        let req_ctx_mutex = ctx.data::<Arc<Mutex<crate::executor::RequestContext>>>().unwrap();

                                        let mut input = serde_json::Map::new();

                                        // Add parent value for nested resolvers
                                        if let Some(parent) = ctx.parent_value.as_value() {
                                            input.insert("parent".to_string(), async_graphql_value_to_json(parent));
                                        }

                                        // Add GraphQL arguments
                                        for (name, accessor) in ctx.args.iter() {
                                            // Directly extract Value ref
                                            let val_ref = accessor.as_value();
                                            input.insert(name.to_string(), async_graphql_value_to_json(val_ref));
                                        }

                                        let result_json = {
                                            let mut req_ctx = req_ctx_mutex.lock().await;

                                            req_ctx.call_log.entry(key.clone())
                                                .or_default()
                                                .push(Value::Object(input.clone()));

                                            // --- START PROFILING ---
                                            req_ctx.profiler.push(&key);
                                            let start = std::time::Instant::now();

                                            // Execute Pipeline (Mock or Real)
                                            let execution_result = if let Some(mock_val) = env.invoker.get_mock(&key) {
                                                mock_val
                                            } else {
                                                let (result, _, _) = crate::executor::execute_pipeline_internal(
                                                    env,
                                                    pipeline,
                                                    Value::Object(input),
                                                    &mut *req_ctx
                                                ).await.map_err(|e| e.to_string())?;
                                                result
                                            };

                                            let elapsed = start.elapsed().as_micros();
                                            req_ctx.profiler.record_sample(elapsed);
                                            req_ctx.profiler.pop();
                                            // --- END PROFILING ---

                                            execution_result
                                        };

                                        // Propagate errors from pipeline
                                        if let Some(obj) = result_json.as_object() {
                                            if let Some(errors) = obj.get("errors") {
                                                if let Some(err_array) = errors.as_array() {
                                                    if let Some(first_err) = err_array.first() {
                                                        let msg = first_err.get("message")
                                                            .and_then(|v| v.as_str())
                                                            .unwrap_or("Unknown pipeline error");
                                                        return Err(async_graphql::Error::new(msg));
                                                    }
                                                }
                                            }
                                        }

                                        // Convert to async_graphql::Value
                                        let result_val = json_to_async_graphql_value(result_json);

                                        // Wrap correctly based on type
                                        match result_val {
                                            async_graphql::Value::List(items) => {
                                                let iter = items.into_iter().map(|item| {
                                                    FieldValue::value(item)
                                                });
                                                Ok(Some(FieldValue::list(iter)))
                                            }
                                            async_graphql::Value::Null => Ok(None),
                                            other => Ok(Some(FieldValue::value(other)))
                                        }
                                    }

                                } else {
                                    // === DEFAULT PROPERTY RESOLVER (Nested Fields) ===
                                    // Use as_value() to read the parent without needing downcast
                                    let parent = ctx.parent_value.as_value()
                                        .ok_or_else(|| async_graphql::Error::new("Parent is not a Value"))?;
                                    
                                    match parent {
                                        async_graphql::Value::Object(map) => {
                                            let name = async_graphql::Name::new(&field_name);
                                            if let Some(val) = map.get(&name) {
                                                match val {
                                                    async_graphql::Value::List(items) => {
                                                        let iter = items.iter().map(|item| {
                                                            FieldValue::value(item.clone())
                                                        });
                                                        Ok(Some(FieldValue::list(iter)))
                                                    },
                                                    _ => Ok(Some(FieldValue::value(val.clone())))
                                                }
                                            } else {
                                                Ok(None)
                                            }
                                        }
                                        _ => Ok(None)
                                    }
                                }
                            })
                        });

                        for arg_def in &field_def.node.arguments {
                            let arg_name = arg_def.node.name.node.as_str();
                            let arg_type = convert_type(&arg_def.node.ty.node);
                            field = field.argument(InputValue::new(arg_name, arg_type));
                        }

                        obj = obj.field(field);
                    }
                    
                    schema_builder = schema_builder.register(obj);
                }
            }
        }

        let schema = schema_builder.finish().map_err(|e| anyhow::anyhow!("Schema build error: {}", e))?;
        Ok(Self { schema })
    }

    pub async fn execute(
        &self,
        query: &str,
        variables: Value,
        _pipeline_state: Value,
        env: &ExecutionEnv,
        ctx: &mut crate::executor::RequestContext,
    ) -> Result<Value, WebPipeError> {
        use async_graphql::dataloader::DataLoader;

        let ctx_arc = Arc::new(Mutex::new(std::mem::take(ctx)));

        // Create the PipelineLoader with batching support and shared request context for profiling
        let loader = DataLoader::new(
            PipelineLoader {
                env: env.clone(),
                req_ctx: ctx_arc.clone(),
            },
            tokio::spawn
        );

        let request = Request::new(query)
            .variables(Variables::from_json(variables))
            .data(env.clone())
            .data(ctx_arc.clone())
            .data(loader);

        let response = self.schema.execute(request).await;

        let mut inner = ctx_arc.lock().await;
        *ctx = std::mem::take(&mut *inner);

        Ok(serde_json::to_value(response).unwrap())
    }
}

fn async_graphql_value_to_json(v: &async_graphql::Value) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

fn json_to_async_graphql_value(v: Value) -> async_graphql::Value {
    async_graphql::Value::from_json(v).unwrap()
}

impl std::fmt::Debug for GraphQLRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphQLRuntime")
            .field("schema", &"DynamicSchema")
            .finish()
    }
}