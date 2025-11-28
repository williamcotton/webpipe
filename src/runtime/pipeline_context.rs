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

    /// Merge incoming state into current state (The "Backpack" semantics).
    /// 
    /// - If both current and incoming are JSON objects, merge them (incoming takes precedence)
    /// - Otherwise, replace current state entirely with incoming
    /// 
    /// This centralizes the state accumulation logic that was previously
    /// scattered throughout the executor.
    pub fn merge_state(&mut self, incoming: Value) {
        if let (Some(current_obj), Some(incoming_obj)) = (
            self.state.as_object_mut(),
            incoming.as_object()
        ) {
            // Merge: incoming keys overwrite current keys
            for (k, v) in incoming_obj {
                current_obj.insert(k.clone(), v.clone());
            }
        } else {
            // Non-object: replace entirely
            self.state = incoming;
        }
    }

    /// Replace state entirely (used by terminal transformers like jq, handlebars)
    #[inline]
    pub fn replace_state(&mut self, new_state: Value) {
        self.state = new_state;
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
