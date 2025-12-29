use crate::cli::{Cli, Commands};
use std::error::Error;
use std::fs;
use std::path::Path;
use std::io::Write;
use chrono::Utc;

pub async fn handle_new(cli: &Cli) -> Result<(), Box<dyn Error>> {
    let (name, here, with_migrations) = match &cli.command {
        Some(Commands::New { name, here, with_migrations }) => {
            (name, *here, *with_migrations)
        }
        _ => unreachable!(),
    };

    let project_dir = if here {
        std::env::current_dir()?
    } else {
        let dir = Path::new(name);
        if dir.exists() {
            return Err(format!("Directory '{}' already exists", name).into());
        }
        fs::create_dir(dir)?;
        dir.to_path_buf()
    };

    println!("Creating WebPipe project: {}", name);

    // Create subdirectories
    fs::create_dir_all(project_dir.join("public"))?;
    if with_migrations {
        fs::create_dir_all(project_dir.join("migrations"))?;
    }

    // Generate files
    generate_app_wp(&project_dir, name)?;
    generate_env_example(&project_dir)?;
    generate_gitignore(&project_dir)?;
    generate_readme(&project_dir, name)?;

    if with_migrations {
        generate_initial_migration(&project_dir)?;
    }

    println!("\n✓ Project created successfully!");
    println!("\nNext steps:");
    if !here {
        println!("  cd {}", name);
    }
    println!("  # Edit .env.example and save as .env.local");
    println!("  wp app.wp");
    if with_migrations {
        println!("  wp migrate up");
    }

    Ok(())
}

fn generate_app_wp(dir: &Path, name: &str) -> std::io::Result<()> {
    let content = include_str!("templates/app.wp.template");
    let content = content.replace("{{PROJECT_NAME}}", name);

    let mut file = fs::File::create(dir.join("app.wp"))?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

fn generate_env_example(dir: &Path) -> std::io::Result<()> {
    let content = include_str!("templates/env.example.template");
    let mut file = fs::File::create(dir.join(".env.example"))?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

fn generate_gitignore(dir: &Path) -> std::io::Result<()> {
    let content = include_str!("templates/gitignore.template");
    let mut file = fs::File::create(dir.join(".gitignore"))?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

fn generate_readme(dir: &Path, name: &str) -> std::io::Result<()> {
    let content = format!(
        "# {}\n\nA WebPipe API project.\n\n## Getting Started\n\n\
         1. Copy `.env.example` to `.env.local` and configure\n\
         2. Run the server: `wp app.wp`\n\
         3. Visit http://127.0.0.1:7770\n\n\
         ## Running Tests\n\n\
         ```bash\nwp app.wp --test\n```\n",
        name
    );
    let mut file = fs::File::create(dir.join("README.md"))?;
    file.write_all(content.as_bytes())?;
    Ok(())
}

fn generate_initial_migration(dir: &Path) -> std::io::Result<()> {
    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let filename = format!("{}_create_users_table.sql", timestamp);

    let content = r#"-- UP --
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    login TEXT UNIQUE NOT NULL,
    email TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    status TEXT DEFAULT 'active',
    created_at TIMESTAMP DEFAULT NOW()
);

CREATE INDEX idx_users_login ON users(login);

-- DOWN --
DROP TABLE IF EXISTS users;
"#;

    let mut file = fs::File::create(dir.join("migrations").join(&filename))?;
    file.write_all(content.as_bytes())?;
    println!("  Generated migration: {}", filename);
    Ok(())
}
