use crate::ast::*;
use crate::error::WebPipeError;
use crate::middleware::MiddlewareRegistry;
use axum::http::{Method, StatusCode};
use serde_json::Value;

pub struct WebPipeRuntime {
    pub program: Program,
    pub middleware_registry: MiddlewareRegistry,
}

impl WebPipeRuntime {
    pub fn new(program: Program) -> Self {
        Self {
            program,
            middleware_registry: MiddlewareRegistry::new(),
        }
    }

    pub async fn execute_route(
        &self,
        method: &Method,
        path: &str,
        input: Value,
    ) -> Result<(StatusCode, Value), WebPipeError> {
        // Find matching route
        let route = self.find_matching_route(method, path)?;
        
        // Execute the pipeline
        match &route.pipeline {
            PipelineRef::Inline(pipeline) => {
                self.execute_pipeline(pipeline, input).await
            }
            PipelineRef::Named(name) => {
                let pipeline = self.find_named_pipeline(name)?;
                self.execute_pipeline(&pipeline.pipeline, input).await
            }
        }
    }

    pub fn execute_pipeline<'a>(
        &'a self,
        pipeline: &'a Pipeline,
        input: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(StatusCode, Value), WebPipeError>> + 'a>> {
        Box::pin(async move {
            let mut current_input = input;
            let status_code = StatusCode::OK;

            for step in &pipeline.steps {
                match step {
                    PipelineStep::Regular { name, config } => {
                        let result = self.middleware_registry
                            .execute(name, config, &current_input)
                            .await?;
                        current_input = result;
                    }
                    PipelineStep::Result { branches } => {
                        return self.execute_result_branches(branches, current_input).await;
                    }
                }
            }

            Ok((status_code, current_input))
        })
    }

    fn execute_result_branches<'a>(
        &'a self,
        branches: &'a [ResultBranch],
        input: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(StatusCode, Value), WebPipeError>> + 'a>> {
        Box::pin(async move {
            // For now, execute the first branch as a simple implementation
            // In a full implementation, this would check current status and route accordingly
            if let Some(branch) = branches.first() {
                let result = self.execute_pipeline(&branch.pipeline, input).await?;
                Ok((StatusCode::from_u16(branch.status_code).unwrap_or(StatusCode::OK), result.1))
            } else {
                Ok((StatusCode::OK, input))
            }
        })
    }

    fn find_matching_route(&self, method: &Method, path: &str) -> Result<&Route, WebPipeError> {
        self.program
            .routes
            .iter()
            .find(|route| route.method == method.as_str() && self.path_matches(&route.path, path))
            .ok_or_else(|| WebPipeError::RouteNotFound(format!("{} {}", method, path)))
    }

    fn find_named_pipeline(&self, name: &str) -> Result<&NamedPipeline, WebPipeError> {
        self.program
            .pipelines
            .iter()
            .find(|pipeline| pipeline.name == name)
            .ok_or_else(|| WebPipeError::PipelineNotFound(name.to_string()))
    }

    fn path_matches(&self, pattern: &str, path: &str) -> bool {
        // Simple implementation - in full version would handle path parameters
        pattern == path || pattern.contains(':')
    }
}