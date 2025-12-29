use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "wp")]
#[command(version, about = "WebPipe - Pipeline-based web API runtime", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    // Legacy positional argument for backwards compatibility
    #[arg(value_name = "FILE", help = "WebPipe file to run")]
    pub file: Option<PathBuf>,

    // Server options (only used when FILE is provided)
    #[arg(long, help = "Host:port to bind (e.g., 127.0.0.1:7770)", value_name = "ADDR")]
    pub address: Option<String>,

    #[arg(long, help = "Port to bind (shorthand for 127.0.0.1:PORT)", value_name = "PORT")]
    pub port: Option<u16>,

    #[arg(long, help = "Run BDD tests")]
    pub test: bool,

    #[arg(long, help = "Verbose test output")]
    pub verbose: bool,

    #[arg(long, help = "Enable trace mode")]
    pub trace: bool,

    #[cfg(feature = "debugger")]
    #[arg(long, help = "Run in debug mode")]
    pub inspect: bool,

    #[cfg(feature = "debugger")]
    #[arg(long, help = "Debug server port", default_value = "5858", value_name = "PORT")]
    pub inspect_port: u16,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Create a new WebPipe project
    New {
        /// Project name
        name: String,

        #[arg(long, help = "Create in current directory instead of new subdirectory")]
        here: bool,

        #[arg(long, help = "Include example migration for users table")]
        with_migrations: bool,
    },

    /// Database migration management
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum MigrateAction {
    /// Create a new migration file
    Create {
        /// Migration name (e.g., "create_users_table")
        name: String,
    },

    /// Run pending migrations
    Up {
        #[arg(short = 'n', long, help = "Number of migrations to run (default: all)")]
        steps: Option<usize>,
    },

    /// Rollback migrations
    Down {
        #[arg(short = 'n', long, help = "Number of migrations to rollback", default_value = "1")]
        steps: usize,
    },

    /// Show migration status
    Status,

    /// Force set migration version (dangerous)
    Force {
        /// Migration version (e.g., "20231215120000")
        version: String,
    },
}

impl Cli {
    /// Determine the operational mode based on CLI arguments
    pub fn mode(&self) -> OperationMode {
        match &self.command {
            Some(Commands::New { .. }) => OperationMode::Scaffold,
            Some(Commands::Migrate { .. }) => OperationMode::Migrate,
            None => {
                if let Some(file) = &self.file {
                    #[cfg(feature = "debugger")]
                    if self.inspect {
                        return OperationMode::Inspect(file.clone());
                    }

                    if self.test {
                        OperationMode::Test(file.clone())
                    } else {
                        OperationMode::Serve(file.clone())
                    }
                } else {
                    OperationMode::Help
                }
            }
        }
    }
}

#[derive(Debug)]
pub enum OperationMode {
    Scaffold,
    Migrate,
    Serve(PathBuf),
    Test(PathBuf),
    #[cfg(feature = "debugger")]
    Inspect(PathBuf),
    Help,
}
