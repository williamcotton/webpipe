use std::{env, error::Error, net::SocketAddr, path::Path, sync::mpsc as std_mpsc, time::Duration};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::{sync::{mpsc as tokio_mpsc, oneshot}, signal};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use webpipe::{ast::parse_program, WebPipeServer, run_tests};

fn load_env_files(env_dir: &Path, override_existing: bool) {
    // Load .env files located next to the WebPipe file
    let files = [".env", ".env.local"];
    for fname in files.iter() {
        let p = env_dir.join(fname);
        if p.exists() {
            if override_existing {
                let _ = dotenvy::from_filename_override(&p);
            } else {
                let _ = dotenvy::from_filename(&p);
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
        eprintln!("Usage: {} <webpipe_file> [host:port|--test]", args[0]);
        std::process::exit(1);
    }

    let file_path = &args[1];
    // Determine if test mode
    let test_mode = args.iter().any(|a| a == "--test");
    let default_addr = "127.0.0.1:8090".to_string();
    let addr_str = args.get(2).filter(|v| v.as_str() != "--test").unwrap_or(&default_addr);
    
    // Parse the address
    let addr: SocketAddr = addr_str.parse()?;

    // Determine the directory for .env files (same directory as the WebPipe file)
    let env_dir = Path::new(file_path).parent().unwrap_or(Path::new("."));

    // Point public dir to <webpipe_dir>/public if not explicitly set
    if std::env::var("WEBPIPE_PUBLIC_DIR").is_err() {
        let public_path = env_dir.join("public");
        if let Some(s) = public_path.to_str() {
            std::env::set_var("WEBPIPE_PUBLIC_DIR", s);
        }
    }

    // Initial load of .env files (do not override already-set process vars)
    load_env_files(env_dir, false);

    // File change notification: bridge notify (std thread) -> tokio
    let (raw_tx, raw_rx): (std_mpsc::Sender<notify::Result<notify::Event>>, std_mpsc::Receiver<notify::Result<notify::Event>>) = std_mpsc::channel();
    let (change_tx, mut change_rx) = tokio_mpsc::unbounded_channel::<()>();

    // Spawn a blocking thread to forward notify events into tokio channel
    std::thread::spawn(move || {
        while let Ok(res) = raw_rx.recv() {
            if res.is_ok() { let _ = change_tx.send(()); }
        }
    });

    // Set up watcher (only for serve mode)
    let mut watcher: Option<RecommendedWatcher> = None;
    if !test_mode {
        let mut w: RecommendedWatcher = notify::recommended_watcher(move |res| {
            let _ = raw_tx.send(res);
        })?;
        w.watch(Path::new(file_path), RecursiveMode::NonRecursive)?;
        // Also watch the directory for .env changes (creation/modification)
        let _ = w.watch(env_dir, RecursiveMode::NonRecursive);
        watcher = Some(w);
    }

    loop {
        // Reload .env files each iteration so edits take effect on hot-reload
        load_env_files(env_dir, true);

        // Parse the WebPipe file
        let input = std::fs::read_to_string(file_path)?;
        let (_leftover_input, program) = parse_program(&input)
            .map_err(|e| format!("Parse error: {}", e))?;

        println!("Parsed WebPipe program:");
        println!("{}", program);

        if test_mode {
            // Run tests once and exit
            match run_tests(program).await {
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
        } else {
            // Create and start the server with a shutdown signal
            let server = WebPipeServer::from_program(program);
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