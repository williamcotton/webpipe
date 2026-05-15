use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "webpipe")]
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

    #[arg(long, help = "Run root pipeline main as a script")]
    pub run: bool,

    #[arg(long, help = "Serve HTTP routes even if the file defines pipeline main")]
    pub serve: bool,

    #[arg(long, help = "Verbose test output")]
    pub verbose: bool,

    #[arg(long, help = "Enable trace mode")]
    pub trace: bool,

    #[arg(long, help = "Run in debug mode")]
    pub inspect: bool,

    #[arg(long, help = "Debug server port", default_value = "5858", value_name = "PORT")]
    pub inspect_port: u16,

    /// Everything after `--` is forwarded to the script as $context.args / $context.options.
    /// `last = true` lets clap capture it without claiming any of webpipe's own flags.
    #[arg(last = true, value_name = "SCRIPT_ARGS")]
    pub script_args: Vec<String>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Initialize a WebPipe project
    Init {
        /// Directory to initialize (defaults to current directory)
        directory: Option<String>,

        #[arg(long, help = "Generate CLAUDE.md with WebPipe language reference")]
        claude: bool,

        #[arg(long, help = "Generate AGENTS.md with WebPipe language reference")]
        codex: bool,

        #[arg(long, help = "Generate GEMINI.md with WebPipe language reference")]
        gemini: bool,
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
            Some(Commands::Init { .. }) => OperationMode::Init,
            Some(Commands::Migrate { .. }) => OperationMode::Migrate,
            None => {
                if let Some(file) = &self.file {
                    // Check for combined flags first
                    if self.test && self.inspect {
                        return OperationMode::TestInspect(file.clone());
                    }

                    if self.inspect {
                        return OperationMode::Inspect(file.clone());
                    }

                    if self.test {
                        OperationMode::Test(file.clone())
                    } else if self.serve || self.port.is_some() || self.address.is_some() {
                        OperationMode::Serve(file.clone())
                    } else if self.run {
                        OperationMode::Run(file.clone())
                    } else {
                        OperationMode::Auto(file.clone())
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
    Init,
    Migrate,
    Auto(PathBuf),
    Run(PathBuf),
    Serve(PathBuf),
    Test(PathBuf),
    Inspect(PathBuf),
    TestInspect(PathBuf),
    Help,
}
