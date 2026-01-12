use serde_json::Value;
use crate::ast::{Tag, TagExpr};
use crate::executor::env::ExecutionEnv;
use crate::executor::context::RequestContext;

pub fn should_execute_step(condition: &Option<TagExpr>, env: &ExecutionEnv, ctx: &RequestContext, input: &Value) -> bool {
    match condition {
        None => true, // No condition means always execute
        Some(expr) => check_tag_expr(expr, env, ctx, input),
    }
}

pub fn check_tag(tag: &Tag, env: &ExecutionEnv, ctx: &RequestContext, input: &Value) -> bool {
    match tag.name.as_str() {
        "env" => check_env_tag(tag, env),
        "async" => true, // async doesn't affect execution, just how it runs
        "flag" => check_flag_tag(tag, ctx),
        "when" => check_when_tag(tag, ctx),
        "guard" => check_guard_tag(tag, input, tag.negated),
        "needs" => true, // @needs is only for static analysis, doesn't affect runtime
        _ => true,       // unknown tags don't prevent execution
    }
}

/// Check @when tag against transient conditions in RequestContext
/// All arguments must be true (AND logic). If tag.negated is true, invert the result.
pub fn check_when_tag(tag: &Tag, ctx: &RequestContext) -> bool {
    if tag.args.is_empty() {
        return true; // Invalid @when() tag without args, don't prevent execution
    }

    // Check all condition arguments - all must be met for step to execute
    for condition_name in &tag.args {
        let is_met = ctx.conditions.get(condition_name.as_str())
            .copied()
            .unwrap_or(false); // Default: False (Fail Closed)

        let matches = if tag.negated {
            !is_met // @!when(admin) - execute if condition is NOT met
        } else {
            is_met  // @when(admin) - execute if condition IS met
        };

        // If any condition check fails, the step should not execute
        if !matches {
            return false;
        }
    }

    true
}

/// Evaluate a boolean tag expression (for dispatch routing)
/// Supports AND, OR operations with proper short-circuit evaluation
pub fn check_tag_expr(expr: &TagExpr, env: &ExecutionEnv, ctx: &RequestContext, input: &Value) -> bool {
    match expr {
        TagExpr::Tag(tag) => check_tag(tag, env, ctx, input),
        TagExpr::And(left, right) => {
            // Short-circuit: if left is false, don't evaluate right
            check_tag_expr(left, env, ctx, input) && check_tag_expr(right, env, ctx, input)
        }
        TagExpr::Or(left, right) => {
            // Short-circuit: if left is true, don't evaluate right
            check_tag_expr(left, env, ctx, input) || check_tag_expr(right, env, ctx, input)
        }
    }
}

pub fn check_flag_tag(tag: &Tag, ctx: &RequestContext) -> bool {
    if tag.args.is_empty() {
        return true; // Invalid @flag() tag without args, don't prevent execution
    }

    // Check all flag arguments - all must be enabled for step to execute
    for flag_name in &tag.args {
        let is_enabled = ctx.feature_flags.get(flag_name.as_str())
            .copied()
            .unwrap_or(false); // Default: False (Fail Closed)

        let matches = if tag.negated {
            !is_enabled // @!flag(beta) - execute if flag is NOT enabled
        } else {
            is_enabled  // @flag(beta) - execute if flag IS enabled
        };

        // If any flag check fails, the step should not execute
        if !matches {
            return false;
        }
    }

    true
}

pub fn check_env_tag(tag: &Tag, env: &ExecutionEnv) -> bool {
    if tag.args.len() != 1 {
        return true; // Invalid @env() tag, don't prevent execution
    }

    let required_env = &tag.args[0];

    match &env.environment {
        Some(current_env) => {
            let matches = current_env == required_env;
            if tag.negated {
                !matches // @!env(production) - execute if NOT production
            } else {
                matches  // @env(production) - execute if IS production
            }
        }
        None => {
            // No environment set: execute non-negated tags, skip negated ones
            !tag.negated
        }
    }
}

/// Check @guard tag - evaluates a JQ expression against the pipeline input
/// Returns true if the step should execute, false otherwise
///
/// # Examples
/// - `@guard(.user.role == "admin")` - execute only if user is admin
/// - `@!guard(.debug)` - execute only if debug is NOT truthy
pub fn check_guard_tag(tag: &Tag, input: &Value, negated: bool) -> bool {
    // 1. Extract JQ filter from args
    let filter = match tag.args.first() {
        Some(f) => f,
        None => {
            tracing::warn!("@guard tag without filter argument - defaulting to true");
            return true; // Guard without arg defaults to true
        }
    };

    // 2. Run JQ logic (uses thread-local cache for performance)
    match crate::middleware::jq_eval_bool(filter, input) {
        Ok(result) => {
            // Apply negation if present
            if negated {
                !result
            } else {
                result
            }
        }
        Err(e) => {
            // Log error and fail closed (don't execute) for safety
            tracing::warn!("Guard JQ error: {} - failing closed (skipping step)", e);
            false
        }
    }
}

/// Extract @async(name) from a TagExpr, walking the expression tree
pub fn get_async_from_tag_expr(expr: &TagExpr) -> Option<String> {
    match expr {
        TagExpr::Tag(tag) => {
            if tag.name == "async" && !tag.negated && tag.args.len() == 1 {
                Some(tag.args[0].clone())
            } else {
                None
            }
        }
        TagExpr::And(left, right) | TagExpr::Or(left, right) => {
            get_async_from_tag_expr(left).or_else(|| get_async_from_tag_expr(right))
        }
    }
}

/// Extract @result(name) from a TagExpr, walking the expression tree
/// Returns the first @result tag found with a single argument
pub fn get_result_from_tag_expr(expr: &TagExpr) -> Option<String> {
    match expr {
        TagExpr::Tag(tag) => {
            if tag.name == "result" && !tag.negated && tag.args.len() == 1 {
                Some(tag.args[0].clone())
            } else {
                None
            }
        }
        TagExpr::And(left, right) | TagExpr::Or(left, right) => {
            get_result_from_tag_expr(left).or_else(|| get_result_from_tag_expr(right))
        }
    }
}
