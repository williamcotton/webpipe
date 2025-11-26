use serde_json::Value;
use std::collections::HashMap;

/// PipelineContext holds the mutable state for a pipeline execution.
/// This struct is passed mutably through middleware to eliminate cloning.
#[derive(Debug)]
pub struct PipelineContext {
    /// The JSON state being transformed through the pipeline
    pub state: Value,

    /// Side-channel metadata for passing headers, status codes, etc.
    pub metadata: HashMap<String, String>,
}

impl PipelineContext {
    /// Create a new PipelineContext with the given initial state
    pub fn new(state: Value) -> Self {
        Self {
            state,
            metadata: HashMap::new(),
        }
    }

    /// Set a metadata value
    pub fn set_metadata(&mut self, key: String, value: String) {
        self.metadata.insert(key, value);
    }

    /// Get a metadata value
    pub fn get_metadata(&self, key: &str) -> Option<&String> {
        self.metadata.get(key)
    }
}
