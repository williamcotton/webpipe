/// WebPipe DAP (Debug Adapter Protocol) Binary
///
/// This binary starts both:
/// 1. HTTP server for handling web requests (default port 7770)
/// 2. DAP server for debugging protocol (default port 5858)
///
/// Usage:
///   webpipe-dap <file.wp> [--port PORT] [--debug-port DEBUG_PORT]
///
/// Example:
///   webpipe-dap example.wp --port 7770 --debug-port 5858

use std::env;
use std::sync::Arc;
use std::net::SocketAddr;
use std::path::Path;
use tokio::sync::Notify;
use webpipe::ast::parse_program;
use webpipe::debugger::{DapServer, dap_server::DapDebuggerHook};
use webpipe::server::WebPipeServer;

fn load_env_files(env_dir: &Path) {
    // Load .env files located next to the WebPipe file
    // Priority: .env.local > .env
    let files = [".env", ".env.local"];
    for fname in files.iter() {
        let p = env_dir.join(fname);
        if p.exists() {
            // We use from_filename (not override) to respect existing env vars
            if let Err(e) = dotenvy::from_filename(&p) {
                eprintln!("[DAP] Warning: Failed to load {}: {}", p.display(), e);
            } else {
                eprintln!("[DAP] Loaded environment from {}", p.display());
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber for logging
    tracing_subscriber::fmt()
        .with_env_filter("webpipe=debug,tower_http=debug")
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} <file.wp> [--port PORT] [--debug-port DEBUG_PORT]", args[0]);
        eprintln!();
        eprintln!("Options:");
        eprintln!("  --port PORT          HTTP server port (default: 7770)");
        eprintln!("  --debug-port PORT    DAP protocol port (default: 5858)");
        eprintln!();
        eprintln!("Example:");
        eprintln!("  {} example.wp --port 7770 --debug-port 5858", args[0]);
        std::process::exit(1);
    }

    let file_path = &args[1];

    // Load environment variables relative to the .wp file
    // This matches the behavior of the main `webpipe` CLI
    let env_dir = Path::new(file_path).parent().unwrap_or(Path::new("."));
    load_env_files(env_dir);

    // Point public dir to <webpipe_dir>/public if not explicitly set
    if std::env::var("WEBPIPE_PUBLIC_DIR").is_err() {
        let public_path = env_dir.join("public");
        if let Some(s) = public_path.to_str() {
            std::env::set_var("WEBPIPE_PUBLIC_DIR", s);
        }
    }

    // Convert to absolute path for breakpoint matching
    let absolute_path = std::fs::canonicalize(file_path)
        .map_err(|e| format!("Failed to resolve absolute path for {}: {}", file_path, e))?;
    let file_path_str = absolute_path.to_str()
        .ok_or_else(|| format!("Path contains invalid UTF-8: {:?}", absolute_path))?
        .to_string();

    // Parse optional ports
    let port = args.iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(7770);

    let debug_port = args.iter()
        .position(|a| a == "--debug-port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(5858);

    eprintln!("=================================");
    eprintln!("WebPipe Debugger");
    eprintln!("=================================");
    eprintln!("File:        {}", file_path_str);
    eprintln!("HTTP Port:   {}", port);
    eprintln!("Debug Port:  {}", debug_port);
    eprintln!("=================================");
    eprintln!();

    // Load and parse program
    eprintln!("[DAP] Loading program from {}", file_path_str);
    let input = std::fs::read_to_string(&file_path_str)
        .map_err(|e| format!("Failed to read file {}: {}", file_path_str, e))?;

    let (_leftover, program) = parse_program(&input)
        .map_err(|e| format!("Failed to parse program: {:?}", e))?;

    eprintln!("[DAP] Program loaded successfully");
    eprintln!("[DAP] Routes: {}", program.routes.len());
    eprintln!("[DAP] Pipelines: {}", program.pipelines.len());
    eprintln!();

    // Create a notifier for shutdown signaling
    let shutdown_notify = Arc::new(Notify::new());

    // Create DAP server with shutdown notifier
    let dap_server = Arc::new(DapServer::new(file_path_str.clone(), debug_port, Some(shutdown_notify.clone())));

    // Start DAP TCP listener (waits for VS Code connection)
    eprintln!("[DAP] Starting DAP server on 127.0.0.1:{}...", debug_port);
    let dap_clone = dap_server.clone();
    tokio::spawn(async move {
        if let Err(e) = dap_clone.start_tcp_listener().await {
            eprintln!("[DAP] DAP server error: {}", e);
        }
    });

    // Give the DAP server a moment to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create debugger hook
    let debugger_hook = Arc::new(DapDebuggerHook { server: dap_server });

    // Create WebPipeServer with debugger hook
    eprintln!("[HTTP] Building server...");
    let server = WebPipeServer::from_program_with_debugger(
        program,
        false, // trace_mode
        Some(debugger_hook)
    ).await?;

    // Start HTTP server
    let addr_str = format!("127.0.0.1:{}", port);
    let addr: SocketAddr = addr_str.parse()
        .map_err(|e| format!("Invalid address {}: {}", addr_str, e))?;

    eprintln!("[HTTP] Starting HTTP server on {}...", addr);
    eprintln!();
    eprintln!("✓ WebPipe debugger ready!");
    eprintln!("  • Press F5 in VS Code to connect debugger");
    eprintln!("  • Send HTTP requests to http://{}", addr);
    eprintln!("  • Set breakpoints in your .wp file");
    eprintln!();

    // Create a composite shutdown future that listens for OS signals OR DAP disconnect
    let shutdown_future = async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\n[DAP] Ctrl+C received, shutting down...");
            },
            _ = shutdown_notify.notified() => {
                eprintln!("\n[DAP] Debugger disconnected, shutting down...");
            }
        }
        
        // Safety net: Force exit if graceful shutdown hangs (e.g. keep-alive connections)
        // This ensures the process actually dies and releases the ports.
        tokio::spawn(async {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            eprintln!("[DAP] Shutdown timed out (2s), forcing exit");
            std::process::exit(0);
        });
    };

    server.serve_with_shutdown(addr, shutdown_future).await?;

    Ok(())
}