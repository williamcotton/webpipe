use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Module ID type - used to uniquely identify loaded modules
/// Using integers for fast lookups and small memory footprint
pub type ModuleId = usize;

/// Metadata about a loaded module
#[derive(Clone, Debug)]
pub struct ModuleMetadata {
    /// Unique module ID
    pub id: ModuleId,
    /// Canonical absolute path to the module file
    pub path: PathBuf,
    /// Import map: alias → Module ID of imported module
    pub import_map: HashMap<String, ModuleId>,
}

/// Registry of all loaded modules
/// Maps Module IDs to their metadata for context-aware resolution
#[derive(Clone, Debug)]
pub struct ModuleRegistry {
    /// Module metadata by ID
    modules: HashMap<ModuleId, ModuleMetadata>,
    /// Reverse lookup: path → Module ID
    path_to_id: HashMap<PathBuf, ModuleId>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self {
            modules: HashMap::new(),
            path_to_id: HashMap::new(),
        }
    }

    /// Get module metadata by ID
    pub fn get_module(&self, id: ModuleId) -> Option<&ModuleMetadata> {
        self.modules.get(&id)
    }

    /// Get module ID by path
    pub fn get_id_by_path(&self, path: &Path) -> Option<ModuleId> {
        self.path_to_id.get(path).copied()
    }

    /// Register a new module and return its assigned ID
    pub fn register(&mut self, path: PathBuf, import_map: HashMap<String, ModuleId>) -> ModuleId {
        // Assign next available ID
        let id = self.modules.len();

        let metadata = ModuleMetadata {
            id,
            path: path.clone(),
            import_map,
        };

        self.modules.insert(id, metadata);
        self.path_to_id.insert(path, id);

        id
    }

    /// Get all modules (for iteration/validation)
    pub fn modules(&self) -> &HashMap<ModuleId, ModuleMetadata> {
        &self.modules
    }
}
