use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use async_graphql::dataloader::Loader;
use crate::executor::ExecutionEnv;
use crate::error::WebPipeError;

/// Key for the DataLoader: (PipelineName, KeyValue)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LoaderKey(pub String, pub Value);

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
        let mut grouped: HashMap<String, Vec<(LoaderKey, Value)>> = HashMap::new();
        for key in keys {
            grouped.entry(key.0.clone())
                .or_insert_with(Vec::new)
                .push((key.clone(), key.1.clone()));
        }

        let mut results = HashMap::new();

        // Execute each pipeline with its batched keys
        for (pipeline_name, key_entries) in grouped {

            // Extract just the key values
            let key_values: Vec<Value> = key_entries.iter().map(|(_, v)| v.clone()).collect();

            // Construct input: { "keys": [val1, val2, ...] }
            let input = serde_json::json!({
                "keys": key_values
            });

            // Find the pipeline in the named pipelines registry
            let pipeline = self.env.named_pipelines.get(&pipeline_name)
                .ok_or_else(|| Arc::new(WebPipeError::PipelineNotFound(
                    format!("{}", pipeline_name)
                )))?;

            // Execute the pipeline using the shared request context for profiling
            let mut req_ctx = self.req_ctx.lock().await;

            match crate::executor::execute_pipeline_internal(
                &self.env,
                pipeline,
                input,
                &mut *req_ctx,
            ).await {
                Ok((result_value, _, _)) => {
                    // Expect the result to be a Map/Object: {"key1": result1, "key2": result2}
                    if let Value::Object(result_map) = result_value {
                        // Map results back to the original LoaderKeys
                        for (loader_key, key_value) in key_entries {
                            // Convert key_value to string for lookup
                            let key_str = match &key_value {
                                Value::String(s) => s.clone(),
                                Value::Number(n) => n.to_string(),
                                Value::Bool(b) => b.to_string(),
                                _ => serde_json::to_string(&key_value)
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
