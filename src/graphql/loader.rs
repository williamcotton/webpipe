use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use async_graphql::dataloader::Loader;
use crate::executor::ExecutionEnv;
use crate::error::WebPipeError;

/// Key for the DataLoader: (PipelineName, KeyValue, Context)
/// Hash/Eq only consider pipeline_name and key for batching,
/// but we carry the context through to pass to the pipeline
#[derive(Debug, Clone)]
pub struct LoaderKey {
    pub pipeline_name: String,
    pub key: Value,
    pub context: Value,  // Full {parent, args, context} from resolver
}

// Implement Hash/Eq to only consider pipeline_name and key for batching
impl PartialEq for LoaderKey {
    fn eq(&self, other: &Self) -> bool {
        self.pipeline_name == other.pipeline_name && self.key == other.key
    }
}

impl Eq for LoaderKey {}

impl std::hash::Hash for LoaderKey {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.pipeline_name.hash(state);
        serde_json::to_string(&self.key).unwrap_or_default().hash(state);
    }
}

/// PipelineLoader implements the async-graphql Loader trait
/// It batches requests by pipeline name and executes each pipeline once with all keys
pub struct PipelineLoader {
    pub env: ExecutionEnv,
    pub req_ctx: Arc<tokio::sync::Mutex<crate::executor::RequestContext>>,
}

impl Loader<LoaderKey> for PipelineLoader {
    type Value = Value;
    type Error = Arc<WebPipeError>;

    async fn load(&self, keys: &[LoaderKey]) -> Result<HashMap<LoaderKey, Self::Value>, Self::Error> {
        // Group keys by pipeline name
        let mut grouped: HashMap<String, Vec<LoaderKey>> = HashMap::new();
        for key in keys {
            grouped.entry(key.pipeline_name.clone())
                .or_insert_with(Vec::new)
                .push(key.clone());
        }

        let mut results = HashMap::new();

        // Execute each pipeline with its batched keys
        for (pipeline_name, key_entries) in grouped {

            // Extract just the key values
            let key_values: Vec<Value> = key_entries.iter().map(|k| k.key.clone()).collect();

            // Extract args and context from the first key
            // All keys in a batch share the same args/context (same GraphQL query)
            let first_context = &key_entries[0].context;
            let args = first_context.get("args").cloned().unwrap_or(Value::Null);
            let context = first_context.get("context").cloned().unwrap_or(Value::Null);

            // Construct input: { "keys": [...], "args": {...}, "context": {...} }
            let input = serde_json::json!({
                "keys": key_values,
                "args": args,
                "context": context
            });

            // Find the pipeline in the named pipelines registry
            // Look up local pipelines (namespace = None)
            let key = (None, pipeline_name.clone());
            let pipeline = self.env.named_pipelines.get(&key)
                .ok_or_else(|| Arc::new(WebPipeError::PipelineNotFound(
                    format!("{}", pipeline_name)
                )))?;

            // Execute the pipeline using the shared request context for profiling
            let mut req_ctx = self.req_ctx.lock().await;

            // Track loader pipeline execution in profiler
            req_ctx.profiler.push(&pipeline_name);
            let start = std::time::Instant::now();

            let result = crate::executor::execute_pipeline_internal(
                &self.env,
                pipeline,
                input,
                &mut *req_ctx,
            ).await;

            let elapsed = start.elapsed().as_micros();
            req_ctx.profiler.record_sample(elapsed);
            req_ctx.profiler.pop();

            match result {
                Ok((result_value, _, _)) => {
                    // Expect the result to be a Map/Object: {"key1": result1, "key2": result2}
                    if let Value::Object(result_map) = result_value {
                        // Map results back to the original LoaderKeys
                        for loader_key in key_entries {
                            // Convert key to string for lookup
                            let key_str = match &loader_key.key {
                                Value::String(s) => s.clone(),
                                Value::Number(n) => n.to_string(),
                                Value::Bool(b) => b.to_string(),
                                _ => serde_json::to_string(&loader_key.key)
                                    .unwrap_or_else(|_| "unknown".to_string()),
                            };

                            if let Some(result) = result_map.get(&key_str) {
                                results.insert(loader_key, result.clone());
                            } else {
                                // If the key is not found in the result map, insert null
                                results.insert(loader_key, Value::Null);
                            }
                        }
                    } else {
                        // If the result is not an object, return an error
                        return Err(Arc::new(WebPipeError::InternalError(
                            format!("Loader pipeline {} must return an object/map, got: {:?}", pipeline_name, result_value)
                        )));
                    }
                }
                Err(e) => return Err(Arc::new(e)),
            }
        }

        Ok(results)
    }
}
