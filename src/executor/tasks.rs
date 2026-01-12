use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::Mutex;
use tokio::task::JoinHandle;
use serde_json::Value;

use crate::error::WebPipeError;
use crate::executor::context::Profiler;

/// Registry for async tasks spawned with @async tag
#[derive(Clone)]
pub struct AsyncTaskRegistry {
    tasks: Arc<Mutex<HashMap<String, JoinHandle<Result<(Value, Profiler), WebPipeError>>>>>,
}

impl AsyncTaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register(&self, name: String, handle: JoinHandle<Result<(Value, Profiler), WebPipeError>>) {
        self.tasks.lock().insert(name, handle);
    }

    pub fn take(&self, name: &str) -> Option<JoinHandle<Result<(Value, Profiler), WebPipeError>>> {
        self.tasks.lock().remove(name)
    }
}

pub fn parse_join_task_names(config: &str) -> Result<Vec<String>, WebPipeError> {
    let trimmed = config.trim();

    // Try parsing as JSON array first
    if trimmed.starts_with('[') {
        match serde_json::from_str::<Vec<String>>(trimmed) {
            Ok(names) => return Ok(names),
            Err(_) => {
                return Err(WebPipeError::ConfigError(
                    "Invalid JSON array for join config".to_string()
                ));
            }
        }
    }

    // Otherwise parse as comma-separated list
    let names: Vec<String> = trimmed
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if names.is_empty() {
        return Err(WebPipeError::ConfigError(
            "join config must specify at least one task name".to_string()
        ));
    }

    Ok(names)
}
