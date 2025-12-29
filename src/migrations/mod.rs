use crate::cli::{Cli, Commands, MigrateAction};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use chrono::Utc;

pub async fn handle_migrate(cli: &Cli) -> Result<(), Box<dyn Error>> {
    let action = match &cli.command {
        Some(Commands::Migrate { action }) => action,
        _ => unreachable!(),
    };

    match action {
        MigrateAction::Create { name } => create_migration(name),
        MigrateAction::Up { steps } => migrate_up(*steps).await,
        MigrateAction::Down { steps } => migrate_down(*steps).await,
        MigrateAction::Status => migration_status().await,
        MigrateAction::Force { version } => force_version(version).await,
    }
}

// Database connection from environment variables
async fn connect_db() -> Result<PgPool, Box<dyn Error>> {
    let host = std::env::var("WP_PG_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port = std::env::var("WP_PG_PORT").unwrap_or_else(|_| "5432".to_string());
    let database = std::env::var("WP_PG_DATABASE")
        .map_err(|_| "WP_PG_DATABASE environment variable required")?;
    let user = std::env::var("WP_PG_USER").unwrap_or_else(|_| "postgres".to_string());
    let password = std::env::var("WP_PG_PASSWORD").unwrap_or_else(|_| "".to_string());

    let database_url = format!(
        "postgres://{}:{}@{}:{}/{}",
        percent_encoding::utf8_percent_encode(&user, percent_encoding::NON_ALPHANUMERIC),
        percent_encoding::utf8_percent_encode(&password, percent_encoding::NON_ALPHANUMERIC),
        host,
        port,
        database
    );

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await
        .map_err(|e| format!("Failed to connect to database: {}", e))?;

    Ok(pool)
}

// Initialize migration tracking table
async fn ensure_migration_table(pool: &PgPool) -> Result<(), Box<dyn Error>> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS _wp_migrations (
            id SERIAL PRIMARY KEY,
            version VARCHAR(14) UNIQUE NOT NULL,
            name TEXT NOT NULL,
            applied_at TIMESTAMP DEFAULT NOW()
        )
        "#
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_wp_migrations_version ON _wp_migrations(version)")
        .execute(pool)
        .await?;

    Ok(())
}

fn create_migration(name: &str) -> Result<(), Box<dyn Error>> {
    let migrations_dir = Path::new("migrations");
    if !migrations_dir.exists() {
        fs::create_dir(migrations_dir)?;
    }

    let timestamp = Utc::now().format("%Y%m%d%H%M%S");
    let filename = format!("{}_{}.sql", timestamp, name);
    let filepath = migrations_dir.join(&filename);

    let template = r#"-- UP --


-- DOWN --

"#;

    fs::write(&filepath, template)?;
    println!("Created migration: {}", filename);
    println!("Edit: {}", filepath.display());

    Ok(())
}

async fn migrate_up(steps: Option<usize>) -> Result<(), Box<dyn Error>> {
    // Load .env files
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_filename(".env.local");

    let pool = connect_db().await?;
    ensure_migration_table(&pool).await?;

    let pending = get_pending_migrations(&pool).await?;

    if pending.is_empty() {
        println!("No pending migrations");
        return Ok(());
    }

    let to_apply = match steps {
        Some(n) => &pending[..n.min(pending.len())],
        None => &pending[..],
    };

    println!("Running {} migration(s)...", to_apply.len());

    for migration in to_apply {
        print!("  Applying {} ... ", migration.filename);
        apply_migration(&pool, migration).await?;
        println!("✓");
    }

    println!("\n✓ Migrations applied successfully");
    Ok(())
}

async fn migrate_down(steps: usize) -> Result<(), Box<dyn Error>> {
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_filename(".env.local");

    let pool = connect_db().await?;
    ensure_migration_table(&pool).await?;

    let applied = get_applied_migrations(&pool).await?;

    if applied.is_empty() {
        println!("No migrations to rollback");
        return Ok(());
    }

    let to_rollback = &applied[..steps.min(applied.len())];

    println!("Rolling back {} migration(s)...", to_rollback.len());

    for migration in to_rollback {
        print!("  Rolling back {} ... ", migration.filename);
        rollback_migration(&pool, migration).await?;
        println!("✓");
    }

    println!("\n✓ Migrations rolled back successfully");
    Ok(())
}

async fn migration_status() -> Result<(), Box<dyn Error>> {
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_filename(".env.local");

    let pool = connect_db().await?;
    ensure_migration_table(&pool).await?;

    let all_files = scan_migration_files()?;
    let applied_versions: Vec<String> = sqlx::query_scalar(
        "SELECT version FROM _wp_migrations ORDER BY version"
    )
    .fetch_all(&pool)
    .await?;

    println!("\nMigration Status:\n");
    println!("{:<20} {:<40} {:<10}", "VERSION", "NAME", "STATUS");
    println!("{}", "-".repeat(70));

    for migration in all_files {
        let status = if applied_versions.contains(&migration.version) {
            "✓ Applied"
        } else {
            "  Pending"
        };
        println!("{:<20} {:<40} {:<10}", migration.version, migration.name, status);
    }

    Ok(())
}

async fn force_version(version: &str) -> Result<(), Box<dyn Error>> {
    let _ = dotenvy::dotenv();
    let _ = dotenvy::from_filename(".env.local");

    println!("WARNING: Force-setting migration version is dangerous!");
    println!("This will mark the migration as applied WITHOUT running it.");
    print!("Are you sure? (yes/no): ");

    use std::io::{self, BufRead};
    let stdin = io::stdin();
    let mut input = String::new();
    stdin.lock().read_line(&mut input)?;

    if input.trim().to_lowercase() != "yes" {
        println!("Aborted");
        return Ok(());
    }

    let pool = connect_db().await?;
    ensure_migration_table(&pool).await?;

    // Find migration file
    let migration = scan_migration_files()?
        .into_iter()
        .find(|m| m.version == version)
        .ok_or_else(|| format!("Migration version {} not found", version))?;

    sqlx::query(
        "INSERT INTO _wp_migrations (version, name) VALUES ($1, $2)
         ON CONFLICT (version) DO NOTHING"
    )
    .bind(&migration.version)
    .bind(&migration.name)
    .execute(&pool)
    .await?;

    println!("✓ Forced version {}", version);
    Ok(())
}

#[derive(Debug)]
struct Migration {
    version: String,    // YYYYMMDDHHMMSS
    name: String,       // e.g., "create_users_table"
    filename: String,   // Full filename
    filepath: PathBuf,
    up_sql: String,
    down_sql: String,
}

fn scan_migration_files() -> Result<Vec<Migration>, Box<dyn Error>> {
    let migrations_dir = Path::new("migrations");

    if !migrations_dir.exists() {
        return Ok(Vec::new());
    }

    let mut migrations = Vec::new();

    for entry in fs::read_dir(migrations_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() || path.extension().and_then(|s| s.to_str()) != Some("sql") {
            continue;
        }

        let filename = path.file_name()
            .and_then(|s| s.to_str())
            .ok_or("Invalid filename")?
            .to_string();

        // Parse filename: YYYYMMDDHHMMSS_name.sql
        let parts: Vec<&str> = filename.splitn(2, '_').collect();
        if parts.len() != 2 {
            eprintln!("Warning: Skipping invalid migration filename: {}", filename);
            continue;
        }

        let version = parts[0].to_string();
        let name = parts[1].trim_end_matches(".sql").to_string();

        // Validate version format
        if version.len() != 14 || !version.chars().all(|c| c.is_numeric()) {
            eprintln!("Warning: Invalid version format in {}", filename);
            continue;
        }

        let content = fs::read_to_string(&path)?;
        let (up_sql, down_sql) = parse_migration_content(&content)?;

        migrations.push(Migration {
            version,
            name,
            filename,
            filepath: path,
            up_sql,
            down_sql,
        });
    }

    // Sort by version
    migrations.sort_by(|a, b| a.version.cmp(&b.version));

    Ok(migrations)
}

fn parse_migration_content(content: &str) -> Result<(String, String), Box<dyn Error>> {
    let up_marker = "-- UP --";
    let down_marker = "-- DOWN --";

    let content_upper = content.to_uppercase();

    let up_start = content_upper.find("-- UP --")
        .ok_or("Missing -- UP -- marker")?;
    let down_start = content_upper.find("-- DOWN --")
        .ok_or("Missing -- DOWN -- marker")?;

    if down_start <= up_start {
        return Err("-- DOWN -- must come after -- UP --".into());
    }

    let up_sql = content[up_start + up_marker.len()..down_start]
        .trim()
        .to_string();
    let down_sql = content[down_start + down_marker.len()..]
        .trim()
        .to_string();

    Ok((up_sql, down_sql))
}

async fn get_pending_migrations(pool: &PgPool) -> Result<Vec<Migration>, Box<dyn Error>> {
    let all_migrations = scan_migration_files()?;
    let applied_versions: Vec<String> = sqlx::query_scalar(
        "SELECT version FROM _wp_migrations ORDER BY version"
    )
    .fetch_all(pool)
    .await?;

    Ok(all_migrations
        .into_iter()
        .filter(|m| !applied_versions.contains(&m.version))
        .collect())
}

async fn get_applied_migrations(pool: &PgPool) -> Result<Vec<Migration>, Box<dyn Error>> {
    let all_migrations = scan_migration_files()?;
    let applied_versions: Vec<String> = sqlx::query_scalar(
        "SELECT version FROM _wp_migrations ORDER BY version DESC"
    )
    .fetch_all(pool)
    .await?;

    let mut applied_migrations: Vec<Migration> = all_migrations
        .into_iter()
        .filter(|m| applied_versions.contains(&m.version))
        .collect();

    // Sort in reverse order (most recent first) for rollback
    applied_migrations.sort_by(|a, b| b.version.cmp(&a.version));

    Ok(applied_migrations)
}

async fn apply_migration(pool: &PgPool, migration: &Migration) -> Result<(), Box<dyn Error>> {
    // Execute UP migration in a transaction
    let mut tx = pool.begin().await?;

    sqlx::query(&migration.up_sql)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Migration failed: {}", e))?;

    sqlx::query(
        "INSERT INTO _wp_migrations (version, name) VALUES ($1, $2)"
    )
    .bind(&migration.version)
    .bind(&migration.name)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

async fn rollback_migration(pool: &PgPool, migration: &Migration) -> Result<(), Box<dyn Error>> {
    // Execute DOWN migration in a transaction
    let mut tx = pool.begin().await?;

    sqlx::query(&migration.down_sql)
        .execute(&mut *tx)
        .await
        .map_err(|e| format!("Rollback failed: {}", e))?;

    sqlx::query(
        "DELETE FROM _wp_migrations WHERE version = $1"
    )
    .bind(&migration.version)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}
