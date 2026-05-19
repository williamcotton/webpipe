use std::{error::Error, net::SocketAddr, path::Path, sync::mpsc as std_mpsc, time::Duration};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use tokio::{sync::{mpsc as tokio_mpsc, oneshot}, signal};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use webpipe::{ast::parse_program, WebPipeServer, run_tests, run_tests_with_debugger};
use clap::Parser;

use std::sync::Arc;
use webpipe::debugger::{DapServer, dap_server::DapDebuggerHook};
use tokio::sync::Notify;

mod cli;
mod scaffold;
mod migrations;

const DEFAULT_LOG_FILTER: &str = "webpipe=debug,tower_http=debug";
const SCRIPT_LOG_FILTER: &str = "webpipe=warn,tower_http=warn";

fn init_tracing(default_filter: &'static str) {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();
}

fn tracing_filter_for_mode(mode: &cli::OperationMode) -> Option<&'static str> {
    match mode {
        cli::OperationMode::Auto(_) | cli::OperationMode::Inspect(_) => None,
        cli::OperationMode::Run(_)
        | cli::OperationMode::Test(_)
        | cli::OperationMode::TestInspect(_) => Some(SCRIPT_LOG_FILTER),
        cli::OperationMode::Init
        | cli::OperationMode::Migrate
        | cli::OperationMode::Serve(_)
        | cli::OperationMode::Help => Some(DEFAULT_LOG_FILTER),
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn cli_for_inspect(serve: bool) -> cli::Cli {
        cli::Cli {
            command: None,
            file: None,
            address: None,
            port: None,
            test: false,
            run: false,
            serve,
            verbose: false,
            trace: false,
            inspect: true,
            inspect_port: 5858,
            script_args: Vec::new(),
        }
    }

    #[test]
    fn inspect_uses_script_mode_for_root_main_pipeline() {
        let src = r#"
pipeline main =
  |> jq: `{ ok: true }`
"#;
        let (_rest, program) = parse_program(src).unwrap();

        assert!(inspect_runs_standalone_script(&program, &cli_for_inspect(false)));
    }

    #[test]
    fn inspect_serve_flag_keeps_http_mode_when_main_exists() {
        let src = r#"
pipeline main =
  |> jq: `{ ok: true }`

GET /
  |> jq: `{ route: true }`
"#;
        let (_rest, program) = parse_program(src).unwrap();

        assert!(!inspect_runs_standalone_script(&program, &cli_for_inspect(true)));
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = cli::Cli::parse();

    // Install the parsed CLI argv (everything after `--`) as the process-wide
    // `$context.args` / `$context.options` default for every pipeline run.
    webpipe::cli_args::init_global(webpipe::cli_args::parse(&cli.script_args));

    let mode = cli.mode();
    if let Some(default_filter) = tracing_filter_for_mode(&mode) {
        init_tracing(default_filter);
    }

    match mode {
        cli::OperationMode::Init => {
            scaffold::handle_init(&cli)?;
        }
        cli::OperationMode::Migrate => {
            migrations::handle_migrate(&cli).await?;
        }
        cli::OperationMode::Auto(file_path) => {
            auto_mode(&file_path, &cli).await?;
        }
        cli::OperationMode::Run(file_path) => {
            run_mode(&file_path, &cli, true).await?;
        }
        cli::OperationMode::Serve(file_path) => {
            serve_mode(&file_path, &cli).await?;
        }
        cli::OperationMode::Test(file_path) => {
            test_mode(&file_path, &cli).await?;
        }
        cli::OperationMode::Inspect(file_path) => {
            inspect_mode(&file_path, &cli).await?;
        }
        cli::OperationMode::TestInspect(file_path) => {
            test_inspect_mode(&file_path, &cli).await?;
        }
        cli::OperationMode::Help => {
            eprintln!("Error: No subcommand or file provided");
            eprintln!();
            eprintln!("Usage:");
            eprintln!("  webpipe <FILE> [OPTIONS]         Run WebPipe file");
            eprintln!("  webpipe init [DIR] [--claude]    Initialize a new project");
            eprintln!("  webpipe migrate <COMMAND>        Manage database migrations");
            eprintln!();
            eprintln!("Run 'webpipe --help' for more information");
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn auto_mode(file_path: &Path, cli: &cli::Cli) -> Result<(), Box<dyn Error>> {
    let input = std::fs::read_to_string(file_path)?;
    let (_leftover_input, program) = parse_program(&input)
        .map_err(|e| format!("Parse error: {}", e))?;

    if program.pipelines.iter().any(|pipeline| pipeline.name == "main") {
        init_tracing(SCRIPT_LOG_FILTER);
        run_parsed_program(file_path, program, cli).await
    } else {
        init_tracing(DEFAULT_LOG_FILTER);
        serve_mode(file_path, cli).await
    }
}

async fn run_mode(
    file_path: &Path,
    cli: &cli::Cli,
    require_main: bool,
) -> Result<(), Box<dyn Error>> {
    let input = std::fs::read_to_string(file_path)?;
    let (_leftover_input, program) = parse_program(&input)
        .map_err(|e| format!("Parse error: {}", e))?;

    if require_main && !program.pipelines.iter().any(|pipeline| pipeline.name == "main") {
        eprintln!("Error: --run requires a root-level pipeline named main");
        std::process::exit(1);
    }

    run_parsed_program(file_path, program, cli).await
}

async fn run_parsed_program(
    file_path: &Path,
    program: webpipe::ast::Program,
    cli: &cli::Cli,
) -> Result<(), Box<dyn Error>> {
    let env_dir = file_path.parent().unwrap_or(Path::new("."));
    load_env_files(env_dir, false, cli.verbose);

    if std::env::var("WEBPIPE_PUBLIC_DIR").is_err() {
        let public_path = env_dir.join("public");
        if let Some(s) = public_path.to_str() {
            std::env::set_var("WEBPIPE_PUBLIC_DIR", s);
        }
    }

    let server = match WebPipeServer::from_program(
        program,
        Some(file_path.to_path_buf()),
        cli.trace,
    )
    .await
    {
        Ok(server) => server,
        Err(e) => {
            eprintln!("{}", e.display_pipeline_details());
            std::process::exit(1);
        }
    };

    match server.execute_main_pipeline(serde_json::json!({})).await {
        Ok((result, content_type, status_code)) => {
            write_script_output(&result, &content_type)?;
            if status_code.unwrap_or(200) >= 400 {
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("{}", e.display_pipeline_details());
            std::process::exit(1);
        }
    }
}

fn write_script_output(result: &Value, content_type: &str) -> Result<(), Box<dyn Error>> {
    use std::io::Write;

    let mut stdout = std::io::stdout().lock();
    if content_type.starts_with("text/html") || content_type.starts_with("image/svg+xml") {
        let body = result.as_str()
            .map(str::to_string)
            .unwrap_or_else(|| serde_json::to_string(result).unwrap_or_default());
        stdout.write_all(body.as_bytes())?;
        if !body.ends_with('\n') {
            stdout.write_all(b"\n")?;
        }
    } else if content_type.starts_with("image/png") {
        let body = result.as_str().unwrap_or("");
        match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, body) {
            Ok(binary) => stdout.write_all(&binary)?,
            Err(_) => stdout.write_all(body.as_bytes())?,
        }
    } else {
        serde_json::to_writer_pretty(&mut stdout, result)?;
        stdout.write_all(b"\n")?;
    }
    stdout.flush()?;
    Ok(())
}

async fn serve_mode(file_path: &Path, cli: &cli::Cli) -> Result<(), Box<dyn Error>> {
    let trace_mode = cli.trace;

    // Determine the directory for .env files (same directory as the WebPipe file)
    let env_dir = file_path.parent().unwrap_or(Path::new("."));

    // Initial load of .env files (do not override already-set process vars)
    load_env_files(env_dir, false, cli.verbose);

    // Point public dir to <webpipe_dir>/public if not explicitly set
    if std::env::var("WEBPIPE_PUBLIC_DIR").is_err() {
        let public_path = env_dir.join("public");
        if let Some(s) = public_path.to_str() {
            std::env::set_var("WEBPIPE_PUBLIC_DIR", s);
        }
    }

    let default_addr = "127.0.0.1:7770".to_string();

    // Determine address from CLI options or environment
    let addr_str = if let Some(port) = cli.port {
        format!("127.0.0.1:{}", port)
    } else if let Some(ref addr) = cli.address {
        addr.clone()
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

    // Set up file watcher (will be updated each reload with all imported files)
    let mut _watcher: Option<RecommendedWatcher> = None;

    loop {
        // Reload .env files each iteration so edits take effect on hot-reload
        load_env_files(env_dir, true, false);

        // Parse the WebPipe file
        let input = std::fs::read_to_string(file_path)?;
        let (_leftover_input, program) = parse_program(&input)
            .map_err(|e| format!("Parse error: {}", e))?;

        // Create and start the server with a shutdown signal
        let server = match WebPipeServer::from_program(program, Some(file_path.to_path_buf()), trace_mode).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Failed to build server: {}", e);
                std::process::exit(1);
            }
        };

        // Update file watcher to watch all imported modules
        let raw_tx_clone = raw_tx.clone();
        _watcher = {
            let mut w: RecommendedWatcher = notify::recommended_watcher(move |res| {
                let _ = raw_tx_clone.send(res);
            })?;
            // Watch the main file
            w.watch(file_path, RecursiveMode::NonRecursive)?;
            // Watch all imported module files
            for module_path in server.module_paths() {
                if let Err(e) = w.watch(module_path, RecursiveMode::NonRecursive) {
                    eprintln!("Warning: Failed to watch imported file {}: {}", module_path.display(), e);
                }
            }
            // Also watch the directory for .env changes
            let _ = w.watch(env_dir, RecursiveMode::NonRecursive);
            Some(w)
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

    Ok(())
}

async fn test_mode(file_path: &Path, cli: &cli::Cli) -> Result<(), Box<dyn Error>> {
    let verbose_mode = cli.verbose;

    // Determine the directory for .env files (same directory as the WebPipe file)
    let env_dir = file_path.parent().unwrap_or(Path::new("."));

    // Load .env files
    load_env_files(env_dir, false, verbose_mode);

    // Parse the WebPipe file
    let input = std::fs::read_to_string(file_path)?;
    let (_leftover_input, program) = parse_program(&input)
        .map_err(|e| format!("Parse error: {}", e))?;

    // Run tests once and exit
    match run_tests(program, Some(file_path.to_path_buf()), verbose_mode).await {
        Ok(summary) => {
            println!("Test Results: {}/{} passed ({} failed)", summary.passed, summary.total, summary.failed);
            for o in summary.outcomes {
                if o.passed {
                    println!("[PASS] {} :: {}", o.describe, o.test);
                } else {
                    println!("[FAIL] {} :: {} -> {}", o.describe, o.test, o.message);
                }
            }
            if summary.failed > 0 {
                std::process::exit(1);
            } else {
                std::process::exit(0);
            }
        }
        Err(e) => {
            eprintln!("Test runner error: {}", e);
            std::process::exit(1);
        }
    }
}

async fn inspect_mode(file_path: &Path, cli: &cli::Cli) -> Result<(), Box<dyn Error>> {
    let trace_mode = cli.trace;
    let inspect_port = cli.inspect_port;

    // Determine the directory for .env files
    let env_dir = file_path.parent().unwrap_or(Path::new("."));
    load_env_files(env_dir, false, cli.verbose);

    if std::env::var("WEBPIPE_PUBLIC_DIR").is_err() {
        let public_path = env_dir.join("public");
        if let Some(s) = public_path.to_str() {
            std::env::set_var("WEBPIPE_PUBLIC_DIR", s);
        }
    }

    // Parse the WebPipe file
    let input = std::fs::read_to_string(file_path)?;
    let (_leftover_input, program) = parse_program(&input)
        .map_err(|e| format!("Parse error: {}", e))?;

    if inspect_runs_standalone_script(&program, cli) {
        init_tracing(SCRIPT_LOG_FILTER);
        return run_script_inspect_mode(file_path, program, inspect_port, trace_mode).await;
    }

    init_tracing(DEFAULT_LOG_FILTER);

    // Determine address
    let default_addr = "127.0.0.1:7770".to_string();
    let addr_str = if let Some(port) = cli.port {
        format!("127.0.0.1:{}", port)
    } else if let Some(ref addr) = cli.address {
        addr.clone()
    } else if let Ok(port) = std::env::var("PORT") {
        format!("0.0.0.0:{}", port)
    } else {
        default_addr.clone()
    };
    let addr: SocketAddr = addr_str.parse()?;

    run_inspect_mode(file_path, program, addr, inspect_port, trace_mode).await
}

fn inspect_runs_standalone_script(program: &webpipe::ast::Program, cli: &cli::Cli) -> bool {
    !cli.serve && program.pipelines.iter().any(|pipeline| pipeline.name == "main")
}

async fn run_script_inspect_mode(
    file_path: &Path,
    program: webpipe::ast::Program,
    inspect_port: u16,
    trace_mode: bool,
) -> Result<(), Box<dyn Error>> {
    // Convert to absolute path for breakpoint matching
    let absolute_path = std::fs::canonicalize(file_path)
        .map_err(|e| format!("Failed to resolve absolute path for {}: {}", file_path.display(), e))?;
    let file_path_str = absolute_path.to_str()
        .ok_or_else(|| format!("Path contains invalid UTF-8: {:?}", absolute_path))?
        .to_string();

    eprintln!("=================================");
    eprintln!("WebPipe Script Debugger");
    eprintln!("=================================");
    eprintln!("File:        {}", file_path_str);
    eprintln!("Debug Port:  {}", inspect_port);
    eprintln!("=================================");
    eprintln!();

    // Create shutdown notifier
    let shutdown_notify = Arc::new(Notify::new());

    // Create DAP server
    let dap_server = Arc::new(DapServer::new(
        file_path_str,
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

    eprintln!("[DAP] Waiting for debugger to connect...");
    eprintln!("  - Press F5 in VS Code to attach debugger");
    eprintln!("  - Set breakpoints in your .wp file");
    eprintln!();

    // The script is one-shot, so wait until the client has sent breakpoints.
    dap_server.wait_for_configuration_done().await;

    eprintln!("[DAP] Debugger connected, running main pipeline...");
    eprintln!();

    // Create debugger hook
    let debugger_hook = Arc::new(DapDebuggerHook { server: dap_server });

    let server = match WebPipeServer::from_program_with_debugger(
        program,
        Some(absolute_path),
        trace_mode,
        Some(debugger_hook)
    ).await
    {
        Ok(server) => server,
        Err(e) => {
            eprintln!("{}", e.display_pipeline_details());
            shutdown_notify.notify_one();
            std::process::exit(1);
        }
    };

    match server.execute_main_pipeline(serde_json::json!({})).await {
        Ok((result, content_type, status_code)) => {
            write_script_output(&result, &content_type)?;
            shutdown_notify.notify_one();
            if status_code.unwrap_or(200) >= 400 {
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("{}", e.display_pipeline_details());
            shutdown_notify.notify_one();
            std::process::exit(1);
        }
    }
}

async fn run_inspect_mode(
    file_path: &Path,
    program: webpipe::ast::Program,
    addr: SocketAddr,
    inspect_port: u16,
    trace_mode: bool,
) -> Result<(), Box<dyn Error>> {

    // Convert to absolute path for breakpoint matching
    let absolute_path = std::fs::canonicalize(file_path)
        .map_err(|e| format!("Failed to resolve absolute path for {}: {}", file_path.display(), e))?;
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
        Some(absolute_path),
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

async fn test_inspect_mode(file_path: &Path, cli: &cli::Cli) -> Result<(), Box<dyn Error>> {
    let verbose_mode = cli.verbose;
    let inspect_port = cli.inspect_port;

    // Determine the directory for .env files (same directory as the WebPipe file)
    let env_dir = file_path.parent().unwrap_or(Path::new("."));

    // Load .env files
    load_env_files(env_dir, false, verbose_mode);

    // Parse the WebPipe file
    let input = std::fs::read_to_string(file_path)?;
    let (_leftover_input, program) = parse_program(&input)
        .map_err(|e| format!("Parse error: {}", e))?;

    // Convert to absolute path for breakpoint matching
    let absolute_path = std::fs::canonicalize(file_path)
        .map_err(|e| format!("Failed to resolve absolute path for {}: {}", file_path.display(), e))?;
    let file_path_str = absolute_path.to_str()
        .ok_or_else(|| format!("Path contains invalid UTF-8: {:?}", absolute_path))?
        .to_string();

    eprintln!("=================================");
    eprintln!("WebPipe Test Debugger");
    eprintln!("=================================");
    eprintln!("File:        {}", file_path_str);
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

    eprintln!("[DAP] Waiting for debugger to connect...");
    eprintln!("  • Press F5 in VS Code to attach debugger");
    eprintln!("  • Set breakpoints in your .wp file");
    eprintln!();

    // Wait for VS Code to connect and send configurationDone
    dap_server.wait_for_configuration_done().await;

    eprintln!("[DAP] Debugger connected, starting tests...");
    eprintln!();

    // Create debugger hook
    let debugger_hook = Arc::new(DapDebuggerHook { server: dap_server });

    // Run tests with debugger
    match run_tests_with_debugger(program, Some(absolute_path), verbose_mode, Some(debugger_hook)).await {
        Ok(summary) => {
            println!();
            println!("Test Results: {}/{} passed ({} failed)", summary.passed, summary.total, summary.failed);
            for o in summary.outcomes {
                if o.passed {
                    println!("[PASS] {} :: {}", o.describe, o.test);
                } else {
                    println!("[FAIL] {} :: {} -> {}", o.describe, o.test, o.message);
                }
            }

            // Signal shutdown
            shutdown_notify.notify_one();

            if summary.failed > 0 {
                std::process::exit(1);
            } else {
                std::process::exit(0);
            }
        }
        Err(e) => {
            eprintln!("Test runner error: {}", e);
            shutdown_notify.notify_one();
            std::process::exit(1);
        }
    }
}
