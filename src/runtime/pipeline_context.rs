use serde_json::Value;

use crate::runtime::context::{CacheStore, CachedResponse};

/// A cache write registered by the cache middleware on a miss. It is executed
/// with the final state of the pipeline run in which it was registered, so a
/// `cache` step caches the output of its own pipeline rather than the
/// response of the enclosing request.
#[derive(Debug)]
pub struct PendingCacheSave {
    pub key: String,
    pub ttl: u64,
    pub store: CacheStore,
    /// True when this save refreshes a stale entry (the caller was elected
    /// refresher by stale-while-revalidate); if the save is skipped the
    /// refresh slot must be released via `CacheStore::clear_refresh`.
    pub refreshing: bool,
}

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
    /// Cache writes to perform with this pipeline run's final state
    pub pending_cache_saves: Vec<PendingCacheSave>,
    /// Set by the cache middleware on a hit; the executor short-circuits the
    /// current pipeline run with this response (typed channel — the cached
    /// value never passes through user-visible state)
    pub cache_hit: Option<CachedResponse>,
    /// Identity of the step currently executing (file:line:col#index), used
    /// to namespace default cache keys so distinct steps never collide
    pub current_step_id: Option<String>,
}

impl PipelineContext {
    /// Create a new PipelineContext with the given initial state
    pub fn new(state: Value) -> Self {
        Self {
            state,
            pending_cache_saves: Vec::new(),
            cache_hit: None,
            current_step_id: None,
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
