use async_recursion::async_recursion;
use serde_json::Value;

use crate::{
    ast::{Pipeline, PipelineStep},
    error::WebPipeError,
};

pub mod context;
pub mod env;
pub mod modules;
pub mod resolver;
pub mod step;
pub mod tags;
pub mod tasks;
pub mod types;

// Re-exports for backward compatibility and convenience
pub use context::{RequestContext, RequestMetadata, Profiler};
pub use context::StepMode;
pub use env::{ExecutionEnv, MiddlewareInvoker, RealInvoker};
pub use modules::{ModuleId, ModuleMetadata, ModuleRegistry};
pub use resolver::{resolve_config_and_autoname, parse_scoped_ref, determine_target_name, extract_error_type, select_branch};
pub use tags::{should_execute_step, check_tag_expr, get_async_from_tag_expr, get_result_from_tag_expr};
pub use tasks::{AsyncTaskRegistry, parse_join_task_names};
pub use types::{PipelineOutput, StepOutcome, ExecutionMode, StepResult, PipelineExecFuture, CachePolicy, LogConfig, RateLimitStatus};
pub use step::{StepContext, RegularStepExecutor, determine_step_name};

/// Dispatch to the appropriate step handler
async fn execute_step<'a>(
    step: &'a PipelineStep,
    idx: usize,
    all_steps: &'a [PipelineStep],
    env: &'a ExecutionEnv,
    ctx: &'a mut RequestContext,
    pipeline_ctx: &mut crate::runtime::PipelineContext,
    current_output: &mut PipelineOutput,
) -> Result<StepOutcome, WebPipeError> {
    // 1. Create StepContext to bundle execution state
    let mut step_ctx = step::StepContext {
        env,
        req_ctx: ctx,
        pipe_ctx: pipeline_ctx,
        output: current_output,
        step_index: idx,
        all_steps,
    };

    // 2. Determine step name for profiling
    let step_name = step::determine_step_name(step);

    // 3. Start Profiling
    step_ctx.req_ctx.profiler.push(&step_name);
    let start_time = std::time::Instant::now();

    // 4. Debugger Hook: before_step
    step::handle_debugger_before(&mut step_ctx, step, &step_name).await?;

    // 5. Dispatch to appropriate handler
    let result = match step {
        PipelineStep::Regular { name, args, config, condition, parsed_join_targets, location, .. } => {
            step::RegularStepExecutor::new(
                name,
                args,
                config,
                condition,
                parsed_join_targets.as_ref(),
                location,
            )
            .execute(&mut step_ctx)
            .await
        }
        PipelineStep::Result { branches, .. } => {
            step::handle_result_step(branches, &mut step_ctx).await
        }
        PipelineStep::If { condition, then_branch, else_branch, .. } => {
            step::handle_if_step(condition, then_branch, else_branch, &mut step_ctx).await
        }
        PipelineStep::Dispatch { branches, default, .. } => {
            step::handle_dispatch_step(branches, default, &mut step_ctx).await
        }
        PipelineStep::Foreach { selector, pipeline, .. } => {
            step::handle_foreach_step(selector, pipeline, &mut step_ctx).await
        }
    };

    // 6. Debugger Hook: after_step
    step::handle_debugger_after(&mut step_ctx, step, &step_name).await;

    // 7. End Profiling
    let elapsed = start_time.elapsed().as_micros();
    step_ctx.req_ctx.profiler.record_sample(elapsed);
    step_ctx.req_ctx.profiler.pop();

    result
}

/// Internal pipeline execution that accepts a mutable RequestContext
/// Public to allow GraphQL runtime to reuse the same context for call logging
#[async_recursion]
pub async fn execute_pipeline_internal<'a>(
    env: &'a ExecutionEnv,
    pipeline: &'a Pipeline,
    input: Value,
    ctx: &'a mut RequestContext,
) -> Result<(Value, String, Option<u16>), WebPipeError> {
    // Track content_type and status_code as we execute steps
    let mut content_type = "application/json".to_string();
    let mut status_code: Option<u16> = None;

    let mut pipeline_ctx = crate::runtime::PipelineContext::new(input);

    for (idx, step) in pipeline.steps.iter().enumerate() {
        // Create output tracker for this iteration
        let mut current_output = PipelineOutput {
            state: Value::Null, // Not used during step execution
            content_type: content_type.clone(),
            status_code,
        };

        let outcome = execute_step(
            step,
            idx,
            &pipeline.steps,
            env,
            ctx,
            &mut pipeline_ctx,
            &mut current_output,
        ).await?;

        // Update tracked metadata
        content_type = current_output.content_type;
        status_code = current_output.status_code;

        match outcome {
            StepOutcome::Continue => continue,
            StepOutcome::Return(output) => return Ok((output.state, output.content_type, output.status_code)),
        }
    }

    // Return final result with accumulated state and metadata
    Ok((pipeline_ctx.state, content_type, status_code))
}

/// Public entry point for pipeline execution
/// Executes pipeline and returns the context so deferred actions can be run
pub async fn execute_pipeline<'a>(
    env: &'a ExecutionEnv,
    pipeline: &'a Pipeline,
    input: Value,
    mut ctx: RequestContext,
) -> Result<(Value, String, Option<u16>, RequestContext), WebPipeError> {
    let result = execute_pipeline_internal(env, pipeline, input, &mut ctx).await?;
    Ok((result.0, result.1, result.2, ctx))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{PipelineStep, Pipeline, TagExpr, Tag, Variable};
    use crate::middleware::MiddlewareRegistry;
    use crate::runtime::context::{CacheStore, RateLimitStore};
    use std::sync::Arc;
    use std::collections::HashMap;
    use async_trait::async_trait;

    /// Helper to create a condition from a single tag
    fn single_tag(name: &str, negated: bool, args: Vec<&str>) -> Option<TagExpr> {
        Some(TagExpr::Tag(Tag {
            name: name.to_string(),
            negated,
            args: args.into_iter().map(|s| s.to_string()).collect(),
        }))
    }

    /// Helper to create an AND condition from two tags
    fn and_tags(tag1: TagExpr, tag2: TagExpr) -> Option<TagExpr> {
        Some(TagExpr::And(Box::new(tag1), Box::new(tag2)))
    }

    struct StubInvoker;
    #[async_trait]
    impl MiddlewareInvoker for StubInvoker {
        async fn call(
            &self,
            name: &str,
            _args: &[String],
            cfg: &str,
            pipeline_ctx: &mut crate::runtime::PipelineContext,
            _env: &ExecutionEnv,
            _ctx: &mut RequestContext,
            _target_name: Option<&str>,
        ) -> Result<crate::middleware::MiddlewareOutput, WebPipeError> {
            let input = &pipeline_ctx.state;
            match name {
                "handlebars" => {
                    pipeline_ctx.state = Value::String(format!("<p>{}</p>", cfg));
                    return Ok(crate::middleware::MiddlewareOutput {
                        content_type: Some("text/html".to_string()),
                    });
                }
                "echo" => {
                    // Try to parse config as JSON and merge with input
                    if let Ok(json_cfg) = serde_json::from_str::<Value>(cfg) {
                        // Merge config into input (config takes precedence)
                        let mut result = input.clone();
                        if let (Some(input_obj), Some(cfg_obj)) = (result.as_object_mut(), json_cfg.as_object()) {
                            for (k, v) in cfg_obj {
                                input_obj.insert(k.clone(), v.clone());
                            }
                        }
                        pipeline_ctx.state = result;
                    } else {
                        pipeline_ctx.state = serde_json::json!({"echo": cfg, "inputCopy": input });
                    }
                },
                _ => {
                    pipeline_ctx.state = serde_json::json!({"ok": true});
                }
            }
            Ok(crate::middleware::MiddlewareOutput::default())
        }
    }

    fn env_with_vars(_vars: Vec<Variable>) -> ExecutionEnv {
        let registry = Arc::new(MiddlewareRegistry::empty());
        // For tests, we use the vars to populate the map. 
        // In the original test helper, it created a map. 
        // Here we need to map Variable to the key format.
        let mut var_map = HashMap::new();
        for var in _vars {
             // For tests, assume no module (None) and var_type matches name or generic
             var_map.insert((None, var.var_type.clone(), var.name.clone()), var);
        }

        ExecutionEnv {
            variables: Arc::new(var_map),
            named_pipelines: Arc::new(HashMap::new()),
            imports: Arc::new(std::collections::HashMap::new()),
            invoker: Arc::new(StubInvoker),
            registry,
            environment: None,

            cache: CacheStore::new(8, 60),
            rate_limit: RateLimitStore::new(1000),
            module_registry: Arc::new(ModuleRegistry::new()),
            debugger: None,
        }
    }


    #[tokio::test]
    async fn result_branch_selection_and_status() {
        let env = env_with_vars(vec![]);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "echo".to_string(), args: Vec::new(), config: "{}".to_string(), config_type: crate::ast::ConfigType::Quoted, condition: None, parsed_join_targets: None, location: crate::ast::SourceLocation::default() },
            PipelineStep::Result { branches: vec![
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Ok, status_code: 201, pipeline: Pipeline { steps: vec![] }, location: crate::ast::SourceLocation::default() },
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Default, status_code: 200, pipeline: Pipeline { steps: vec![] }, location: crate::ast::SourceLocation::default() },
            ], location: crate::ast::SourceLocation::default() }
        ]};
        let (out, _ct, status, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.is_object());
        assert_eq!(status, Some(201));
    }

    #[tokio::test]
    async fn result_branch_custom_error_type_selected() {
        let env = env_with_vars(vec![]);
        let input = serde_json::json!({ "errors": [ { "type": "validationError", "message": "bad" } ] });
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Result { branches: vec![
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Custom("validationError".to_string()), status_code: 422, pipeline: Pipeline { steps: vec![] }, location: crate::ast::SourceLocation::default() },
                crate::ast::ResultBranch { branch_type: crate::ast::ResultBranchType::Default, status_code: 500, pipeline: Pipeline { steps: vec![] }, location: crate::ast::SourceLocation::default() },
            ], location: crate::ast::SourceLocation::default() }
        ]};
        let (_out, _ct, status, _ctx) = execute_pipeline(&env, &pipeline, input, RequestContext::new()).await.unwrap();
        assert_eq!(status, Some(422));
    }

    #[tokio::test]
    async fn handlebars_sets_html_content_type() {
        let env = env_with_vars(vec![]);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "handlebars".to_string(), args: Vec::new(), config: "Hello".to_string(), config_type: crate::ast::ConfigType::Quoted, condition: None, parsed_join_targets: None, location: crate::ast::SourceLocation::default() }
        ]};
        let (_out, ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert_eq!(ct, "text/html");
    }

    #[tokio::test]
    async fn variable_auto_naming_adds_and_removes_result_name() {
        let vars = vec![Variable { var_type: "echo".to_string(), name: "myVar".to_string(), value: "{}".to_string() }];
        let env = env_with_vars(vars);
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular { name: "echo".to_string(), args: Vec::new(), config: "myVar".to_string(), config_type: crate::ast::ConfigType::Identifier, condition: None, parsed_join_targets: None, location: crate::ast::SourceLocation::default() }
        ]};
        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.get("resultName").is_none());
        // Ensure output is an object and not empty
        assert!(out.is_object());
    }

    #[tokio::test]
    async fn env_tag_executes_step_when_environment_matches() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("production".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "test".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", false, vec!["production"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.is_object());
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("test"));
    }

    #[tokio::test]
    async fn env_tag_skips_step_when_environment_doesnt_match() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("development".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"message": "production only"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", false, vec!["production"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({"initial": "data"}), RequestContext::new()).await.unwrap();
        // Should return initial input unchanged since step was skipped
        assert_eq!(out.get("initial").and_then(|v| v.as_str()), Some("data"));
        assert!(out.get("message").is_none());
    }

    #[tokio::test]
    async fn negated_env_tag_skips_when_environment_matches() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("production".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"debug": true}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", true, vec!["production"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({"initial": "data"}), RequestContext::new()).await.unwrap();
        // Should skip the debug step in production
        assert!(out.get("debug").is_none());
        assert_eq!(out.get("initial").and_then(|v| v.as_str()), Some("data"));
    }

    #[tokio::test]
    async fn negated_env_tag_executes_when_environment_doesnt_match() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("development".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "dev".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", true, vec!["production"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        // Should execute in development (not production)
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("dev"));
    }

    #[tokio::test]
    async fn multiple_env_tags_all_must_match() {
        let mut env = env_with_vars(vec![]);
        env.environment = Some("production".to_string());

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "executed".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: and_tags(
                    TagExpr::Tag(Tag { name: "env".to_string(), negated: false, args: vec!["production".to_string()] }),
                    TagExpr::Tag(Tag { name: "env".to_string(), negated: true, args: vec!["staging".to_string()] })
                ),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        // Should execute: env is production (✓) and env is not staging (✓)
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("executed"));
    }

    #[tokio::test]
    async fn no_environment_set_executes_non_negated_tags() {
        let env = env_with_vars(vec![]);
        // env.environment is None

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "noenv".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("env", false, vec!["production"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        // When no environment is set, non-negated tags execute
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("noenv"));
    }

    #[tokio::test]
    async fn flag_tag_skips_when_flag_disabled() {
        let env_no_flags = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "tagged".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("flag", false, vec!["beta"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        // No flags in env - should fail closed (skip the step)
        let (out, _ct, _st, _ctx) = execute_pipeline(&env_no_flags, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.get("echo").is_none());

        // With flag enabled in RequestContext - should execute
        let mut ctx_with_flag = RequestContext::new();
        ctx_with_flag.feature_flags.insert("beta".to_string(), true);
        let (out2, _ct2, _st2, _ctx2) = execute_pipeline(&env_no_flags, &pipeline, serde_json::json!({}), ctx_with_flag).await.unwrap();
        assert_eq!(out2.get("echo").and_then(|v| v.as_str()), Some("tagged"));
    }

    #[tokio::test]
    async fn non_flag_tags_do_not_prevent_execution() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "tagged".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("needs", false, vec!["flags"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        // @needs tags are for static analysis only, don't prevent execution
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("tagged"));
    }

    #[tokio::test]
    async fn async_tag_spawns_task_without_blocking() {
        let env = env_with_vars(vec![]);

        // Pipeline with async step - should continue immediately
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"async": "data"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["task1"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            },
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"sync": "data"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        // The async step should not modify the state
        assert!(out.get("async").is_none());
        // The sync step should execute and modify state
        assert_eq!(out.get("sync").and_then(|v| v.as_str()), Some("data"));
    }

    #[tokio::test]
    async fn join_waits_for_async_tasks() {
        let env = env_with_vars(vec![]);

        // Pipeline with async step followed by join
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"result": "from-async"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["task1"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                args: Vec::new(), config: "task1".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: Some(vec!["task1".to_string()]),
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        // Check that async results are available under .async.task1
        assert!(out.get("async").is_some());
        let async_obj = out.get("async").unwrap().as_object().unwrap();
        assert!(async_obj.contains_key("task1"));
        let task1_result = async_obj.get("task1").unwrap();
        assert_eq!(task1_result.get("result").and_then(|v| v.as_str()), Some("from-async"));
    }

    #[tokio::test]
    async fn join_multiple_async_tasks() {
        let env = env_with_vars(vec![]);

        // Pipeline with multiple async steps
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"data": "task1"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["task1"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            },
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"data": "task2"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["task2"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                args: Vec::new(), config: "task1,task2".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: Some(vec!["task1".to_string(), "task2".to_string()]),
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        // Check both tasks completed
        let async_obj = out.get("async").unwrap().as_object().unwrap();
        assert_eq!(async_obj.len(), 2);
        assert_eq!(async_obj.get("task1").unwrap().get("data").and_then(|v| v.as_str()), Some("task1"));
        assert_eq!(async_obj.get("task2").unwrap().get("data").and_then(|v| v.as_str()), Some("task2"));
    }

    #[tokio::test]
    async fn join_with_json_array_config() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"data": "a"}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["a"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                args: Vec::new(), config: r#"["a"]"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: Some(vec!["a".to_string()]),
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        let async_obj = out.get("async").unwrap().as_object().unwrap();
        assert_eq!(async_obj.get("a").unwrap().get("data").and_then(|v| v.as_str()), Some("a"));
    }

    #[tokio::test]
    async fn async_step_uses_state_snapshot() {
        let env = env_with_vars(vec![]);

        // Pipeline that modifies state, spawns async with that state, then modifies again
        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"counter": 1}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            },
            // Async task should see counter: 1
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"asyncSaw": 0}"#.to_string(), // Will be replaced with actual counter value
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("async", false, vec!["snapshot"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            },
            // Modify state after async spawn
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: r#"{"counter": 2}"#.to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            },
            PipelineStep::Regular {
                name: "join".to_string(),
                args: Vec::new(), config: "snapshot".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: None,
                parsed_join_targets: Some(vec!["snapshot".to_string()]),
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();

        // Main state should have counter: 2
        assert_eq!(out.get("counter").and_then(|v| v.as_u64()), Some(2));

        // Async task should have seen the snapshot with counter: 1
        let async_obj = out.get("async").unwrap().as_object().unwrap();
        let snapshot_result = async_obj.get("snapshot").unwrap();
        // The async echo will copy the input which had counter: 1
        assert!(snapshot_result.get("counter").is_some());
    }

    #[tokio::test]
    async fn when_tag_skips_when_condition_not_set() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "admin_only".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", false, vec!["is_admin"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        // No conditions set - should fail closed (skip the step)
        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert!(out.get("echo").is_none());
    }

    #[tokio::test]
    async fn when_tag_executes_when_condition_set() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "admin_only".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", false, vec!["is_admin"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        // With condition enabled in RequestContext - should execute
        let mut ctx_with_condition = RequestContext::new();
        ctx_with_condition.conditions.insert("is_admin".to_string(), true);
        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), ctx_with_condition).await.unwrap();
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("admin_only"));
    }

    #[tokio::test]
    async fn negated_when_tag_executes_when_condition_not_set() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "non_admin".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", true, vec!["is_admin"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        // No conditions set - negated @!when(is_admin) should execute
        let (out, _ct, _st, _ctx) = execute_pipeline(&env, &pipeline, serde_json::json!({}), RequestContext::new()).await.unwrap();
        assert_eq!(out.get("echo").and_then(|v| v.as_str()), Some("non_admin"));
    }

    #[tokio::test]
    async fn when_tag_multiple_conditions_all_must_be_true() {
        let env = env_with_vars(vec![]);

        let pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "admin_and_premium".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", false, vec!["is_admin", "is_premium"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        // Only one condition set - should skip
        let mut ctx_partial = RequestContext::new();
        ctx_partial.conditions.insert("is_admin".to_string(), true);
        let (out1, _ct1, _st1, _ctx1) = execute_pipeline(&env, &pipeline, serde_json::json!({}), ctx_partial).await.unwrap();
        assert!(out1.get("echo").is_none());

        // Both conditions set - should execute
        let mut ctx_both = RequestContext::new();
        ctx_both.conditions.insert("is_admin".to_string(), true);
        ctx_both.conditions.insert("is_premium".to_string(), true);
        let (out2, _ct2, _st2, _ctx2) = execute_pipeline(&env, &pipeline, serde_json::json!({}), ctx_both).await.unwrap();
        assert_eq!(out2.get("echo").and_then(|v| v.as_str()), Some("admin_and_premium"));
    }

    #[tokio::test]
    async fn when_tag_isolation_from_flag_tag() {
        let env = env_with_vars(vec![]);

        // @flag(beta) step
        let flag_pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "flag_beta".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("flag", false, vec!["beta"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        // @when(beta) step
        let when_pipeline = Pipeline { steps: vec![
            PipelineStep::Regular {
                name: "echo".to_string(),
                args: Vec::new(), config: "when_beta".to_string(),
                config_type: crate::ast::ConfigType::Quoted,
                condition: single_tag("when", false, vec!["beta"]),
                parsed_join_targets: None,
                location: crate::ast::SourceLocation::default(),
            }
        ]};

        // Set beta in conditions only (not in flags)
        let mut ctx1 = RequestContext::new();
        ctx1.conditions.insert("beta".to_string(), true);

        // @flag(beta) should skip (beta is not in feature_flags)
        let (out1, _ct1, _st1, _ctx1) = execute_pipeline(&env, &flag_pipeline, serde_json::json!({}), ctx1).await.unwrap();
        assert!(out1.get("echo").is_none(), "flag tag should not match condition");

        // @when(beta) should execute (beta is in conditions)
        let mut ctx2 = RequestContext::new();
        ctx2.conditions.insert("beta".to_string(), true);
        let (out2, _ct2, _st2, _ctx2) = execute_pipeline(&env, &when_pipeline, serde_json::json!({}), ctx2).await.unwrap();
        assert_eq!(out2.get("echo").and_then(|v| v.as_str()), Some("when_beta"));
    }

    #[tokio::test]
    async fn request_context_to_value_includes_conditions() {
        let mut ctx = RequestContext::new();
        ctx.conditions.insert("is_admin".to_string(), true);
        ctx.conditions.insert("is_mobile".to_string(), false);

        let env = env_with_vars(vec![]);
        let context_value = ctx.to_value(&env);

        // Check that conditions are exposed
        let conditions = context_value.get("conditions").expect("conditions should be present");
        assert_eq!(conditions.get("is_admin").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(conditions.get("is_mobile").and_then(|v| v.as_bool()), Some(false));
    }
}