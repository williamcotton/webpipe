use crate::cli::{Cli, Commands};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::io::Write;

pub fn handle_init(cli: &Cli) -> Result<(), Box<dyn Error>> {
    let (directory, claude, codex, gemini) = match &cli.command {
        Some(Commands::Init { directory, claude, codex, gemini }) => {
            (directory, *claude, *codex, *gemini)
        }
        _ => unreachable!(),
    };

    let dir = match directory {
        Some(d) => {
            let path = Path::new(d);
            if !path.exists() {
                fs::create_dir_all(path)?;
            }
            path.to_path_buf()
        }
        None => std::env::current_dir()?,
    };

    println!("Initializing WebPipe project in {}", dir.display());

    // Always create app.wp
    let app_path = dir.join("app.wp");
    if app_path.exists() {
        println!("  app.wp already exists, skipping");
    } else {
        let content = include_str!("templates/app.wp.template");
        let mut file = fs::File::create(&app_path)?;
        file.write_all(content.as_bytes())?;
        println!("  Created app.wp");
    }

    let llm_content = include_str!("templates/LLM.md.template");
    if claude {
        generate_ai_file(&dir, "CLAUDE.md", llm_content)?;
    }
    if codex {
        generate_ai_file(&dir, "AGENTS.md", llm_content)?;
    }
    if gemini {
        generate_ai_file(&dir, "GEMINI.md", llm_content)?;
    }

    println!("\n✓ Initialized successfully!");
    println!("\nNext steps:");
    println!("  webpipe app.wp");
    println!("  webpipe app.wp --test");

    Ok(())
}

fn generate_ai_file(dir: &Path, filename: &str, content: &str) -> Result<(), Box<dyn Error>> {
    let path = dir.join(filename);
    if path.exists() {
        println!("  {} already exists, skipping", filename);
    } else {
        let mut file = fs::File::create(&path)?;
        file.write_all(content.as_bytes())?;
        println!("  Created {}", filename);
    }
    Ok(())
}
