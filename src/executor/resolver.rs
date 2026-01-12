use serde_json::Value;
use crate::executor::env::ExecutionEnv;
use crate::ast::{SourceLocation, TagExpr, ResultBranch, ResultBranchType};
use crate::executor::tags::get_result_from_tag_expr;

/// Parse a scoped reference: "namespace::name" OR "name"
/// Returns (namespace, name) where namespace is None for local references
pub fn parse_scoped_ref(config: &str) -> (Option<String>, String) {
    if let Some(idx) = config.find("::") {
        let namespace = config[..idx].to_string();
        let name = config[idx+2..].to_string();
        (Some(namespace), name)
    } else {
        (None, config.to_string())
    }
}

pub fn resolve_config_and_autoname(
    env: &ExecutionEnv,
    step_location: &SourceLocation,
    middleware_name: &str,
    step_config: &str,
    input: &Value,
) -> (String, Value, bool) {
    // Parse step_config for scoped references: alias::name
    let (alias, var_name) = parse_scoped_ref(step_config);

    // If there's an alias, do context-aware resolution using Module IDs
    if let Some(alias) = alias {
        // Get the module ID from the step's location
        if let Some(step_module_id) = step_location.module_id {
            // Get the module metadata for the step's module
            if let Some(step_module) = env.module_registry.get_module(step_module_id) {
                // Resolve the alias to the target Module ID using this module's import map
                if let Some(&target_module_id) = step_module.import_map.get(&alias) {
                    // Look up the variable in the target module
                    let key = (Some(target_module_id), middleware_name.to_string(), var_name.clone());
                    if let Some(var) = env.variables.get(&key) {
                        let resolved_config = var.value.clone();
                        let mut new_input = input.clone();
                        let mut auto_named = false;
                        if let Some(obj) = new_input.as_object_mut() {
                            if !obj.contains_key("resultName") {
                                obj.insert("resultName".to_string(), Value::String(var.name.clone()));
                                auto_named = true;
                            }
                        }
                        return (resolved_config, new_input, auto_named);
                    } else {
                        tracing::warn!("Variable not found with key: ({:?}, {}, {})", Some(target_module_id), middleware_name, var_name);
                    }
                } else {
                    tracing::warn!("Alias '{}' not found in import map for module_id {}", alias, step_module_id);
                }
            } else {
                tracing::warn!("Module metadata not found for module_id {}", step_module_id);
            }
        } else {
            tracing::warn!("Step has no module_id, but has scoped reference {}::{}", alias, var_name);
        }
        // If we couldn't resolve the scoped reference, return as-is (will error downstream)
        return (step_config.to_string(), input.clone(), false);
    }

    // No alias - try local variable lookup using step's module_id
    // First try with step's module_id (for module-based resolution)
    let module_id_opt = step_location.module_id;
    let key = (module_id_opt, middleware_name.to_string(), var_name.clone());

    if let Some(var) = env.variables.get(&key) {
        let resolved_config = var.value.clone();
        let mut new_input = input.clone();
        let mut auto_named = false;
        if let Some(obj) = new_input.as_object_mut() {
            if !obj.contains_key("resultName") {
                obj.insert("resultName".to_string(), Value::String(var.name.clone()));
                auto_named = true;
            }
        }
        return (resolved_config, new_input, auto_named);
    }

    // Fallback: try with module_id = None (backward compatibility for symbols registered without module_id)
    if module_id_opt.is_some() {
        let fallback_key = (None, middleware_name.to_string(), var_name.clone());
        if let Some(var) = env.variables.get(&fallback_key) {
            let resolved_config = var.value.clone();
            let mut new_input = input.clone();
            let mut auto_named = false;
            if let Some(obj) = new_input.as_object_mut() {
                if !obj.contains_key("resultName") {
                    obj.insert("resultName".to_string(), Value::String(var.name.clone()));
                    auto_named = true;
                }
            }
            return (resolved_config, new_input, auto_named);
        }
    }

    // If local variable not found, return as-is (might be a literal value)
    (step_config.to_string(), input.clone(), false)
}

pub fn extract_error_type(input: &Value) -> Option<String> {
    if let Some(errors) = input.get("errors") {
        if let Some(errors_array) = errors.as_array() {
            if let Some(first_error) = errors_array.first() {
                if let Some(error_type) = first_error.get("type").and_then(|v| v.as_str()) {
                    return Some(error_type.to_string());
                }
            }
        }
    }
    None
}

/// Determine the target name for result wrapping with correct precedence:
/// 1. @result(name) tag (highest priority)
/// 2. resultName from state (legacy explicit + auto-named)
///
/// Returns (target_name, should_cleanup_result_name)
pub fn determine_target_name(
    condition: &Option<TagExpr>,
    state: &Value,
    auto_named: bool,
) -> (Option<String>, bool) {
    // Priority 1: @result(name) tag
    if let Some(ref expr) = condition {
        if let Some(result_name) = get_result_from_tag_expr(expr) {
            return (Some(result_name), false);
        }
    }

    // Priority 2 & 3: resultName from state (covers both explicit and auto-named)
    if let Some(result_name) = state.get("resultName").and_then(|v| v.as_str()) {
        return (Some(result_name.to_string()), auto_named);
    }

    (None, false)
}

pub fn select_branch<'a>(
    branches: &'a [ResultBranch],
    error_type: &Option<String>,
) -> Option<&'a ResultBranch> {

    if let Some(err_type) = error_type {
        if let Some(branch) = branches.iter().find(|b| matches!(&b.branch_type, ResultBranchType::Custom(name) if name == err_type)) {
            return Some(branch);
        }
    } else if let Some(branch) = branches.iter().find(|b| matches!(b.branch_type, ResultBranchType::Ok)) {
        return Some(branch);
    }

    branches
        .iter()
        .find(|b| matches!(b.branch_type, ResultBranchType::Default))
}
