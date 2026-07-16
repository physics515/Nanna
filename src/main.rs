#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Nanna - High-performance AI assistant in Rust.
//!
//! Moon god of the digital realm.
//! Built with SIMD and GPU acceleration for unrelenting performance.

mod commands;
mod onboarding;
mod setup;

use clap::{Parser, Subcommand};
use commands::cli::{list_sessions, run_cli, run_once};
use commands::credentials::handle_credentials_command;
use commands::daemon::handle_daemon_command;
use commands::serve::{run_daemon, run_server};
use commands::workspace::handle_workspace_command;
use nanna_config::Config;
use setup::ensure_api_key;
use tracing::{info, Level};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "nanna")]
#[command(author, version, about = "High-performance AI assistant", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Config file path
    #[arg(short, long)]
    config: Option<String>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(short, long, default_value = "info")]
    log_level: String,

    /// Run in daemon mode (background service)
    #[arg(long, hide = true)]
    daemon_mode: bool,

    /// Daemon host
    #[arg(long, hide = true, default_value = "127.0.0.1")]
    host: String,

    /// Daemon port
    #[arg(long, hide = true, default_value = "9999")]
    port: u16,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Nanna with setup wizard
    Init,

    /// Show configuration status
    Status,

    /// Start the HTTP server
    Server {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "0.0.0.0")]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },

    /// Daemon management (always-on background service)
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },

    /// Interactive CLI mode
    Chat {
        /// Session ID to resume
        #[arg(short, long)]
        session: Option<String>,

        /// Model to use
        #[arg(short, long)]
        model: Option<String>,

        /// Stream responses (print as they arrive)
        #[arg(long)]
        stream: bool,
    },

    /// List recent sessions
    Sessions {
        /// Number of sessions to show
        #[arg(short, long, default_value = "10")]
        limit: i64,
    },

    /// Run a single prompt and exit
    Run {
        /// The prompt to run
        prompt: String,

        /// Model to use
        #[arg(short, long)]
        model: Option<String>,
    },

    /// Show or generate configuration
    Config {
        /// Generate default config
        #[arg(long)]
        generate: bool,
    },

    /// Workspace management
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },

    /// Manage Claude CLI credentials (OAuth)
    Credentials {
        #[command(subcommand)]
        action: CredentialsAction,
    },
}

#[derive(Subcommand)]
enum WorkspaceAction {
    /// Initialize a new workspace in the current directory
    Init {
        /// Template to use (minimal, standard, project, assistant, research)
        #[arg(short, long, default_value = "standard")]
        template: String,

        /// Path to initialize (defaults to current directory)
        path: Option<String>,
    },

    /// Show current workspace status
    Status,

    /// List available templates
    Templates,

    /// Reload workspace files
    Reload,
}

#[derive(Subcommand)]
enum CredentialsAction {
    /// Show current credential status
    Status,

    /// Import credentials from Claude Code CLI (~/.claude/.credentials.json)
    Import,

    /// Run `claude setup-token` to authenticate via Claude Code CLI
    Setup,

    /// Refresh the OAuth token (if expired or expiring soon)
    Refresh,

    /// Clear stored credentials
    Clear,
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon in the background
    Start {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value = "9999")]
        port: u16,
    },

    /// Stop the running daemon
    Stop,

    /// Check daemon status
    Status,

    /// Restart the daemon
    Restart {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = "127.0.0.1")]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value = "9999")]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = match cli.log_level.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO, // "info" or unknown defaults to INFO
    };

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(log_level.into())
                .from_env_lossy(),
        )
        .init();

    info!("🌙 Nanna v{} rising...", env!("CARGO_PKG_VERSION"));

    // Load configuration
    let config = if let Some(path) = &cli.config {
        Config::load_from(&path.into())?
    } else {
        Config::load().unwrap_or_else(|e| {
            info!("Using default config ({})", e);
            Config::default()
        })
    }
    .with_env_overrides();

    // Daemon mode - run background server
    if cli.daemon_mode {
        info!("Starting in daemon mode on {}:{}", cli.host, cli.port);
        return run_daemon(&config, cli.host, cli.port).await;
    }

    // Handle commands
    match cli.command {
        Some(Commands::Init) => {
            let _config = onboarding::run_onboarding()?;
            return Ok(());
        }
        Some(Commands::Status) => {
            onboarding::show_status(&config)?;
            return Ok(());
        }
        Some(Commands::Config { generate }) => {
            if generate {
                println!("{}", nanna_config::generate_default_config());
            } else {
                let path = Config::default_config_path()?;
                println!("Config path: {}", path.display());
                println!("\n{}", toml::to_string_pretty(&config)?);
            }
            return Ok(());
        }
        Some(Commands::Workspace { action }) => {
            handle_workspace_command(action).await?;
            return Ok(());
        }
        Some(Commands::Credentials { action }) => {
            handle_credentials_command(action).await?;
            return Ok(());
        }
        Some(Commands::Daemon { action }) => {
            handle_daemon_command(action, &config).await?;
            return Ok(());
        }
        Some(Commands::Server { host, port }) => {
            // Check for API key, offer quick setup if missing
            let config = ensure_api_key(config)?;
            run_server(&config, host, port).await?;
        }
        Some(Commands::Chat { session, model, stream }) => {
            // Check for first run
            if onboarding::is_first_run() {
                println!("Welcome! Let's get you set up first.\n");
                let config = onboarding::run_onboarding()?;
                run_cli(&config, session, model, stream).await?;
            } else {
                // Check for API key, offer quick setup if missing
                let config = ensure_api_key(config)?;
                run_cli(&config, session, model, stream).await?;
            }
        }
        Some(Commands::Sessions { limit }) => {
            list_sessions(&config, limit).await?;
        }
        Some(Commands::Run { prompt, model }) => {
            let config = ensure_api_key(config)?;
            run_once(&config, &prompt, model).await?;
        }
        None => {
            // Default: check for first run, then CLI mode
            if onboarding::is_first_run() {
                println!("Welcome! Let's get you set up first.\n");
                let config = onboarding::run_onboarding()?;
                run_cli(&config, None, None, false).await?;
            } else {
                let config = ensure_api_key(config)?;
                run_cli(&config, None, None, false).await?;
            }
        }
    }

    Ok(())
}
