use serde_json::Value;

/// Runtime keys that should be preserved during Transform operations
/// These are needed for async/join, accumulated results, and request context
const RUNTIME_KEYS: &[&str] = &[
    "async", "data", "originalRequest",
    "query", "params", "body", "headers", "cookies",
    "method", "path", "ip", "content_type"
];

/// PipelineContext holds the mutable state for a pipeline execution.
/// This struct is passed mutably through middleware to eliminate cloning.
#[derive(Debug)]
pub struct PipelineContext {
    /// The JSON state being transformed through the pipeline
    pub state: Value,
}

impl PipelineContext {
    /// Create a new PipelineContext with the given initial state
    pub fn new(state: Value) -> Self {
        Self { state }
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

    /// Backup system context keys before a Transform operation.
    /// Returns a vector of (key, value) pairs that can be restored later.
    pub fn backup_system_keys(&self) -> Vec<(String, Value)> {
        RUNTIME_KEYS.iter()
            .filter_map(|&key| {
                self.state.get(key).map(|v| (key.to_string(), v.clone()))
            })
            .collect()
    }

    /// Restore system context keys after a Transform operation.
    /// Only restores keys that don't already exist in the new state.
    pub fn restore_system_keys(&mut self, backups: Vec<(String, Value)>) {
        if let Some(obj) = self.state.as_object_mut() {
            for (key, val) in backups {
                if !obj.contains_key(&key) {
                    obj.insert(key, val);
                }
            }
        }
    }
}
