/// DAP (Debug Adapter Protocol) Server Implementation
///
/// This module implements a DAP server that:
/// - Listens on a TCP port for VS Code connections
/// - Manages breakpoints across HTTP requests
/// - Pauses pipeline execution when breakpoints are hit
/// - Provides variable introspection via DAP protocol
/// - Supports stepOver command for step-through debugging

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicI64, Ordering};
use dashmap::DashMap;
use tokio::sync::{oneshot, Mutex, Notify};
use tokio::net::TcpListener;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use serde_json::{Value as JsonValue, json};
use async_trait::async_trait;

use crate::ast::SourceLocation;
use crate::error::WebPipeError;
use super::hook::{DebuggerHook, StepAction};
use super::protocol::*;

/// Command to send to a paused thread
#[derive(Debug, Clone)]
pub enum StepCommand {
    /// Continue execution normally, optionally updating state
    Continue(Option<serde_json::Value>),
    /// Step over: pause at next step in same pipeline
    StepOver(Option<serde_json::Value>),
    /// Step in: pause at next step (entering nested pipelines)
    StepIn(Option<serde_json::Value>),
    /// Step out: pause when returning to parent pipeline
    StepOut(Option<serde_json::Value>),
}

/// Stepping mode for a running thread
#[derive(Debug, Clone, Copy)]
pub enum SteppingMode {
    /// Pause at next instruction
    StepInto,
    /// Pause at next instruction where stack_depth <= target_depth
    StepOver { target_depth: usize },
    /// Pause at next instruction where stack_depth < target_depth
    StepOut { target_depth: usize },
}

/// State of a paused thread (HTTP request)
pub struct PausedState {
    /// Channel to resume execution
    pub resume_tx: oneshot::Sender<StepCommand>,

    /// Snapshot of pipeline state at breakpoint
    pub state: serde_json::Value,

    /// Location where execution paused
    pub location: SourceLocation,

    /// Name of the step that paused
    pub step_name: String,

    /// Timestamp when paused (for debugging)
    pub paused_at: std::time::Instant,

    /// Full execution stack (e.g., ["GET /api", "pipeline:auth", "pg"])
    pub stack: Vec<String>,

    /// Counter for generating unique variable references for this thread
    pub var_ref_index: i64,
}

/// DAP Server - manages debugging sessions
pub struct DapServer {
    /// Breakpoints: file_path -> Vec<line_numbers>
    breakpoints: Arc<DashMap<String, Vec<usize>>>,

    /// Breakpoint IDs: file_path -> (line -> breakpoint_id)
    breakpoint_ids: Arc<DashMap<String, std::collections::HashMap<usize, i64>>>,

    /// Next breakpoint ID
    next_breakpoint_id: Arc<AtomicI64>,

    /// Paused threads: thread_id -> PausedState
    paused_threads: Arc<DashMap<u64, PausedState>>,

    /// Threads in stepping mode (should pause at next step)
    stepping_threads: Arc<DashMap<u64, SteppingMode>>,

    /// Threads we've announced to VS Code (to avoid duplicate thread events)
    announced_threads: Arc<dashmap::DashSet<u64>>,

    /// Next thread ID (incremented for each HTTP request)
    next_thread_id: Arc<AtomicU64>,

    /// Variable reference cache: var_ref -> JSON Value
    /// This enables expanding nested objects/arrays in VS Code Variables panel
    variable_cache: Arc<DashMap<i64, serde_json::Value>>,

    /// Maps variable reference -> JSON Pointer string (e.g. "/data/users/0")
    variable_pointers: Arc<DashMap<i64, String>>,

    /// File path being debugged
    file_path: String,

    /// DAP TCP port (default: 5858)
    debug_port: u16,

    /// DAP client writer (VS Code connection)
    /// We split the stream so reading doesn't block writing
    client_writer: Arc<Mutex<Option<OwnedWriteHalf>>>,

    /// Sequence number for DAP messages
    seq: Arc<AtomicI64>,

    /// Notifier to signal when the debug session ends (client disconnects)
    shutdown_notify: Option<Arc<Notify>>,
}

/// Stride for variable references to avoid collision between threads
/// Thread 1: 100,000 - 199,999
/// Thread 2: 200,000 - 299,999
const VAR_REF_STRIDE: i64 = 100_000;

impl DapServer {
    /// Create a new DAP server
    pub fn new(file_path: String, debug_port: u16, shutdown_notify: Option<Arc<Notify>>) -> Self {
        Self {
            breakpoints: Arc::new(DashMap::new()),
            breakpoint_ids: Arc::new(DashMap::new()),
            next_breakpoint_id: Arc::new(AtomicI64::new(1)),
            paused_threads: Arc::new(DashMap::new()),
            stepping_threads: Arc::new(DashMap::new()),
            announced_threads: Arc::new(dashmap::DashSet::new()),
            next_thread_id: Arc::new(AtomicU64::new(1)),
            variable_cache: Arc::new(DashMap::new()),
            variable_pointers: Arc::new(DashMap::new()),
            file_path,
            debug_port,
            client_writer: Arc::new(Mutex::new(None)),
            seq: Arc::new(AtomicI64::new(1)),
            shutdown_notify,
        }
    }

    /// Allocate a new thread ID for an incoming HTTP request
    pub fn allocate_thread_id(&self) -> u64 {
        self.next_thread_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Escape keys for JSON Pointer (RFC 6901)
    /// '~' becomes '~0' and '/' becomes '~1'
    fn escape_json_pointer(key: &str) -> String {
        key.replace('~', "~0").replace('/', "~1")
    }

    /// Clear variable cache for a thread (called when thread resumes)
    fn clear_thread_variables(&self, thread_id: u64) {
        // Variable references for a thread are in the range [thread_id * STRIDE, (thread_id + 1) * STRIDE - 1]
        let start = thread_id as i64 * VAR_REF_STRIDE;
        let end = start + VAR_REF_STRIDE - 1;

        // Remove all cached variables for this thread
        self.variable_cache.retain(|k, _v| *k < start || *k > end);

        // Clean up pointers too
        self.variable_pointers.retain(|k, _v| *k < start || *k > end);
    }

    /// Start TCP listener and wait for VS Code connection
    pub async fn start_tcp_listener(self: Arc<Self>) -> Result<(), WebPipeError> {
        let listener = TcpListener::bind(("127.0.0.1", self.debug_port))
            .await
            .map_err(|e| WebPipeError::DebuggerError(format!("Failed to bind to port {}: {}", self.debug_port, e)))?;

        eprintln!("[DAP] Listening on 127.0.0.1:{}", self.debug_port);

        // Accept connection from VS Code
        let (stream, addr) = listener.accept().await
            .map_err(|e| WebPipeError::DebuggerError(format!("Failed to accept connection: {}", e)))?;

        eprintln!("[DAP] Client connected from {}", addr);

        // Split stream into reader and writer
        // This prevents the read loop from locking the writer, which caused deadlocks
        let (reader, writer) = stream.into_split();

        *self.client_writer.lock().await = Some(writer);

        // Spawn task to handle DAP messages
        let server = self.clone();
        tokio::spawn(async move {
            // Clone the shutdown notifier *before* server is moved into handle_messages
            let shutdown_notify = server.shutdown_notify.clone();

            if let Err(e) = server.handle_messages(reader).await {
                eprintln!("[DAP] Message handler error: {}", e);
            }
            
            // Connection closed or error occurred - trigger shutdown
            if let Some(notify) = shutdown_notify {
                eprintln!("[DAP] Connection closed, triggering shutdown");
                notify.notify_one();
            }
        });

        Ok(())
    }

    /// Handle incoming DAP messages from VS Code
    async fn handle_messages(self: Arc<Self>, mut reader: OwnedReadHalf) -> Result<(), WebPipeError> {
        loop {
            let msg = self.read_message(&mut reader).await?;

            // Parse the message
            let request: Request = serde_json::from_str(&msg)
                .map_err(|e| WebPipeError::DebuggerError(format!("Failed to parse request: {}", e)))?;

            // Log all commands to see complete protocol flow
            eprintln!("[DAP] {} (seq: {})", request.command, request.seq);

            // Dispatch to handler
            match request.command.as_str() {
                "initialize" => self.handle_initialize(request).await?,
                "launch" => self.handle_launch(request).await?,
                "setBreakpoints" => self.handle_set_breakpoints(request).await?,
                "configurationDone" => self.handle_configuration_done(request).await?,
                "threads" => self.handle_threads(request).await?,
                "stackTrace" => self.handle_stack_trace(request).await?,
                "scopes" => self.handle_scopes(request).await?,
                "variables" => self.handle_variables(request).await?,
                "setVariable" => self.handle_set_variable(request).await?,
                "evaluate" => self.handle_evaluate(request).await?,
                "continue" => self.handle_continue(request).await?,
                "next" => self.handle_next(request).await?,
                "stepIn" => self.handle_step_in(request).await?,
                "stepOut" => self.handle_step_out(request).await?,
                "pause" => self.handle_pause(request).await?,
                "source" => self.handle_source(request).await?,
                "disconnect" => {
                    self.handle_disconnect(request).await?;
                    break;
                }
                _ => {
                    eprintln!("[DAP] Unhandled command: {}", request.command);
                    self.send_error_response(request.seq, &request.command, "Command not supported").await?;
                }
            }
        }

        Ok(())
    }

    /// Read a DAP message from the client (Content-Length header + JSON body)
    async fn read_message(&self, reader: &mut OwnedReadHalf) -> Result<String, WebPipeError> {
        // Read headers
        let mut headers = String::new();
        let mut buf = [0u8; 1];

        loop {
            reader.read_exact(&mut buf).await
                .map_err(|e| WebPipeError::DebuggerError(format!("Failed to read: {}", e)))?;
            headers.push(buf[0] as char);

            if headers.ends_with("\r\n\r\n") {
                break;
            }
        }

        // Parse Content-Length
        let content_length = headers
            .lines()
            .find(|line| line.starts_with("Content-Length:"))
            .and_then(|line| line.split(':').nth(1))
            .and_then(|s| s.trim().parse::<usize>().ok())
            .ok_or_else(|| WebPipeError::DebuggerError("Missing Content-Length header".to_string()))?;

        // Read body
        let mut body = vec![0u8; content_length];
        reader.read_exact(&mut body).await
            .map_err(|e| WebPipeError::DebuggerError(format!("Failed to read body: {}", e)))?;

        String::from_utf8(body)
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid UTF-8: {}", e)))
    }

    /// Send a DAP message to the client
    async fn send_message(&self, msg: &str) -> Result<(), WebPipeError> {
        let mut client_guard = self.client_writer.lock().await;
        let client = client_guard.as_mut()
            .ok_or_else(|| WebPipeError::DebuggerError("No client connected".to_string()))?;

        let header = format!("Content-Length: {}\r\n\r\n", msg.len());

        client.write_all(header.as_bytes()).await
            .map_err(|e| WebPipeError::DebuggerError(format!("Failed to write header: {}", e)))?;
        client.write_all(msg.as_bytes()).await
            .map_err(|e| WebPipeError::DebuggerError(format!("Failed to write body: {}", e)))?;
        client.flush().await
            .map_err(|e| WebPipeError::DebuggerError(format!("Failed to flush: {}", e)))?;

        Ok(())
    }

    /// Send a response
    async fn send_response(&self, request_seq: i64, command: &str, body: Option<JsonValue>) -> Result<(), WebPipeError> {
        let response = Response {
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            request_seq,
            success: true,
            command: command.to_string(),
            message: None,
            body,
        };

        let json = serde_json::to_string(&json!({
            "type": "response",
            "seq": response.seq,
            "request_seq": response.request_seq,
            "success": response.success,
            "command": response.command,
            "body": response.body,
        })).unwrap();

        // Response sent successfully
        self.send_message(&json).await
    }

    /// Send an error response
    async fn send_error_response(&self, request_seq: i64, command: &str, message: &str) -> Result<(), WebPipeError> {
        let response = Response {
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            request_seq,
            success: false,
            command: command.to_string(),
            message: Some(message.to_string()),
            body: None,
        };

        let json = serde_json::to_string(&json!({
            "type": "response",
            "seq": response.seq,
            "request_seq": response.request_seq,
            "success": response.success,
            "command": response.command,
            "message": response.message,
        })).unwrap();

        self.send_message(&json).await
    }

    /// Send an event
    async fn send_event(&self, event_name: &str, body: Option<JsonValue>) -> Result<(), WebPipeError> {
        let event = Event {
            seq: self.seq.fetch_add(1, Ordering::SeqCst),
            event: event_name.to_string(),
            body,
        };

        let json = serde_json::to_string(&json!({
            "type": "event",
            "seq": event.seq,
            "event": event.event,
            "body": event.body,
        })).unwrap();

        // Event sent
        self.send_message(&json).await
    }

    // ========================================================================
    // Request Handlers
    // ========================================================================

    async fn handle_initialize(&self, request: Request) -> Result<(), WebPipeError> {
        let capabilities = Capabilities {
            supports_configuration_done_request: Some(true),
            supports_function_breakpoints: Some(false),
            supports_conditional_breakpoints: Some(false),
            supports_hit_conditional_breakpoints: Some(false),
            supports_evaluate_for_hovers: Some(true),
            supports_step_back: Some(false),
            supports_set_variable: Some(true),
            supports_restart_frame: Some(false),
            supports_goto_targets_request: Some(false),
            supports_step_in_targets_request: Some(false),
            supports_completions_request: Some(false),
            supports_modules_request: Some(false),
            supports_restart_request: Some(false),
            supports_exception_options: Some(false),
            supports_value_formatting_options: Some(false),
            supports_exception_info_request: Some(false),
            support_terminate_debuggee: Some(false),
            support_suspend_debuggee: Some(false),
            supports_delayed_stack_trace_loading: Some(false),
            supports_loaded_sources_request: Some(false),
            supports_log_points: Some(false),
            supports_terminate_threads_request: Some(false),
            supports_set_expression: Some(false),
            supports_terminate_request: Some(false),
            supports_data_breakpoints: Some(false),
            supports_read_memory_request: Some(false),
            supports_write_memory_request: Some(false),
            supports_disassemble_request: Some(false),
            supports_cancel_request: Some(false),
            supports_breakpoint_locations_request: Some(false),
            supports_clipboard_context: Some(false),
            supports_stepping_granularity: Some(false),
            supports_instruction_breakpoints: Some(false),
            supports_exception_filter_options: Some(false),
        };

        self.send_response(request.seq, "initialize", Some(serde_json::to_value(capabilities).unwrap())).await?;

        // Send initialized event
        self.send_event("initialized", None).await?;

        Ok(())
    }

    async fn handle_launch(&self, request: Request) -> Result<(), WebPipeError> {
        // Launch is handled by the extension (it spawns webpipe-dap)
        // We just acknowledge the request
        self.send_response(request.seq, "launch", None).await
    }

    async fn handle_set_breakpoints(&self, request: Request) -> Result<(), WebPipeError> {
        let args: SetBreakpointsArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid setBreakpoints arguments: {}", e)))?;

        let file_path = args.source.path.unwrap_or_else(|| self.file_path.clone());

        // Extract line numbers
        let lines: Vec<usize> = if let Some(breakpoints) = args.breakpoints {
            breakpoints.iter().map(|bp| bp.line as usize).collect()
        } else if let Some(lines) = args.lines {
            lines.iter().map(|&line| line as usize).collect()
        } else {
            Vec::new()
        };

        eprintln!("[DAP] Set breakpoints on file {}: {:?}", file_path, lines);

        // Update breakpoints and assign IDs
        let mut bp_id_map = std::collections::HashMap::new();
        if lines.is_empty() {
            self.breakpoints.remove(&file_path);
            self.breakpoint_ids.remove(&file_path);
        } else {
            self.breakpoints.insert(file_path.clone(), lines.clone());

            // Assign unique ID to each breakpoint
            for &line in &lines {
                let bp_id = self.next_breakpoint_id.fetch_add(1, Ordering::SeqCst);
                bp_id_map.insert(line, bp_id);
            }
            self.breakpoint_ids.insert(file_path.clone(), bp_id_map.clone());
        }

        // Build response with IDs
        let breakpoints: Vec<Breakpoint> = lines.iter().map(|&line| Breakpoint {
            id: bp_id_map.get(&line).copied(),
            verified: true,
            message: None,
            source: Some(Source {
                name: None,
                path: Some(file_path.clone()),
                source_reference: None,
                presentation_hint: None,
                origin: None,
                sources: None,
                adapter_data: None,
                checksums: None,
            }),
            line: Some(line as i64),
            column: None,
            end_line: None,
            end_column: None,
            instruction_reference: None,
            offset: None,
        }).collect();

        let body = SetBreakpointsResponseBody { breakpoints };

        // Breakpoints updated successfully

        self.send_response(request.seq, "setBreakpoints", Some(serde_json::to_value(body).unwrap())).await
    }

    async fn handle_configuration_done(&self, request: Request) -> Result<(), WebPipeError> {
        self.send_response(request.seq, "configurationDone", None).await
    }

    async fn handle_threads(&self, request: Request) -> Result<(), WebPipeError> {
        // Return all announced threads, not just paused ones.
        // This ensures VS Code knows about threads even when they are running (e.g. stepping).
        let threads: Vec<Thread> = self.announced_threads.iter()
            .map(|id_ref| {
                let id = *id_ref;
                Thread {
                    id: id as i64,
                    name: format!("HTTP Request #{}", id),
                }
            })
            .collect();

        let body = ThreadsResponseBody { threads };

        self.send_response(request.seq, "threads", Some(serde_json::to_value(body).unwrap())).await
    }

    async fn handle_stack_trace(&self, request: Request) -> Result<(), WebPipeError> {
        let args: StackTraceArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid stackTrace arguments: {}", e)))?;

        let thread_id = args.thread_id as u64;

        let stack_frames = if let Some(paused) = self.paused_threads.get(&thread_id) {
            // Map the collected stack (Vec<String>) to DAP StackFrames
            // We reverse it because DAP expects the "top" frame (current step) first
            paused.stack.iter()
                .rev()
                .enumerate()
                .map(|(i, name)| {
                    // For the top frame (index 0), use the real source location
                    // For parent frames, we don't track location yet, so use 0/None
                    let (line, column, source) = if i == 0 {
                        (
                            paused.location.line as i64,
                            paused.location.column as i64,
                            Some(Source {
                                name: None,
                                path: Some(self.file_path.clone()),
                                source_reference: None,
                                presentation_hint: None,
                                origin: None,
                                sources: None,
                                adapter_data: None,
                                checksums: None,
                            })
                        )
                    } else {
                        (0, 0, None) // Parent frames are "de-emphasized" without source
                    };

                    // Encode both thread_id and stack_index into frame_id
                    // frame_id = thread_id * 10000 + stack_index
                    // This allows up to 10,000 stack frames per thread
                    let frame_id = thread_id as i64 * 10000 + i as i64;

                    StackFrame {
                        id: frame_id,
                        name: name.clone(),
                        source,
                        line,
                        column,
                        end_line: None,
                        end_column: None,
                        can_restart: None,
                        instruction_pointer_reference: None,
                        module_id: None,
                        presentation_hint: if i == 0 { None } else { Some("label".to_string()) },
                    }
                })
                .collect()
        } else {
            Vec::new()
        };

        let total_frames = stack_frames.len();

        let body = StackTraceResponseBody {
            stack_frames,
            total_frames: Some(total_frames as i64),
        };

        self.send_response(request.seq, "stackTrace", Some(serde_json::to_value(body).unwrap())).await
    }

    async fn handle_scopes(&self, request: Request) -> Result<(), WebPipeError> {
        let args: ScopesArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid scopes arguments: {}", e)))?;

        // Decode thread_id from frame_id (frame_id = thread_id * 10000 + stack_index)
        let thread_id = (args.frame_id / 10000) as u64;

        // Root scope is always at offset 1 for the thread
        let root_ref = thread_id as i64 * VAR_REF_STRIDE + 1;

        // Register root pointer (empty string = root of JSON document)
        self.variable_pointers.insert(root_ref, "".to_string());

        let scopes = vec![Scope {
            name: "Pipeline State".to_string(),
            presentation_hint: None,
            variables_reference: root_ref,
            named_variables: None,
            indexed_variables: None,
            expensive: false,
            source: None,
            line: None,
            column: None,
            end_line: None,
            end_column: None,
        }];

        let body = ScopesResponseBody { scopes };

        self.send_response(request.seq, "scopes", Some(serde_json::to_value(body).unwrap())).await
    }

    async fn handle_variables(&self, request: Request) -> Result<(), WebPipeError> {
        let args: VariablesArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid variables arguments: {}", e)))?;

        let var_ref = args.variables_reference;
        let thread_id = (var_ref / VAR_REF_STRIDE) as u64;

        // Retrieve parent pointer
        let parent_ptr = self.variable_pointers.get(&var_ref)
            .map(|r| r.value().clone())
            .unwrap_or_default();

        // Need mutable access to thread state to generate new reference IDs
        // even if we are just reading variables
        let variables = if var_ref % VAR_REF_STRIDE == 1 {
            // This is a scope reference - show top-level pipeline state
            if let Some(mut paused) = self.paused_threads.get_mut(&thread_id) {
                // We must clone state to avoid holding lock while calling json_to_variables_cached?
                // Actually json_to_variables_cached takes &self so it's fine,
                // BUT it also modifies variable_cache and variable_pointers.
                // We hold 'paused' (RefMut) here.
                let state = paused.state.clone();
                self.json_to_variables_cached(&state, thread_id, &parent_ptr, &mut paused.var_ref_index)
            } else {
                Vec::new()
            }
        } else {
            // This is a nested variable reference
            // Clone value from cache to avoid deadlock (reading cache while json_to_variables_cached writes to it)
            if let Some(cached_value) = self.variable_cache.get(&var_ref).map(|r| r.clone()) {
                if let Some(mut paused) = self.paused_threads.get_mut(&thread_id) {
                    self.json_to_variables_cached(&cached_value, thread_id, &parent_ptr, &mut paused.var_ref_index)
                } else {
                    Vec::new()
                }
            } else {
                eprintln!("[DAP] Warning: Variable reference {} not found in cache", var_ref);
                Vec::new()
            }
        };

        let body = VariablesResponseBody { variables };

        self.send_response(request.seq, "variables", Some(serde_json::to_value(body).unwrap())).await
    }

    async fn handle_set_variable(&self, request: Request) -> Result<(), WebPipeError> {
        let args: SetVariableArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid setVariable arguments: {}", e)))?;

        let var_ref = args.variables_reference;
        let thread_id = (var_ref / VAR_REF_STRIDE) as u64;

        // Parse the new value (try JSON, fallback to string)
        let new_val = match serde_json::from_str::<serde_json::Value>(&args.value) {
            Ok(v) => v,
            Err(_) => serde_json::Value::String(args.value.clone()),
        };

        // Resolve the JSON Pointer path
        // We need the pointer to the *container* (var_ref) + the child key (args.name)
        let container_ptr = self.variable_pointers.get(&var_ref)
            .map(|r| r.value().clone());

        if let Some(parent_ptr) = container_ptr {
            // Construct full pointer to the specific property being set
            // e.g. parent="/users/0", name="name" -> "/users/0/name"
            let target_ptr = format!("{}/{}", parent_ptr, Self::escape_json_pointer(&args.name));

            if let Some(mut paused) = self.paused_threads.get_mut(&thread_id) {
                // Patch the ROOT state using JSON Pointer
                if let Some(target) = paused.state.pointer_mut(&target_ptr) {
                    *target = new_val.clone();

                    // Update the CACHE (Variable View) to keep UI in sync
                    if let Some(mut cached_container) = self.variable_cache.get_mut(&var_ref) {
                        if let Some(obj) = cached_container.as_object_mut() {
                            obj.insert(args.name.clone(), new_val.clone());
                        } else if let Some(arr) = cached_container.as_array_mut() {
                            // Handle array index parsing if name is "[0]" or "0"
                            if let Ok(idx) = args.name.trim_matches(|c| c == '[' || c == ']').parse::<usize>() {
                                if idx < arr.len() {
                                    arr[idx] = new_val.clone();
                                }
                            }
                        }
                    }

                    // Build success response
                    let body = SetVariableResponseBody {
                        value: format_preview(&new_val),
                        type_: Some(type_name(&new_val)),
                        variables_reference: if is_complex(&new_val) {
                            // Generate thread-local variable reference
                            paused.var_ref_index += 1;
                            let ref_id = thread_id as i64 * VAR_REF_STRIDE + paused.var_ref_index;
                            
                            self.variable_cache.insert(ref_id, new_val.clone());
                            // Store pointer for the new complex value
                            self.variable_pointers.insert(ref_id, target_ptr.clone());
                            Some(ref_id)
                        } else {
                            Some(0)
                        },
                        named_variables: None,
                        indexed_variables: None,
                    };

                    return self.send_response(request.seq, "setVariable", Some(serde_json::to_value(body).unwrap())).await;
                }
            }
        }

        self.send_error_response(request.seq, "setVariable", "Failed to update variable").await
    }

    async fn handle_evaluate(&self, request: Request) -> Result<(), WebPipeError> {
        let args: EvaluateArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid evaluate arguments: {}", e)))?;

        // 1. Resolve Scope
        // DAP gives us a frameId (which we mapped to thread_id in handle_stack_trace)
        // If frameId is missing, we can't evaluate contextually.
        let thread_id = args.frame_id.map(|id| (id / 10000) as u64).unwrap_or(0);

        if let Some(mut paused) = self.paused_threads.get_mut(&thread_id) {
            // 2. Prepare the Expression
            // VS Code sends raw text. Since WebPipe uses JQ, we try to interpret it as a JQ filter.
            // If the user hovers "user", we might want to treat it as ".user".
            let expr = if args.expression.starts_with('.') {
                args.expression.clone()
            } else {
                format!(".{}", args.expression)
            };

            // 3. Evaluate using the existing JQ runtime
            // We use the paused state as the input context
            let eval_result = crate::runtime::jq::evaluate(&expr, &paused.state);

            match eval_result {
                Ok(value) => {
                    // 4. Format Result (Reuse existing variable logic)
                    // If it's a complex object, we cache it so it can be expanded in the UI
                    let var_ref = if is_complex(&value) {
                        paused.var_ref_index += 1;
                        let ref_id = thread_id as i64 * VAR_REF_STRIDE + paused.var_ref_index;

                        self.variable_cache.insert(ref_id, value.clone());
                        ref_id
                    } else {
                        0
                    };

                    let response_body = EvaluateResponseBody {
                        result: format_preview(&value),
                        type_: Some(type_name(&value)),
                        variables_reference: var_ref,
                        named_variables: if matches!(&value, serde_json::Value::Object(m) if !m.is_empty()) {
                            Some(value.as_object().unwrap().len() as i64)
                        } else {
                            None
                        },
                        indexed_variables: if matches!(&value, serde_json::Value::Array(a) if !a.is_empty()) {
                            Some(value.as_array().unwrap().len() as i64)
                        } else {
                            None
                        },
                        presentation_hint: None,
                    };

                    self.send_response(request.seq, "evaluate", Some(serde_json::to_value(response_body).unwrap())).await
                }
                Err(e) => {
                    // Evaluation failed (e.g. invalid path).
                    // This is common for hovers over keywords or non-variables.
                    // We return a failure so VS Code doesn't show a popup.
                    self.send_error_response(request.seq, "evaluate", &format!("Evaluation failed: {}", e)).await
                }
            }
        } else {
            self.send_error_response(request.seq, "evaluate", "Thread not paused").await
        }
    }

    async fn handle_continue(&self, request: Request) -> Result<(), WebPipeError> {
        let args: ContinueArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid continue arguments: {}", e)))?;

        let thread_id = args.thread_id as u64;

        self.clear_thread_variables(thread_id);
        self.stepping_threads.remove(&thread_id);

        if let Some((_, paused)) = self.paused_threads.remove(&thread_id) {
            // Pass the potentially modified state back to the hook
            let _ = paused.resume_tx.send(StepCommand::Continue(Some(paused.state)));
        }

        let body = ContinueResponseBody {
            all_threads_continued: Some(false),
        };
        self.send_response(request.seq, "continue", Some(serde_json::to_value(body).unwrap())).await
    }

    async fn handle_next(&self, request: Request) -> Result<(), WebPipeError> {
        let args: NextArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid next arguments: {}", e)))?;

        let thread_id = args.thread_id as u64;

        self.clear_thread_variables(thread_id);

        let current_depth = if let Some(paused) = self.paused_threads.get(&thread_id) {
            paused.stack.len()
        } else {
            1
        };

        self.stepping_threads.insert(thread_id, SteppingMode::StepOver { target_depth: current_depth });

        if let Some((_, paused)) = self.paused_threads.remove(&thread_id) {
            let _ = paused.resume_tx.send(StepCommand::StepOver(Some(paused.state)));
        }

        self.send_response(request.seq, "next", None).await
    }

    async fn handle_step_in(&self, request: Request) -> Result<(), WebPipeError> {
        let args: StepInArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid stepIn arguments: {}", e)))?;

        let thread_id = args.thread_id as u64;

        self.clear_thread_variables(thread_id);

        self.stepping_threads.insert(thread_id, SteppingMode::StepInto);

        if let Some((_, paused)) = self.paused_threads.remove(&thread_id) {
            let _ = paused.resume_tx.send(StepCommand::StepIn(Some(paused.state)));
        }
        self.send_response(request.seq, "stepIn", None).await
    }

    async fn handle_step_out(&self, request: Request) -> Result<(), WebPipeError> {
        let args: StepOutArguments = serde_json::from_value(request.arguments.unwrap_or_default())
            .map_err(|e| WebPipeError::DebuggerError(format!("Invalid stepOut arguments: {}", e)))?;

        let thread_id = args.thread_id as u64;

        self.clear_thread_variables(thread_id);

        let current_depth = if let Some(paused) = self.paused_threads.get(&thread_id) {
            paused.stack.len()
        } else {
            1
        };

        self.stepping_threads.insert(thread_id, SteppingMode::StepOut { target_depth: current_depth });

        if let Some((_, paused)) = self.paused_threads.remove(&thread_id) {
            let _ = paused.resume_tx.send(StepCommand::StepOut(Some(paused.state)));
        }
        self.send_response(request.seq, "stepOut", None).await
    }

    async fn handle_pause(&self, request: Request) -> Result<(), WebPipeError> {
        // VS Code sends pause when it thinks execution is running
        // If we're already paused, just acknowledge
        // This shouldn't happen in normal flow, but handle it gracefully
        eprintln!("[DAP] Pause command received (already paused or no active threads)");
        self.send_response(request.seq, "pause", None).await
    }

    async fn handle_source(&self, request: Request) -> Result<(), WebPipeError> {
        // VS Code requests source content - read the .wp file
        match std::fs::read_to_string(&self.file_path) {
            Ok(content) => {
                let body = json!({
                    "content": content,
                    "mimeType": "text/plain"
                });
                self.send_response(request.seq, "source", Some(body)).await
            }
            Err(e) => {
                eprintln!("[DAP] Failed to read source file {}: {}", self.file_path, e);
                self.send_error_response(request.seq, "source", &format!("Failed to read source: {}", e)).await
            }
        }
    }

    async fn handle_disconnect(&self, request: Request) -> Result<(), WebPipeError> {
        eprintln!("[DAP] Disconnecting...");

        let thread_ids: Vec<u64> = self.paused_threads.iter().map(|entry| *entry.key()).collect();
        for thread_id in thread_ids {
            if let Some((_, paused)) = self.paused_threads.remove(&thread_id) {
                // Return state as-is on disconnect
                let _ = paused.resume_tx.send(StepCommand::Continue(Some(paused.state)));
            }
        }
        // ... rest of handle_disconnect ...
        self.announced_threads.clear();
        self.send_response(request.seq, "disconnect", None).await?;
        if let Some(notify) = &self.shutdown_notify {
            notify.notify_one();
        }
        Ok(())
    }

    // ========================================================================
    // Helper Methods
    // ========================================================================

    /// Convert JSON value to DAP variables with caching for nested expansion
    /// Generates thread-local variable references using the provided counter.
    fn json_to_variables_cached(&self, value: &serde_json::Value, thread_id: u64, parent_ptr: &str, var_ref_counter: &mut i64) -> Vec<Variable> {
        match value {
            serde_json::Value::Object(map) => {
                map.iter().map(|(k, v)| {
                    let var_ref = if is_complex(v) {
                        // Generate a new variable reference relative to thread
                        *var_ref_counter += 1;
                        let ref_id = thread_id as i64 * VAR_REF_STRIDE + *var_ref_counter;
                        
                        self.variable_cache.insert(ref_id, v.clone());

                        // Store pointer: parent + / + escaped_key
                        let child_ptr = format!("{}/{}", parent_ptr, Self::escape_json_pointer(k));
                        self.variable_pointers.insert(ref_id, child_ptr);

                        ref_id
                    } else {
                        0
                    };

                    Variable {
                        name: k.clone(),
                        value: format_preview(v),
                        type_: Some(type_name(v)),
                        presentation_hint: None,
                        evaluate_name: None,
                        variables_reference: var_ref,
                        named_variables: if matches!(v, serde_json::Value::Object(m) if !m.is_empty()) {
                            Some(v.as_object().unwrap().len() as i64)
                        } else {
                            None
                        },
                        indexed_variables: if matches!(v, serde_json::Value::Array(a) if !a.is_empty()) {
                            Some(v.as_array().unwrap().len() as i64)
                        } else {
                            None
                        },
                        memory_reference: None,
                    }
                }).collect()
            }
            serde_json::Value::Array(arr) => {
                arr.iter().enumerate().map(|(i, v)| {
                    let var_ref = if is_complex(v) {
                        // Generate a new variable reference relative to thread
                        *var_ref_counter += 1;
                        let ref_id = thread_id as i64 * VAR_REF_STRIDE + *var_ref_counter;

                        self.variable_cache.insert(ref_id, v.clone());

                        // Store pointer: parent + / + index
                        let child_ptr = format!("{}/{}", parent_ptr, i);
                        self.variable_pointers.insert(ref_id, child_ptr);

                        ref_id
                    } else {
                        0
                    };

                    Variable {
                        name: format!("[{}]", i),
                        value: format_preview(v),
                        type_: Some(type_name(v)),
                        presentation_hint: None,
                        evaluate_name: None,
                        variables_reference: var_ref,
                        named_variables: if matches!(v, serde_json::Value::Object(m) if !m.is_empty()) {
                            Some(v.as_object().unwrap().len() as i64)
                        } else {
                            None
                        },
                        indexed_variables: if matches!(v, serde_json::Value::Array(a) if !a.is_empty()) {
                            Some(v.as_array().unwrap().len() as i64)
                        } else {
                            None
                        },
                        memory_reference: None,
                    }
                }).collect()
            }
            serde_json::Value::String(s) => {
                // For standalone strings, show as a single value
                vec![Variable {
                    name: "value".to_string(),
                    value: format!("\"{}\"", s),
                    type_: Some("string".to_string()),
                    presentation_hint: None,
                    evaluate_name: None,
                    variables_reference: 0,
                    named_variables: None,
                    indexed_variables: None,
                    memory_reference: None,
                }]
            }
            serde_json::Value::Number(n) => {
                vec![Variable {
                    name: "value".to_string(),
                    value: n.to_string(),
                    type_: Some("number".to_string()),
                    presentation_hint: None,
                    evaluate_name: None,
                    variables_reference: 0,
                    named_variables: None,
                    indexed_variables: None,
                    memory_reference: None,
                }]
            }
            serde_json::Value::Bool(b) => {
                vec![Variable {
                    name: "value".to_string(),
                    value: b.to_string(),
                    type_: Some("boolean".to_string()),
                    presentation_hint: None,
                    evaluate_name: None,
                    variables_reference: 0,
                    named_variables: None,
                    indexed_variables: None,
                    memory_reference: None,
                }]
            }
            serde_json::Value::Null => {
                vec![Variable {
                    name: "value".to_string(),
                    value: "null".to_string(),
                    type_: Some("null".to_string()),
                    presentation_hint: None,
                    evaluate_name: None,
                    variables_reference: 0,
                    named_variables: None,
                    indexed_variables: None,
                    memory_reference: None,
                }]
            }
        }
    }

    /// Get breakpoint ID for a given line (if breakpoint exists)
    pub fn get_breakpoint_id(&self, line: usize) -> Option<i64> {
        self.breakpoint_ids.get(&self.file_path)
            .and_then(|map| map.get(&line).copied())
    }

    /// Check if there's a breakpoint at the given line
    pub fn has_breakpoint(&self, line: usize) -> bool {
        self.breakpoints.get(&self.file_path)
            .map(|lines| lines.contains(&line))
            .unwrap_or(false)
    }

    /// Send a "stopped" event to VS Code when a breakpoint is hit
    pub async fn send_stopped_event(&self, thread_id: u64, reason: &str, breakpoint_id: Option<i64>) -> Result<(), WebPipeError> {
        let body = StoppedEventBody {
            reason: reason.to_string(),
            description: Some(format!("Paused on {}", reason)),
            thread_id: Some(thread_id as i64),
            preserve_focus_hint: Some(false),  // Force VS Code to focus this thread
            text: None,
            all_threads_stopped: Some(true),  // Force VS Code to refresh all threads/stacks
            hit_breakpoint_ids: breakpoint_id.map(|id| vec![id]),
        };

        let body_json = serde_json::to_value(&body).unwrap();

        eprintln!("[DAP] Sending stopped event for thread {}: {:?}", thread_id, body.reason);
        eprintln!("[DAP] Stopped event body: {}", serde_json::to_string_pretty(&body_json).unwrap());

        // Removed explicit output event to avoid potential race with stopped event

        self.send_event("stopped", Some(body_json)).await
    }

    /// Send a "continued" event to VS Code when execution resumes
    pub async fn send_continued_event(&self, thread_id: u64) -> Result<(), WebPipeError> {
        let body = json!({
            "threadId": thread_id as i64,
            "allThreadsContinued": false
        });

        self.send_event("continued", Some(body)).await
    }

    /// Send a "thread" event to VS Code when a thread starts or exits
    pub async fn send_thread_event(&self, thread_id: u64, reason: &str) -> Result<(), WebPipeError> {
        let body = json!({
            "reason": reason,  // "started" or "exited"
            "threadId": thread_id as i64
        });

        eprintln!("[DAP] Thread {} {}", thread_id, reason);
        self.send_event("thread", Some(body)).await
    }
}

// ============================================================================
// DapDebuggerHook Implementation
// ============================================================================

/// Debugger hook that integrates with DAP server
pub struct DapDebuggerHook {
    pub server: Arc<DapServer>,
}

#[async_trait]
impl DebuggerHook for DapDebuggerHook {
    fn allocate_thread_id(&self) -> u64 {
        self.server.allocate_thread_id()
    }

    async fn before_step(
        &self,
        thread_id: u64,
        step_name: &str,
        location: &SourceLocation,
        state: &mut serde_json::Value, // Changed to mutable
        stack: Vec<String>,
    ) -> Result<StepAction, WebPipeError> {
        let at_breakpoint = self.server.has_breakpoint(location.line);
        let stack_depth = stack.len();

        let should_pause_step = if let Some(mode) = self.server.stepping_threads.get(&thread_id) {
            match *mode {
                SteppingMode::StepInto => true,
                SteppingMode::StepOver { target_depth } => stack_depth <= target_depth,
                SteppingMode::StepOut { target_depth } => stack_depth < target_depth,
            }
        } else {
            false
        };

        if at_breakpoint || should_pause_step {
            // ... announce thread logic ...
            if !self.server.announced_threads.contains(&thread_id) {
                self.server.announced_threads.insert(thread_id);
                self.server.send_thread_event(thread_id, "started").await?;
            }

            let reason = if at_breakpoint { "breakpoint" } else { "step" };
            eprintln!("[DAP] Paused at line {} (thread {}, reason: {})", location.line, thread_id, reason);

            let breakpoint_id = if at_breakpoint {
                self.server.get_breakpoint_id(location.line)
            } else {
                None
            };

            self.server.stepping_threads.remove(&thread_id);

            let (tx, rx) = oneshot::channel();

            self.server.paused_threads.insert(thread_id, PausedState {
                resume_tx: tx,
                state: state.clone(),
                location: location.clone(),
                step_name: step_name.to_string(),
                paused_at: std::time::Instant::now(),
                stack,
                var_ref_index: 1, // Start index at 1 (0 reserved/unused, root uses index 1 explicitly in handle_scopes)
            });

            self.server.send_stopped_event(thread_id, reason, breakpoint_id).await?;

            let command = rx.await
                .map_err(|_| WebPipeError::DebuggerError("Resume channel closed".to_string()))?;

            eprintln!("[DAP] Resuming thread {} with command {:?}", thread_id, command);

            // Extract the new state and action from the command
            let (action, new_state) = match command {
                StepCommand::Continue(s) => (StepAction::Continue, s),
                StepCommand::StepOver(s) => (StepAction::StepOver, s),
                StepCommand::StepIn(s) => (StepAction::StepIn, s),
                StepCommand::StepOut(s) => (StepAction::StepOut, s),
            };

            // Write back the modified state if present
            if let Some(s) = new_state {
                *state = s;
            }

            Ok(action)
        } else {
            Ok(StepAction::Continue)
        }
    }

    // ... after_step ...
    async fn after_step(
        &self,
        _thread_id: u64,
        _step_name: &str,
        _location: &SourceLocation,
        _state: &serde_json::Value,
        _stack: Vec<String>,
    ) {
    }
}

// ============================================================================
// Variable Introspection Helpers
// ============================================================================

fn is_complex(v: &serde_json::Value) -> bool {
    matches!(v, serde_json::Value::Object(_) | serde_json::Value::Array(_))
}

fn format_preview(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(_) => "{...}".to_string(),
        serde_json::Value::Array(arr) => format!("[{} items]", arr.len()),
        serde_json::Value::String(s) => format!("\"{}\"", s),
        _ => v.to_string(),
    }
}

fn type_name(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }.to_string()
}