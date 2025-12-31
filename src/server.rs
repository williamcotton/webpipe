use crate::ast::{Program, Pipeline, PipelineRef, PipelineStep, SourceLocation};
use crate::middleware::MiddlewareRegistry;
use crate::error::WebPipeError;
use axum::{
    body::{Bytes, Body},
    extract::{Query, State, OriginalUri, ConnectInfo},
    http::{Method, StatusCode, HeaderMap},
    response::{IntoResponse, Json, Html, Response},
    routing::{get, on, MethodFilter},
    Router,
};
use serde_json::Value;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::signal;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::{debug, info, warn};
use tokio::fs as tokio_fs;
use std::path::{Path, PathBuf};
use crate::runtime::Context;
use crate::executor::{ExecutionEnv, RealInvoker};
use crate::http::request::build_request_from_axum_with_ip;

// number coercion helpers moved to http::request

// merge helper moved to shared executor

/// Check if a TagExpr contains any @flag tag
fn tag_expr_has_flag(expr: &crate::ast::TagExpr) -> bool {
    match expr {
        crate::ast::TagExpr::Tag(tag) => tag.name == "flag",
        crate::ast::TagExpr::And(left, right) | crate::ast::TagExpr::Or(left, right) => {
            tag_expr_has_flag(left) || tag_expr_has_flag(right)
        }
    }
}

/// Check if a TagExpr contains @needs(flags)
fn tag_expr_has_needs_flags(expr: &crate::ast::TagExpr) -> bool {
    match expr {
        crate::ast::TagExpr::Tag(tag) => {
            tag.name == "needs" && tag.args.contains(&"flags".to_string())
        }
        crate::ast::TagExpr::And(left, right) | crate::ast::TagExpr::Or(left, right) => {
            tag_expr_has_needs_flags(left) || tag_expr_has_needs_flags(right)
        }
    }
}

/// Recursive function to detect if a pipeline requires feature flags
fn pipeline_needs_flags(
    pipeline: &Pipeline,
    named_pipelines: &HashMap<(Option<String>, String), Arc<Pipeline>>
) -> bool {
    for step in &pipeline.steps {
        match step {
            PipelineStep::Regular { name, config, condition, .. } => {
                if let Some(expr) = condition {
                    // 1. Direct Usage: @flag(...)
                    if tag_expr_has_flag(expr) {
                        return true;
                    }

                    // 2. Explicit Opt-in: @needs(flags)
                    if tag_expr_has_needs_flags(expr) {
                        return true;
                    }
                }

                // 3. Recursive Reference: |> pipeline: name
                if name == "pipeline" {
                    let target = config.trim();
                    let key = (None, target.to_string());
                    if let Some(sub) = named_pipelines.get(&key) {
                        if pipeline_needs_flags(sub, named_pipelines) {
                            return true;
                        }
                    }
                }
            },
            PipelineStep::Result { branches, .. } => {
                // 4. Recursive Branching: |> result ...
                for branch in branches {
                    if pipeline_needs_flags(&branch.pipeline, named_pipelines) {
                        return true;
                    }
                }
            },
            PipelineStep::If { condition, then_branch, else_branch, .. } => {
                // 5. If/Else blocks: check condition, then, and else branches
                if pipeline_needs_flags(condition, named_pipelines) {
                    return true;
                }
                if pipeline_needs_flags(then_branch, named_pipelines) {
                    return true;
                }
                if let Some(else_pipe) = else_branch {
                    if pipeline_needs_flags(else_pipe, named_pipelines) {
                        return true;
                    }
                }
            }
            PipelineStep::Dispatch { branches, default, .. } => {
                // 6. Dispatch blocks: check all case branches and tags
                for branch in branches {
                    // Check if branch condition expression contains a flag tag
                    if tag_expr_has_flag(&branch.condition) {
                        return true;
                    }
                    // Check if branch pipeline needs flags
                    if pipeline_needs_flags(&branch.pipeline, named_pipelines) {
                        return true;
                    }
                }
                // Check default branch if present
                if let Some(default_pipe) = default {
                    if pipeline_needs_flags(default_pipe, named_pipelines) {
                        return true;
                    }
                }
            }
            PipelineStep::Foreach { pipeline, .. } => {
                // 7. Foreach blocks: check inner pipeline
                if pipeline_needs_flags(pipeline, named_pipelines) {
                    return true;
                }
            }
        }
    }
    false
}

/// Create a synthetic pipeline for the automatic GraphQL endpoint
pub fn create_graphql_endpoint_pipeline() -> Pipeline {
    use crate::ast::ConfigType;

    Pipeline {
        steps: vec![
            // Extract query and variables from request body
            PipelineStep::Regular {
                name: "jq".to_string(),
                args: vec![],
                config: r#"{
                    query: .body.query,
                    variables: (.body.variables // {})
                }"#.to_string(),
                config_type: ConfigType::Backtick,
                condition: None,
                parsed_join_targets: None,
                location: SourceLocation { line: 0, column: 0, offset: 0, file_path: None },
            },
            // Execute GraphQL with empty config (triggers dynamic mode)
            PipelineStep::Regular {
                name: "graphql".to_string(),
                args: vec![],
                config: "".to_string(),
                config_type: ConfigType::Backtick,
                condition: None,
                parsed_join_targets: None,
                location: SourceLocation { line: 0, column: 0, offset: 0, file_path: None },
            },
        ],
    }
}

/// Helper function to update all SourceLocations in a pipeline with a file path
fn set_pipeline_file_path(pipeline: &mut Pipeline, file_path: &str) {
    for step in &mut pipeline.steps {
        match step {
            PipelineStep::Regular { location, .. } => {
                location.file_path = Some(file_path.to_string());
            }
            PipelineStep::Result { location, branches, .. } => {
                location.file_path = Some(file_path.to_string());
                for branch in branches {
                    set_pipeline_file_path(&mut branch.pipeline, file_path);
                }
            }
            PipelineStep::If { location, condition, then_branch, else_branch, .. } => {
                location.file_path = Some(file_path.to_string());
                set_pipeline_file_path(condition, file_path);
                set_pipeline_file_path(then_branch, file_path);
                if let Some(else_pipe) = else_branch {
                    set_pipeline_file_path(else_pipe, file_path);
                }
            }
            PipelineStep::Dispatch { location, branches, default, .. } => {
                location.file_path = Some(file_path.to_string());
                for branch in branches {
                    set_pipeline_file_path(&mut branch.pipeline, file_path);
                }
                if let Some(default_pipe) = default {
                    set_pipeline_file_path(default_pipe, file_path);
                }
            }
            PipelineStep::Foreach { location, pipeline, .. } => {
                location.file_path = Some(file_path.to_string());
                set_pipeline_file_path(pipeline, file_path);
            }
        }
    }
}

/// Load imports and collect configs from imported modules
fn load_imported_configs(
    program: &Program,
    file_path: Option<&PathBuf>,
) -> Vec<crate::ast::Config> {
    let mut all_configs: Vec<crate::ast::Config> = Vec::new();

    if let Some(file_path) = file_path {
        if !program.imports.is_empty() {
            let mut loader = crate::loader::ModuleLoader::new();

            for import in &program.imports {
                match loader.resolve_import_path(file_path, &import.path) {
                    Ok(resolved_path) => {
                        match loader.load_module(&resolved_path) {
                            Ok(imported_program) => {
                                // Collect imported configs
                                all_configs.extend(imported_program.configs.clone());
                            },
                            Err(e) => {
                                warn!("Failed to load imported module '{}' from '{}': {}",
                                    import.alias, import.path, e);
                            }
                        }
                    },
                    Err(e) => {
                        warn!("Failed to resolve import '{}' from '{}': {}",
                            import.alias, import.path, e);
                    }
                }
            }
        }
    }

    all_configs
}

/// Load Handlebars/Mustache variables from imports with namespace prefixes
fn load_imported_handlebars_variables(
    program: &Program,
    file_path: Option<&PathBuf>,
) -> Vec<crate::ast::Variable> {
    use tracing::warn;

    let mut all_variables: Vec<crate::ast::Variable> = Vec::new();

    if let Some(file_path) = file_path {
        if !program.imports.is_empty() {
            let mut loader = crate::loader::ModuleLoader::new();

            for import in &program.imports {
                match loader.resolve_import_path(file_path, &import.path) {
                    Ok(resolved_path) => {
                        match loader.load_module(&resolved_path) {
                            Ok(imported_program) => {
                                // Collect Handlebars/Mustache variables with namespace prefix
                                for var in &imported_program.variables {
                                    if var.var_type == "handlebars" || var.var_type == "mustache" {
                                        // Register with namespaced name using / (Handlebars syntax)
                                        all_variables.push(crate::ast::Variable {
                                            var_type: var.var_type.clone(),
                                            name: format!("{}/{}", import.alias, var.name),
                                            value: var.value.clone(),
                                        });
                                    }
                                }
                            },
                            Err(e) => {
                                warn!("Failed to load imported module '{}' from '{}': {}",
                                    import.alias, import.path, e);
                            }
                        }
                    },
                    Err(e) => {
                        warn!("Failed to resolve import '{}' from '{}': {}",
                            import.alias, import.path, e);
                    }
                }
            }
        }
    }

    all_variables
}

/// Load imports and merge GraphQL schemas, routes, and pipelines from imported modules
fn merge_imported_graphql(
    program: &Program,
    file_path: Option<&PathBuf>,
) -> Program {
    let mut merged_program = program.clone();
    let mut merged_schema_parts: Vec<String> = Vec::new();

    // Add main program's schema if it exists
    if let Some(ref schema) = program.graphql_schema {
        merged_schema_parts.push(schema.sdl.clone());
    }

    if let Some(file_path) = file_path {
        if !program.imports.is_empty() {
            let mut loader = crate::loader::ModuleLoader::new();

            for import in &program.imports {
                match loader.resolve_import_path(file_path, &import.path) {
                    Ok(resolved_path) => {
                        match loader.load_module(&resolved_path) {
                            Ok(imported_program) => {
                                // Merge GraphQL schema
                                if let Some(ref imported_schema) = imported_program.graphql_schema {
                                    merged_schema_parts.push(imported_schema.sdl.clone());
                                }

                                // Merge query resolvers
                                merged_program.queries.extend(imported_program.queries.clone());

                                // Merge mutation resolvers
                                merged_program.mutations.extend(imported_program.mutations.clone());

                                // Merge type resolvers
                                merged_program.resolvers.extend(imported_program.resolvers.clone());

                                // Merge routes from imports
                                merged_program.routes.extend(imported_program.routes.clone());

                                // Merge pipelines from imports
                                merged_program.pipelines.extend(imported_program.pipelines.clone());
                            },
                            Err(e) => {
                                warn!("Failed to load imported module '{}' from '{}': {}",
                                    import.alias, import.path, e);
                            }
                        }
                    },
                    Err(e) => {
                        warn!("Failed to resolve import '{}' from '{}': {}",
                            import.alias, import.path, e);
                    }
                }
            }
        }
    }

    // Combine all schema parts into a single schema
    if !merged_schema_parts.is_empty() {
        let combined_sdl = merged_schema_parts.join("\n\n");
        merged_program.graphql_schema = Some(crate::ast::GraphQLSchema {
            sdl: combined_sdl,
        });
    }

    merged_program
}

pub struct WebPipeServer {
    program: Program,
    file_path: Option<PathBuf>,  // Path to the main .wp file (for resolving imports)
    middleware_registry: Arc<MiddlewareRegistry>,
    ctx: Arc<Context>,
    trace_enabled: bool,
    #[cfg(feature = "debugger")]
    debugger: Option<std::sync::Arc<dyn crate::debugger::DebuggerHook>>,
}

#[derive(Clone)]
pub struct RoutePayload {
    pub pipeline: Arc<Pipeline>,
    pub needs_flags: bool,
}

#[derive(Clone)]
pub struct ServerState {
    get_router: Arc<matchit::Router<RoutePayload>>,
    post_router: Arc<matchit::Router<RoutePayload>>,
    put_router: Arc<matchit::Router<RoutePayload>>,
    delete_router: Arc<matchit::Router<RoutePayload>>,
    env: Arc<ExecutionEnv>,
    feature_flags: Option<Arc<Pipeline>>,
    trace_enabled: bool,
}

impl ServerState {
    // variable resolution now handled by shared executor

    pub async fn execute_pipeline<'a>(
        &'a self,
        pipeline: &'a Pipeline,
        input: Value,
        ctx: crate::executor::RequestContext,
    ) -> Result<(Value, String, Option<u16>, crate::executor::RequestContext), crate::error::WebPipeError> {
        crate::executor::execute_pipeline(&self.env, pipeline, input, ctx).await
    }

    /// Create a new RequestContext for this request
    /// SECURITY: Feature flags are extracted from the feature_flags pipeline result,
    /// NOT from user-provided JSON input (which would be a security vulnerability)
    fn create_request_context(&self, flags: HashMap<String, bool>) -> crate::executor::RequestContext {
        #[cfg(feature = "debugger")]
        let debug_thread_id = if let Some(ref debugger) = self.env.debugger {
            Some(debugger.allocate_thread_id())
        } else {
            None
        };

        crate::executor::RequestContext {
            feature_flags: flags,
            conditions: HashMap::new(),
            async_registry: crate::executor::AsyncTaskRegistry::new(),
            deferred: Vec::new(),
            metadata: crate::executor::RequestMetadata::default(),
            call_log: HashMap::new(),
            profiler: crate::executor::Profiler::default(),
            #[cfg(feature = "debugger")]
            debug_thread_id,
            #[cfg(feature = "debugger")]
            debug_step_mode: None,
        }
    }

}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct WebPipeRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub body: Value,
    pub content_type: String,
}

impl WebPipeServer {
    #[cfg(feature = "debugger")]
    pub async fn from_program_with_debugger(
        program: Program,
        file_path: Option<PathBuf>,
        trace_enabled: bool,
        debugger: Option<std::sync::Arc<dyn crate::debugger::DebuggerHook>>,
    ) -> Result<Self, WebPipeError> {
        Self::from_program_internal(program, file_path, trace_enabled, debugger).await
    }

    pub async fn from_program(program: Program, file_path: Option<PathBuf>, trace_enabled: bool) -> Result<Self, WebPipeError> {
        Self::from_program_internal(program, file_path, trace_enabled, None).await
    }

    async fn from_program_internal(
        program: Program,
        file_path: Option<PathBuf>,
        trace_enabled: bool,
        #[cfg(feature = "debugger")]
        debugger: Option<std::sync::Arc<dyn crate::debugger::DebuggerHook>>,
        #[cfg(not(feature = "debugger"))]
        _debugger: Option<()>,
    ) -> Result<Self, WebPipeError> {
        // 1. Load imported configs and merge with main program configs
        let imported_configs = load_imported_configs(&program, file_path.as_ref());
        let mut all_configs = imported_configs;
        all_configs.extend(program.configs.clone());

        // 2. Load imported Handlebars/Mustache variables with namespace prefixes
        let imported_hb_vars = load_imported_handlebars_variables(&program, file_path.as_ref());
        let mut all_variables = imported_hb_vars;
        all_variables.extend(program.variables.clone());

        // 3. Merge imported GraphQL schemas and resolvers
        let merged_program = merge_imported_graphql(&program, file_path.as_ref());

        // 4. Build GraphQL runtime first (if schema exists)
        let graphql_runtime = if merged_program.graphql_schema.is_some() {
            Some(Arc::new(
                crate::graphql::GraphQLRuntime::from_program(&merged_program)
                    .map_err(|e| WebPipeError::ConfigError(format!("GraphQL initialization failed: {}", e)))?
            ))
        } else {
            None
        };

        // 5. Build a Context from merged configs and variables (also initializes global config)
        let mut ctx = Context::from_program_configs(all_configs, &all_variables).await?;

        // 6. Add GraphQL runtime to context
        ctx.graphql = graphql_runtime;

        // 7. Build middleware registry with context
        let ctx_arc = Arc::new(ctx);
        let middleware_registry = Arc::new(MiddlewareRegistry::with_builtins(ctx_arc.clone()));

        // 8. Use merged_program (includes imported routes, pipelines, and GraphQL)
        Ok(Self {
            program: merged_program,
            file_path,
            middleware_registry,
            ctx: ctx_arc,
            trace_enabled,
            #[cfg(feature = "debugger")]
            debugger,
        })
    }

    pub fn router(self) -> Router {
        let mut router = Router::new().route("/health", get(health_check));

        // Build named_pipelines and variables_map with import support
        // Key: (namespace, name) where namespace is None for local symbols
        let mut named_pipelines: HashMap<(Option<String>, String), Arc<Pipeline>> = HashMap::new();
        let mut variables_map: HashMap<(Option<String>, String, String), crate::ast::Variable> = HashMap::new();
        let mut imports_map: HashMap<String, PathBuf> = HashMap::new();

        // Register local symbols first
        for p in &self.program.pipelines {
            named_pipelines.insert((None, p.name.clone()), Arc::new(p.pipeline.clone()));
        }

        for v in &self.program.variables {
            variables_map.insert((None, v.var_type.clone(), v.name.clone()), v.clone());
        }

        // Add GraphQL query resolvers to named_pipelines
        for query in &self.program.queries {
            named_pipelines.insert(
                (None, format!("query::{}", query.name)),
                Arc::new(query.pipeline.clone())
            );
        }

        // Add GraphQL mutation resolvers to named_pipelines
        for mutation in &self.program.mutations {
            named_pipelines.insert(
                (None, format!("mutation::{}", mutation.name)),
                Arc::new(mutation.pipeline.clone())
            );
        }

        // Load and register imported symbols if file_path is available
        if let Some(ref file_path) = self.file_path {
            if !self.program.imports.is_empty() {
                let mut loader = crate::loader::ModuleLoader::new();

                // Build import map and load each imported file
                for import in &self.program.imports {
                    match loader.resolve_import_path(file_path, &import.path) {
                        Ok(resolved_path) => {
                            imports_map.insert(import.alias.clone(), resolved_path.clone());

                            // Load the imported program
                            match loader.load_module(&resolved_path) {
                                Ok(imported_program) => {
                                    let imported_file_path = resolved_path.to_string_lossy().to_string();

                                    // Register imported pipelines with namespace = Some(alias)
                                    for p in &imported_program.pipelines {
                                        let mut pipeline = p.pipeline.clone();
                                        // Update all SourceLocations in the pipeline to include the imported file path
                                        set_pipeline_file_path(&mut pipeline, &imported_file_path);
                                        named_pipelines.insert(
                                            (Some(import.alias.clone()), p.name.clone()),
                                            Arc::new(pipeline)
                                        );
                                    }

                                    // Register imported variables with namespace = Some(alias)
                                    for v in &imported_program.variables {
                                        variables_map.insert(
                                            (Some(import.alias.clone()), v.var_type.clone(), v.name.clone()),
                                            v.clone()
                                        );
                                    }

                                    // Register imported GraphQL query resolvers
                                    for query in &imported_program.queries {
                                        let mut pipeline = query.pipeline.clone();
                                        set_pipeline_file_path(&mut pipeline, &imported_file_path);
                                        named_pipelines.insert(
                                            (None, format!("query::{}", query.name)),
                                            Arc::new(pipeline)
                                        );
                                    }

                                    // Register imported GraphQL mutation resolvers
                                    for mutation in &imported_program.mutations {
                                        let mut pipeline = mutation.pipeline.clone();
                                        set_pipeline_file_path(&mut pipeline, &imported_file_path);
                                        named_pipelines.insert(
                                            (None, format!("mutation::{}", mutation.name)),
                                            Arc::new(pipeline)
                                        );
                                    }
                                },
                                Err(e) => {
                                    warn!("Failed to load imported module '{}' from '{}': {}",
                                        import.alias, import.path, e);
                                }
                            }
                        },
                        Err(e) => {
                            warn!("Failed to resolve import '{}' from '{}': {}",
                                import.alias, import.path, e);
                        }
                    }
                }
            }
        }

        // Build method-specific matchers with RoutePayload
        let mut get_router = matchit::Router::new();
        let mut post_router = matchit::Router::new();
        let mut put_router = matchit::Router::new();
        let mut delete_router = matchit::Router::new();

        for route in &self.program.routes {
            let path = route.path.clone();
            let pipeline = match &route.pipeline {
                PipelineRef::Inline(p) => Arc::new(p.clone()),
                PipelineRef::Named(name) => {
                    let p = self.program
                        .pipelines
                        .iter()
                        .find(|pl| pl.name == *name)
                        .expect("pipeline not found")
                        .pipeline
                        .clone();
                    Arc::new(p)
                }
            };

            // Static analysis: determine if this route needs flags
            let needs_flags = pipeline_needs_flags(&pipeline, &named_pipelines);

            let payload = RoutePayload {
                pipeline: pipeline.clone(),
                needs_flags,
            };

            match route.method.as_str() {
                "GET" => {
                    let _ = get_router.insert(path.clone(), payload.clone());
                }
                "POST" => {
                    let _ = post_router.insert(path.clone(), payload.clone());
                }
                "PUT" => {
                    let _ = put_router.insert(path.clone(), payload.clone());
                }
                "DELETE" => {
                    let _ = delete_router.insert(path.clone(), payload.clone());
                }
                other => {
                    warn!("Unsupported HTTP method: {}", other);
                    continue;
                }
            }
        }

        // Register automatic GraphQL endpoint if configured
        if let Some(endpoint) = crate::config::global().get_graphql_endpoint() {
            // Only register if we have a GraphQL runtime (includes imported schemas)
            if self.ctx.graphql.is_some() {
                // Check if user has already defined a POST route for this endpoint
                let user_defined_route = self.program.routes.iter().any(|route| {
                    route.method == "POST" && route.path == endpoint
                });

                if user_defined_route {
                    info!("User-defined POST {} route found, skipping automatic GraphQL endpoint registration", endpoint);
                } else {
                    let graphql_pipeline = Arc::new(create_graphql_endpoint_pipeline());

                    // Check if pipeline needs flags (should be false for our simple pipeline)
                    let needs_flags = pipeline_needs_flags(&graphql_pipeline, &named_pipelines);

                    let payload = RoutePayload {
                        pipeline: graphql_pipeline,
                        needs_flags,
                    };

                    // Register POST route
                    if let Err(e) = post_router.insert(endpoint.clone(), payload) {
                        warn!("Failed to register GraphQL endpoint at POST {}: {}", endpoint, e);
                    } else {
                        info!("Registered automatic GraphQL endpoint at POST {}", endpoint);
                    }
                }
            } else {
                warn!(
                    "GraphQL endpoint configured at {} but no GraphQL schema defined",
                    endpoint
                );
            }
        }

        // Build ExecutionEnv with loaded symbols and imports
        let env = Arc::new(ExecutionEnv {
            variables: Arc::new(variables_map),
            named_pipelines: Arc::new(named_pipelines),
            imports: Arc::new(imports_map),
            invoker: Arc::new(RealInvoker::new(self.middleware_registry.clone())),
            registry: self.middleware_registry.clone(),
            environment: std::env::var("WEBPIPE_ENV").ok(),
            cache: self.ctx.cache.clone(),
            rate_limit: self.ctx.rate_limit.clone(),
            #[cfg(feature = "debugger")]
            debugger: self.debugger.clone(),
        });

        // Set the execution environment in the Context so GraphQL middleware can access it
        *self.ctx.execution_env.write() = Some(env.clone());

        let server_state = ServerState {
            get_router: Arc::new(get_router),
            post_router: Arc::new(post_router),
            put_router: Arc::new(put_router),
            delete_router: Arc::new(delete_router),
            env: env.clone(),
            feature_flags: self.program.feature_flags.as_ref().map(|p| Arc::new(p.clone())),
            trace_enabled: self.trace_enabled,
        };

        // Single catch-all per method
        router = router
            .route("/", on(MethodFilter::GET, unified_handler))
            .route("/*path", on(MethodFilter::GET, unified_handler))
            .route("/*path", on(MethodFilter::POST, unified_handler))
            .route("/*path", on(MethodFilter::PUT, unified_handler))
            .route("/*path", on(MethodFilter::DELETE, unified_handler));

        router
            .layer(
                ServiceBuilder::new()
                    .layer(CorsLayer::permissive())
                    .into_inner(),
            )
            .with_state(server_state)
    }

    pub async fn serve(self, addr: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        
        info!("Web Pipe server starting on {}", addr);
        
        axum::serve(listener, self.router().into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown_signal())
            .await?;
            
        info!("Web Pipe server shutdown complete");
        Ok(())
    }

    pub async fn serve_with_shutdown<Sh>(
        self,
        addr: SocketAddr,
        shutdown: Sh,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        Sh: std::future::Future<Output = ()> + Send + 'static,
    {
        let listener = tokio::net::TcpListener::bind(addr).await?;

        info!("Web Pipe server starting on {}", addr);

        axum::serve(listener, self.router().into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown)
            .await?;

        info!("Web Pipe server shutdown complete");
        Ok(())
    }
}

// configure_pg_from_config removed: middleware reads config directly

async fn unified_handler(
    State(state): State<ServerState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    method: Method,
    headers: HeaderMap,
    OriginalUri(orig_uri): OriginalUri,
    Query(query_params): Query<HashMap<String, String>>,
    body: Bytes,
) -> impl IntoResponse {
    // Select router based on method
    let path = orig_uri.path().to_string();
    
    // Extract remote IP from connection (if available)
    let remote_ip = connect_info.map(|ConnectInfo(addr)| addr.ip().to_string());

    // Try static first on GET so dynamic catch-alls don't shadow assets
    if method == Method::GET {
        if let Some(resp) = try_serve_static(&path).await { return resp; }
    }
    let (payload, path_params) = match method.as_str() {
        "GET" => match state.get_router.at(&path) {
            Ok(m) => (m.value.clone(), m.params.iter().map(|(k,v)| (k.to_string(), v.to_string())).collect::<HashMap<_,_>>()),
            Err(_) => {
                if let Some(resp) = try_serve_static(&path).await { return resp; }
                return StatusCode::NOT_FOUND.into_response();
            },
        },
        "POST" => match state.post_router.at(&path) {
            Ok(m) => (m.value.clone(), m.params.iter().map(|(k,v)| (k.to_string(), v.to_string())).collect::<HashMap<_,_>>()),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        "PUT" => match state.put_router.at(&path) {
            Ok(m) => (m.value.clone(), m.params.iter().map(|(k,v)| (k.to_string(), v.to_string())).collect::<HashMap<_,_>>()),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        "DELETE" => match state.delete_router.at(&path) {
            Ok(m) => (m.value.clone(), m.params.iter().map(|(k,v)| (k.to_string(), v.to_string())).collect::<HashMap<_,_>>()),
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        },
        _ => return StatusCode::METHOD_NOT_ALLOWED.into_response(),
    };

    respond_with_pipeline(state, method, headers, path, path_params, query_params, body, payload, remote_ip.as_deref()).await
}

async fn respond_with_pipeline(
    state: ServerState,
    method: Method,
    headers: HeaderMap,
    path: String,
    path_params: HashMap<String, String>,
    query_params: HashMap<String, String>,
    body: Bytes,
    payload: RoutePayload,
    remote_ip: Option<&str>,
) -> Response {
    // Build unified request JSON and content type via shared helper
    let (mut request_json, _content_type) = build_request_from_axum_with_ip(&method, &headers, &path, &path_params, &query_params, &body, remote_ip);

    // --- PRE-FLIGHT CHECK: Execute feature flags pipeline if needed ---
    let mut flags = HashMap::new();
    if payload.needs_flags {
        if let Some(flag_pipeline) = &state.feature_flags {
            // Execute Flag Pipeline with Timeout (50ms)
            // Create a temporary RequestContext for the flag pipeline
            let flag_ctx = state.create_request_context(HashMap::new());
            let flag_result = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                state.execute_pipeline(flag_pipeline, request_json.clone(), flag_ctx)
            ).await;

            match flag_result {
                Ok(Ok((result_json, _, _, flag_ctx))) => {
                    // 1. Extract flags from the Context (Primary source)
                    flags.extend(flag_ctx.feature_flags);

                    // Merge user (if auth happened in pre-flight)
                    if let Some(user) = result_json.get("user") {
                        if let Some(obj) = request_json.as_object_mut() {
                            obj.insert("user".to_string(), user.clone());
                        }
                    }
                },
                Ok(Err(e)) => {
                    warn!("Flag resolution error: {}", e);
                },
                Err(_) => {
                    warn!("Flag resolution timed out");
                }
            }
        }
    }

    // Create RequestContext with feature flags
    let mut ctx = state.create_request_context(flags);

    // Push the route as the root of the profiler stack
    let route_label = format!("{} {}", method, path);
    ctx.profiler.push(&route_label);
    let route_start = std::time::Instant::now();

    // Execute the pipeline with the RequestContext
    match crate::executor::execute_pipeline(&state.env, &payload.pipeline, request_json.clone(), ctx).await {
        Ok((result, content_type, status_code, mut ctx)) => {
            // Record route timing and pop from stack
            let route_elapsed = route_start.elapsed().as_micros();
            ctx.profiler.record_sample(route_elapsed);
            ctx.profiler.pop();

            // --- OUTPUT TRACE DATA ---
            if state.trace_enabled && !ctx.profiler.samples.is_empty() {
                println!("\n# speedscope trace start");
                for (stack, micros) in &ctx.profiler.samples {
                    println!("{} {}", stack, micros);
                }
                println!("# speedscope trace end\n");
            }
            // -------------------------

            // Run all deferred actions (e.g. caching, logging) with the clean result
            ctx.run_deferred(&result, &content_type, &state.env);

            // Determine the HTTP status code
            let http_status = if let Some(code) = status_code {
                StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
            } else {
                StatusCode::OK
            };
            
            // Create the base response
            let mut response = if content_type.starts_with("text/html") {
                match result.as_str() {
                    Some(html) => (http_status, Html(html.to_string())).into_response(),
                    None => {
                        // If not a string but should be HTML, try to serialize as JSON
                        match serde_json::to_string(&result) {
                            Ok(json_str) => (http_status, Html(json_str)).into_response(),
                            Err(_) => (http_status, Html("".to_string())).into_response()
                        }
                    }
                }
            } else {
                (http_status, Json(result.clone())).into_response()
            };

            // Apply custom headers from setHeaders field
            use axum::http::header::{HeaderName, HeaderValue};
            use axum::http::header::SET_COOKIE;

            let headers = response.headers_mut();

            // If setCookies present, include Set-Cookie headers
            if let Some(cookies) = result.get("setCookies").and_then(|v| v.as_array()) {
                for cookie in cookies.iter().filter_map(|v| v.as_str()) {
                    if let Ok(val) = HeaderValue::from_str(cookie) {
                        headers.append(SET_COOKIE, val);
                    }
                }
            }

            // If setHeaders present, include custom headers
            if let Some(custom_headers) = result.get("setHeaders").and_then(|v| v.as_object()) {
                for (header_name, header_value) in custom_headers {
                    if let Some(value_str) = header_value.as_str() {
                        if let (Ok(name), Ok(val)) = (
                            HeaderName::from_bytes(header_name.as_bytes()),
                            HeaderValue::from_str(value_str)
                        ) {
                            headers.insert(name, val);
                        }
                    }
                }
            }

            // Note: post_execute hooks are no longer needed - logging uses ctx.defer()
            // and rate limit info is in ctx.metadata.rate_limit_status
            response
        }
        Err(e) => {
            // Use debug logging for rate limits (expected during testing), warn for actual errors
            match &e {
                crate::error::WebPipeError::RateLimitExceeded(_) => {
                    debug!("Rate limit exceeded: {}", e);
                }
                _ => {
                    warn!("Pipeline execution failed: {}", e);
                }
            }
            let error_response = serde_json::json!({
                "error": "Pipeline execution failed",
                "details": e.to_string()
            });
            (StatusCode::INTERNAL_SERVER_ERROR, Json(error_response)).into_response()
        }
    }
}


async fn health_check() -> impl IntoResponse {
    let mut status = HashMap::new();
    status.insert("status", "ok");
    status.insert("version", env!("CARGO_PKG_VERSION"));
    
    (StatusCode::OK, serde_json::to_string(&status).unwrap())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received, starting graceful shutdown");

    // Spawn a task that forces the process to exit if graceful shutdown takes too long
    // This prevents the server from hanging on open connections (e.g. keep-alive)
    // and holding onto the ports, which blocks subsequent debug sessions.
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        warn!("Graceful shutdown timed out (2s), forcing exit to release ports");
        std::process::exit(0);
    });
}

// --- Static file serving (public/ directory) ---

fn public_dir_path() -> PathBuf {
    std::env::var("WEBPIPE_PUBLIC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("public"))
}

fn normalize_static_path(request_path: &str) -> Option<PathBuf> {
    use std::path::Component;
    use percent_encoding::percent_decode_str;

    if !request_path.starts_with('/') { return None; }

    // Decode URL encoding to catch encoded traversal attempts like %2e%2e
    let decoded = match percent_decode_str(request_path).decode_utf8() {
        Ok(s) => s.into_owned(),
        Err(_) => return None,
    };

    let mut rel = decoded.trim_start_matches('/');
    if rel.is_empty() { rel = "index.html"; }

    // Parse the path and ensure no parent directory components exist
    let path = Path::new(rel);
    for component in path.components() {
        match component {
            Component::Normal(_) => continue,
            Component::RootDir | Component::CurDir => continue,
            Component::ParentDir => return None, // Block any .. components
            Component::Prefix(_) => return None,  // Block Windows drive prefixes
        }
    }

    // Build the full path and canonicalize to resolve any remaining tricks
    let public_dir = public_dir_path();
    let full = public_dir.join(rel);

    // Canonicalize both paths to resolve symlinks and relative components
    // This is the final defense - ensure the resolved path is still under public_dir
    match (full.canonicalize(), public_dir.canonicalize()) {
        (Ok(canonical_full), Ok(canonical_public)) => {
            if canonical_full.starts_with(&canonical_public) {
                Some(full)
            } else {
                None
            }
        }
        _ => {
            // If canonicalization fails (file doesn't exist yet), do a prefix check on the non-canonical paths
            // This allows serving of files that exist while still protecting against traversal
            if full.starts_with(&public_dir) {
                Some(full)
            } else {
                None
            }
        }
    }
}

fn guess_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        _ => "application/octet-stream",
    }
}

async fn try_serve_static(request_path: &str) -> Option<Response> {
    let mut candidate = normalize_static_path(request_path)?;

    if request_path.ends_with('/') {
        candidate.push("index.html");
    }

    if !candidate.exists() && Path::new(request_path).extension().is_none() {
        let mut idx = candidate.clone();
        idx.push("index.html");
        if idx.exists() { candidate = idx; }
    }

    let data = match tokio_fs::read(&candidate).await {
        Ok(bytes) => bytes,
        Err(_) => return None,
    };

    let ct = guess_content_type(&candidate);
    let mut resp = Response::new(Body::from(data));
    let _ = resp.headers_mut().insert(axum::http::header::CONTENT_TYPE, axum::http::HeaderValue::from_static(ct));
    Some(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use axum::http::HeaderValue;
    use std::collections::HashMap;

    #[test]
    fn content_type_guessing() {
        assert_eq!(guess_content_type(Path::new("a.html")), "text/html; charset=utf-8");
        assert_eq!(guess_content_type(Path::new("a.css")), "text/css; charset=utf-8");
        assert_eq!(guess_content_type(Path::new("a.json")), "application/json; charset=utf-8");
        assert_eq!(guess_content_type(Path::new("a.bin")), "application/octet-stream");
    }

    #[test]
    fn static_path_normalization() {
        std::env::set_var("WEBPIPE_PUBLIC_DIR", "public");
        // normal
        let p = normalize_static_path("/index.html").unwrap();
        assert!(p.ends_with("index.html"));
        // traversal guarded
        assert!(normalize_static_path("/../secret").is_none());
        // root -> index.html
        let p2 = normalize_static_path("/").unwrap();
        assert!(p2.ends_with("index.html"));
    }

    #[test]
    fn path_traversal_attacks_blocked() {
        std::env::set_var("WEBPIPE_PUBLIC_DIR", "public");

        // Classic path traversal
        assert!(normalize_static_path("/../../../etc/passwd").is_none());
        assert!(normalize_static_path("/../../secret").is_none());
        assert!(normalize_static_path("/foo/../../bar").is_none());

        // URL-encoded traversal attempts
        assert!(normalize_static_path("/%2e%2e/etc/passwd").is_none());
        assert!(normalize_static_path("/%2e%2e%2f%2e%2e%2fetc%2fpasswd").is_none());
        assert!(normalize_static_path("/foo/%2e%2e/%2e%2e/secret").is_none());

        // Mixed encoding
        assert!(normalize_static_path("/foo/..%2f..%2fsecret").is_none());
        assert!(normalize_static_path("/%2e%2e/../etc/passwd").is_none());

        // Double-encoded attempts (decodes to %2e%2e which is not a valid filename, so it will return Some but the file won't exist)
        // This is acceptable since the path still doesn't escape the public directory

        // On Windows, backslashes are path separators and should be blocked in combination with ..
        // On Unix, backslash is just a regular filename character, so /..\ is a directory named ".."
        #[cfg(target_os = "windows")]
        {
            assert!(normalize_static_path("/..\\..\\secret").is_none());
        }

        // Null byte injection (caught by UTF-8 validation)
        assert!(normalize_static_path("/../secret%00.txt").is_none() ||
                normalize_static_path("/../secret%00.txt").unwrap().to_str().unwrap().contains("secret%00"));

        // Valid paths should still work
        assert!(normalize_static_path("/index.html").is_some());
        assert!(normalize_static_path("/assets/style.css").is_some());
        assert!(normalize_static_path("/js/app.js").is_some());
        assert!(normalize_static_path("/").is_some());
    }

    #[test]
    fn path_normalization_with_special_chars() {
        std::env::set_var("WEBPIPE_PUBLIC_DIR", "public");

        // URL-encoded normal characters should work
        assert!(normalize_static_path("/file%20name.html").is_some());
        assert!(normalize_static_path("/foo%2Fbar.txt").is_some());

        // Current directory references should be handled
        assert!(normalize_static_path("/./file.html").is_some());
        assert!(normalize_static_path("/foo/./bar.txt").is_some());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_drive_prefixes_blocked() {
        std::env::set_var("WEBPIPE_PUBLIC_DIR", "public");

        // Windows absolute paths and drive letters should be blocked
        assert!(normalize_static_path("/C:/Windows/System32/config/sam").is_none());
        assert!(normalize_static_path("/C:\\Windows\\System32").is_none());
    }

    #[tokio::test]
    async fn health_check_ok() {
        let resp = health_check().await.into_response();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn try_serve_static_serves_file_and_none_for_missing() {
        let base = std::env::temp_dir().join(format!("wp_public_{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&base).await.unwrap();
        std::env::set_var("WEBPIPE_PUBLIC_DIR", &base);
        let file_path = base.join("hello.txt");
        tokio_fs::write(&file_path, b"hi").await.unwrap();
        // direct file
        let resp_opt = try_serve_static("/hello.txt").await;
        assert!(resp_opt.is_some());
        // missing -> None
        let none = try_serve_static("/nope.txt").await;
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn respond_with_pipeline_sets_cookies_and_html() {
        // Build a minimal state with jq/handlebars available
        let ctx = Context::from_program_configs(vec![], &[]).await.unwrap();
        let registry = Arc::new(MiddlewareRegistry::with_builtins(Arc::new(ctx.clone())));
        let env = ExecutionEnv {
            variables: Arc::new(HashMap::new()),
            named_pipelines: Arc::new(HashMap::new()),
            imports: Arc::new(std::collections::HashMap::new()),
            invoker: Arc::new(RealInvoker::new(registry.clone())),
            registry: registry.clone(),
            environment: None,
            cache: ctx.cache.clone(),
            rate_limit: ctx.rate_limit.clone(),
            #[cfg(feature = "debugger")]
            debugger: None,
        };
        let state = ServerState {
            get_router: Arc::new(matchit::Router::new()),
            post_router: Arc::new(matchit::Router::new()),
            put_router: Arc::new(matchit::Router::new()),
            delete_router: Arc::new(matchit::Router::new()),
            env: Arc::new(env),
            feature_flags: None,
            trace_enabled: false,
        };
        // Craft a tiny pipeline that sets cookies via jq
        let p_set_cookie = Arc::new(Pipeline { steps: vec![
            crate::ast::PipelineStep::Regular { name: "jq".to_string(), args: Vec::new(), config: "{ setCookies: [\"a=b\"] }".to_string(), config_type: crate::ast::ConfigType::Quoted, condition: None, parsed_join_targets: None, location: crate::ast::SourceLocation { line: 0, column: 0, offset: 0, file_path: None } }
        ]});
        let resp = respond_with_pipeline(
            state.clone(),
            Method::GET,
            HeaderMap::new(),
            "/test".to_string(),
            HashMap::new(),
            HashMap::new(),
            axum::body::Bytes::new(),
            RoutePayload { pipeline: p_set_cookie.clone(), needs_flags: false },
            Some("127.0.0.1"),
        ).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // Header present
        assert!(resp.headers().get_all(axum::http::header::SET_COOKIE).iter().any(|h| h == &HeaderValue::from_static("a=b")));

        // Pipeline that renders HTML
        let p_html = Arc::new(Pipeline { steps: vec![
            crate::ast::PipelineStep::Regular { name: "handlebars".to_string(), args: Vec::new(), config: "<p>OK</p>".to_string(), config_type: crate::ast::ConfigType::Quoted, condition: None, parsed_join_targets: None, location: crate::ast::SourceLocation { line: 0, column: 0, offset: 0, file_path: None } }
        ]});
        let resp2 = respond_with_pipeline(
            state,
            Method::GET,
            HeaderMap::new(),
            "/test".to_string(),
            HashMap::new(),
            HashMap::new(),
            axum::body::Bytes::new(),
            RoutePayload { pipeline: p_html.clone(), needs_flags: false },
            Some("127.0.0.1"),
        ).await;
        assert_eq!(resp2.status(), StatusCode::OK);
        assert_eq!(resp2.headers().get(axum::http::header::CONTENT_TYPE).unwrap(), "text/html; charset=utf-8");
    }
}