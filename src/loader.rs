use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;

use crate::ast::{Program, parse_program};
use crate::error::{WebPipeError, Result};

/// Module loader for handling .wp file imports
/// Provides caching and circular import detection
pub struct ModuleLoader {
    /// Cache of parsed programs by absolute path
    cache: HashMap<PathBuf, Program>,

    /// Stack of currently loading files (for circular import detection)
    loading_stack: Vec<PathBuf>,
}

impl ModuleLoader {
    /// Create a new module loader
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            loading_stack: Vec::new(),
        }
    }

    /// Load a module from a file path
    /// Returns the parsed Program or an error
    pub fn load_module(&mut self, path: &Path) -> Result<Program> {
        // Convert to absolute path for consistent caching
        let abs_path = path.canonicalize().map_err(|e| {
            WebPipeError::ImportNotFound(format!(
                "Cannot resolve path '{}': {}",
                path.display(),
                e
            ))
        })?;

        // Check cache first
        if let Some(program) = self.cache.get(&abs_path) {
            return Ok(program.clone());
        }

        // Check for circular imports
        if self.loading_stack.contains(&abs_path) {
            let cycle = self.format_import_cycle(&abs_path);
            return Err(WebPipeError::CircularImport(cycle));
        }

        // Push to loading stack
        self.loading_stack.push(abs_path.clone());

        // Load and parse the file
        let content = fs::read_to_string(&abs_path).map_err(|e| {
            WebPipeError::ImportNotFound(format!(
                "Cannot read file '{}': {}",
                abs_path.display(),
                e
            ))
        })?;

        let (_remaining, program) = parse_program(&content).map_err(|e| {
            WebPipeError::ParseError(format!(
                "Failed to parse '{}': {}",
                abs_path.display(),
                e
            ))
        })?;

        // Recursively load imports for validation
        for import in &program.imports {
            let import_path = self.resolve_import_path(&abs_path, &import.path)?;
            // Recursive load - will cache and detect cycles
            self.load_module(&import_path)?;
        }

        // Pop from loading stack
        self.loading_stack.pop();

        // Cache the result
        self.cache.insert(abs_path.clone(), program.clone());

        Ok(program)
    }

    /// Resolve an import path relative to the current file
    /// Returns the absolute path to the imported file
    pub fn resolve_import_path(
        &self,
        current_file: &Path,
        import_path: &str,
    ) -> Result<PathBuf> {
        // Get the directory of the current file
        let current_dir = current_file.parent().ok_or_else(|| {
            WebPipeError::ImportNotFound(format!(
                "Cannot determine parent directory of '{}'",
                current_file.display()
            ))
        })?;

        // Resolve the import path relative to the current directory
        let resolved = current_dir.join(import_path);

        // Convert to absolute path
        let abs_path = resolved.canonicalize().map_err(|e| {
            WebPipeError::ImportNotFound(format!(
                "Cannot resolve import '{}' from '{}': {}",
                import_path,
                current_file.display(),
                e
            ))
        })?;

        Ok(abs_path)
    }

    /// Format a circular import cycle for error messages
    fn format_import_cycle(&self, target: &Path) -> String {
        let mut cycle_parts: Vec<String> = self
            .loading_stack
            .iter()
            .skip_while(|p| *p != target)
            .map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("?")
                    .to_string()
            })
            .collect();

        // Add the target again to close the cycle
        if let Some(name) = target.file_name().and_then(|n| n.to_str()) {
            cycle_parts.push(name.to_string());
        }

        cycle_parts.join(" -> ")
    }

    /// Get the cached program for a given path (if available)
    pub fn get_cached(&self, path: &Path) -> Option<&Program> {
        self.cache.get(path)
    }

    /// Clear the module cache
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

impl Default for ModuleLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_load_module() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.wp");

        let mut file = fs::File::create(&file_path).unwrap();
        file.write_all(b"GET /test\n  |> jq: `{}`").unwrap();

        let mut loader = ModuleLoader::new();
        let result = loader.load_module(&file_path);

        assert!(result.is_ok());
        let program = result.unwrap();
        assert_eq!(program.routes.len(), 1);
    }

    #[test]
    fn test_caching() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.wp");

        let mut file = fs::File::create(&file_path).unwrap();
        file.write_all(b"GET /test\n  |> jq: `{}`").unwrap();

        let mut loader = ModuleLoader::new();

        // First load
        let result1 = loader.load_module(&file_path);
        assert!(result1.is_ok());

        // Second load should hit cache
        let result2 = loader.load_module(&file_path);
        assert!(result2.is_ok());

        // Verify cache contains the file
        let abs_path = file_path.canonicalize().unwrap();
        assert!(loader.get_cached(&abs_path).is_some());
    }

    #[test]
    fn test_resolve_import_path() {
        let temp_dir = TempDir::new().unwrap();
        let main_file = temp_dir.path().join("main.wp");
        let lib_file = temp_dir.path().join("lib.wp");

        fs::File::create(&main_file).unwrap();
        fs::File::create(&lib_file).unwrap();

        let loader = ModuleLoader::new();
        let resolved = loader.resolve_import_path(&main_file, "./lib.wp");

        assert!(resolved.is_ok());
        assert_eq!(resolved.unwrap(), lib_file.canonicalize().unwrap());
    }

    #[test]
    fn test_import_not_found() {
        let temp_dir = TempDir::new().unwrap();
        let main_file = temp_dir.path().join("main.wp");
        fs::File::create(&main_file).unwrap();

        let loader = ModuleLoader::new();
        let result = loader.resolve_import_path(&main_file, "./nonexistent.wp");

        assert!(result.is_err());
        match result {
            Err(WebPipeError::ImportNotFound(_)) => {},
            _ => panic!("Expected ImportNotFound error"),
        }
    }

    #[test]
    fn test_circular_import_detection() {
        let temp_dir = TempDir::new().unwrap();
        let file_a = temp_dir.path().join("a.wp");
        let file_b = temp_dir.path().join("b.wp");

        // a.wp imports b.wp
        let mut file = fs::File::create(&file_a).unwrap();
        file.write_all(b"import \"./b.wp\" as b\n").unwrap();

        // b.wp imports a.wp (circular!)
        let mut file = fs::File::create(&file_b).unwrap();
        file.write_all(b"import \"./a.wp\" as a\n").unwrap();

        let mut loader = ModuleLoader::new();
        let result = loader.load_module(&file_a);

        assert!(result.is_err());
        match result {
            Err(WebPipeError::CircularImport(msg)) => {
                assert!(msg.contains("a.wp"));
                assert!(msg.contains("b.wp"));
            },
            _ => panic!("Expected CircularImport error"),
        }
    }
}
