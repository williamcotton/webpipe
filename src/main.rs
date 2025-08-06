use std::{env, error::Error, net::SocketAddr};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use webpipe::{ast::parse_program, WebPipeServer};

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
        eprintln!("Usage: {} <webpipe_file> [host:port]", args[0]);
        std::process::exit(1);
    }

    let file_path = &args[1];
    let default_addr = "127.0.0.1:8090".to_string();
    let addr_str = args.get(2).unwrap_or(&default_addr);
    
    // Parse the WebPipe file
    let input = std::fs::read_to_string(file_path)?;
    let (_leftover_input, program) = parse_program(&input)
        .map_err(|e| format!("Parse error: {}", e))?;

    println!("Parsed WebPipe program:");
    println!("{}", program);
    
    // Parse the address
    let addr: SocketAddr = addr_str.parse()?;
    
    // Create and start the server
    let server = WebPipeServer::from_program(program);
    server.serve(addr).await?;

    Ok(())
}