use std::{env, error::Error, net::SocketAddr, path::Path, sync::mpsc as std_mpsc, time::Duration};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::{sync::{mpsc as tokio_mpsc, oneshot}, signal};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use webpipe::{ast::parse_program, WebPipeServer, run_tests};

#[cfg(feature = "debugger")]
use std::sync::Arc;
#[cfg(feature = "debugger")]
use webpipe::debugger::{DapServer, dap_server::DapDebuggerHook};
#[cfg(feature = "debugger")]
use tokio::sync::Notify;

fn load_env_files(env_dir: &Path, override_existing: bool, debug: bool) {
    // Load .env files located next to the WebPipe file
    let files = [".env", ".env.local"];
    for fname in files.iter() {
        let p = env_dir.join(fname);
        if p.exists() {
            let result = if override_existing {
                dotenvy::from_filename_override(&p)
            } else {
                dotenvy::from_filename(&p)
            };
            if debug && result.is_err() {
                eprintln!("Warning: Failed to load {}: {}", p.display(), result.unwrap_err());
                eprintln!("Hint: Check .env file syntax - each line should be KEY=VALUE format");
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "webpipe=debug,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args: Vec<String> = env::args().collect();
    
    if args.len() < 2 {
        eprintln!("Usage: {} <webpipe_file> [host:port] [--test] [--verbose] [--trace] [--inspect] [--inspect-port PORT] [--port PORT]", args[0]);
        std::process::exit(1);
    }

    let file_path = &args[1];
    // Determine if test mode, verbose mode, and trace mode
    let test_mode = args.iter().any(|a| a == "--test");
    let verbose_mode = args.iter().any(|a| a == "--verbose");
    let trace_mode = args.iter().any(|a| a == "--trace");

    // Parse --inspect flag
    #[cfg(feature = "debugger")]
    let inspect_mode = args.iter().any(|a| a == "--inspect");
    #[cfg(not(feature = "debugger"))]
    let inspect_mode = false;

    // Validate incompatible flags
    if test_mode && inspect_mode {
        eprintln!("Error: --test and --inspect cannot be used together");
        std::process::exit(1);
    }

    // Error if inspect mode requested but debugger feature not enabled
    #[cfg(not(feature = "debugger"))]
    if inspect_mode {
        eprintln!("Error: --inspect requires webpipe to be built with the debugger feature");
        eprintln!("Rebuild with: cargo build --features debugger");
        std::process::exit(1);
    }

    // Parse --inspect-port (only if debugger feature enabled)
    #[cfg(feature = "debugger")]
    let inspect_port: u16 = args.iter()
        .position(|a| a == "--inspect-port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(5858);

    // Determine the directory for .env files (same directory as the WebPipe file)
    let env_dir = Path::new(file_path).parent().unwrap_or(Path::new("."));

    // Initial load of .env files (do not override already-set process vars)
    load_env_files(env_dir, false, verbose_mode);

    // Point public dir to <webpipe_dir>/public if not explicitly set
    if std::env::var("WEBPIPE_PUBLIC_DIR").is_err() {
        let public_path = env_dir.join("public");
        if let Some(s) = public_path.to_str() {
            std::env::set_var("WEBPIPE_PUBLIC_DIR", s);
        }
    }

    let default_addr = "127.0.0.1:7770".to_string();

    // If --port is explicitly provided, use it
    let explicit_port_flag = args.iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse::<u16>().ok());

    // If explicit CLI addr is provided (and not a flag), use it. Otherwise, if PORT env is set (Heroku), bind 0.0.0.0:PORT.
    let explicit_cli_addr = args.get(2).filter(|v| {
        let s = v.as_str();
        s != "--test" && s != "--trace" && s != "--verbose"
            && s != "--inspect" && s != "--inspect-port" && s != "--port"
    }).cloned();

    let addr_str = if let Some(port) = explicit_port_flag {
        format!("127.0.0.1:{}", port)
    } else if let Some(a) = explicit_cli_addr {
        a
    } else if let Ok(port) = std::env::var("PORT") {
        format!("0.0.0.0:{}", port)
    } else {
        default_addr.clone()
    };
    
    // Parse the address
    let addr: SocketAddr = addr_str.parse()?;

    // File change notification: bridge notify (std thread) -> tokio
    let (raw_tx, raw_rx): (std_mpsc::Sender<notify::Result<notify::Event>>, std_mpsc::Receiver<notify::Result<notify::Event>>) = std_mpsc::channel();
    let (change_tx, mut change_rx) = tokio_mpsc::unbounded_channel::<()>();

    // Spawn a blocking thread to forward notify events into tokio channel
    std::thread::spawn(move || {
        while let Ok(res) = raw_rx.recv() {
            if res.is_ok() { let _ = change_tx.send(()); }
        }
    });

    // Set up watcher (only for serve mode, not test or inspect mode)
    let mut _watcher: Option<RecommendedWatcher> = None;
    if !test_mode && !inspect_mode {
        let mut w: RecommendedWatcher = notify::recommended_watcher(move |res| {
            let _ = raw_tx.send(res);
        })?;
        w.watch(Path::new(file_path), RecursiveMode::NonRecursive)?;
        // Also watch the directory for .env changes (creation/modification)
        let _ = w.watch(env_dir, RecursiveMode::NonRecursive);
        _watcher = Some(w);
    }

    loop {
        // Reload .env files each iteration so edits take effect on hot-reload
        load_env_files(env_dir, true, false);

        // Parse the WebPipe file
        let input = std::fs::read_to_string(file_path)?;
        let (_leftover_input, program) = parse_program(&input)
            .map_err(|e| format!("Parse error: {}", e))?;

        if test_mode {
            // Run tests once and exit
            match run_tests(program, verbose_mode).await {
                Ok(summary) => {
                    println!("Test Results: {}/{} passed ({} failed)", summary.passed, summary.total, summary.failed);
                    for o in summary.outcomes {
                        if o.passed {
                            println!("[PASS] {} :: {}", o.describe, o.test);
                        } else {
                            println!("[FAIL] {} :: {} -> {}", o.describe, o.test, o.message);
                        }
                    }
                    if summary.failed > 0 { std::process::exit(1); } else { std::process::exit(0); }
                }
                Err(e) => {
                    eprintln!("Test runner error: {}", e);
                    std::process::exit(1);
                }
            }
        } else if inspect_mode {
            // Run in inspect/debug mode
            #[cfg(feature = "debugger")]
            {
                return run_inspect_mode(file_path, program, addr, inspect_port, trace_mode).await;
            }
        } else {
            // Create and start the server with a shutdown signal
            let server = match WebPipeServer::from_program(program, trace_mode).await {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("Failed to build server: {}", e);
                    std::process::exit(1);
                }
            };
            let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
            let mut serve_fut = Box::pin(server.serve_with_shutdown(addr, async move { let _ = shutdown_rx.await; }));

            tokio::select! {
                res = &mut serve_fut => {
                    if let Err(e) = res { eprintln!("Server error: {}", e); }
                    // If the server ended (e.g., bind failure), exit.
                    break;
                }
                _ = change_rx.recv() => {
                    // Debounce a bit and drain queued events
                    tokio::time::sleep(Duration::from_millis(150)).await;
                    while change_rx.try_recv().is_ok() {}
                    let _ = shutdown_tx.send(());
                    // Ensure the server stops before we restart
                    let _ = serve_fut.await;
                    // Loop will reparse and restart
                    continue;
                }
                _ = signal::ctrl_c() => {
                    let _ = shutdown_tx.send(());
                    let _ = serve_fut.await;
                    break;
                }
            }
        }
    }

    Ok(())
}

#[cfg(feature = "debugger")]
async fn run_inspect_mode(
    file_path: &str,
    program: webpipe::ast::Program,
    addr: SocketAddr,
    inspect_port: u16,
    trace_mode: bool,
) -> Result<(), Box<dyn Error>> {
    use webpipe::ast::Program;

    // Convert to absolute path for breakpoint matching
    let absolute_path = std::fs::canonicalize(file_path)
        .map_err(|e| format!("Failed to resolve absolute path for {}: {}", file_path, e))?;
    let file_path_str = absolute_path.to_str()
        .ok_or_else(|| format!("Path contains invalid UTF-8: {:?}", absolute_path))?
        .to_string();

    eprintln!("=================================");
    eprintln!("WebPipe Debugger");
    eprintln!("=================================");
    eprintln!("File:        {}", file_path_str);
    eprintln!("HTTP Port:   {}", addr.port());
    eprintln!("Debug Port:  {}", inspect_port);
    eprintln!("=================================");
    eprintln!();

    // Create shutdown notifier
    let shutdown_notify = Arc::new(Notify::new());

    // Create DAP server
    let dap_server = Arc::new(DapServer::new(
        file_path_str.clone(),
        inspect_port,
        Some(shutdown_notify.clone())
    ));

    // Start DAP TCP listener
    eprintln!("[DAP] Starting DAP server on 127.0.0.1:{}...", inspect_port);
    let dap_clone = dap_server.clone();
    tokio::spawn(async move {
        if let Err(e) = dap_clone.start_tcp_listener().await {
            eprintln!("[DAP] DAP server error: {}", e);
        }
    });

    // Give DAP server time to bind
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Create debugger hook
    let debugger_hook = Arc::new(DapDebuggerHook { server: dap_server });

    // Build server with debugger
    eprintln!("[HTTP] Building server...");
    let server = WebPipeServer::from_program_with_debugger(
        program,
        trace_mode,
        Some(debugger_hook)
    ).await?;

    eprintln!("[HTTP] Starting HTTP server on {}...", addr);
    eprintln!();
    eprintln!("✓ WebPipe debugger ready!");
    eprintln!("  • Press F5 in VS Code to connect debugger");
    eprintln!("  • Send HTTP requests to http://{}", addr);
    eprintln!("  • Set breakpoints in your .wp file");
    eprintln!();

    // Composite shutdown: Ctrl+C OR DAP disconnect
    let shutdown_future = async move {
        tokio::select! {
            _ = signal::ctrl_c() => {
                eprintln!("\n[DAP] Ctrl+C received, shutting down...");
            },
            _ = shutdown_notify.notified() => {
                eprintln!("\n[DAP] Debugger disconnected, shutting down...");
            }
        }

        // Force exit after 2s if graceful shutdown hangs
        tokio::spawn(async {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            eprintln!("[DAP] Shutdown timed out (2s), forcing exit");
            std::process::exit(0);
        });
    };

    server.serve_with_shutdown(addr, shutdown_future).await?;

    Ok(())
}