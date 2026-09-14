#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]
// Raised for the same reason as `nanna-daemon`'s: proving a future or closure
// is `Send` walks into wgpu's `Global`/`Hub`/`Registry` graph by way of
// `CosineSimilaritySearch`, which is deeper than the default limit of 128.
// nightly-2026-08-25 turned that overflow into the future-incompatible
// `recursion_depth_exceeding_limit` warning (rust#159228), which is scheduled
// to become a hard error. Solver depth only — no behaviour, no codegen change.
#![recursion_limit = "256"]

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
use nanna_config::bind::LOOPBACK_HOST;
use nanna_daemon::DEFAULT_IPC_PORT;
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
    #[arg(long, hide = true, default_value = LOOPBACK_HOST)]
    host: String,

    /// Daemon port
    #[arg(long, hide = true, default_value_t = DEFAULT_IPC_PORT)]
    port: u16,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize Nanna with setup wizard
    Init,

    /// Show configuration status
    Status,

    /// Diagnose configuration problems and say how to fix each one.
    ///
    /// Offline by default: no provider, network or keyring probe runs, so this
    /// is safe and fast anywhere. Exits non-zero when a check fails.
    Doctor {
        /// Also probe each Ollama server the configuration uses: is it
        /// answering, and does it have the configured models? Never reads the
        /// keyring or sends a provider key.
        #[arg(long)]
        online: bool,
    },

    /// Start the HTTP server
    Server {
        /// Host to bind to. Defaults to loopback; pass `0.0.0.0` to expose the
        /// server to other machines (it has no authentication of its own).
        #[arg(short = 'H', long, default_value = nanna_config::LOOPBACK_HOST)]
        host: String,

        /// Port to listen on. Defaults to `[server].port` in config.toml (or the
        /// `PORT` environment variable), which itself defaults to 3000.
        #[arg(short, long)]
        port: Option<u16>,
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

    /// Export a session — or, with `--memories`, the memory store — as
    /// Markdown (readable) or JSON (lossless)
    //
    // Exactly one target, enforced by a group: `requires = "memories"` on
    // `--scope` was satisfied by the bool flag's implicit `false` default, so
    // `nanna export <id> --scope x` parsed and silently ignored the scope.
    #[command(group(clap::ArgGroup::new("target").required(true).args(["session", "memories"])))]
    Export {
        /// Session ID (see `nanna sessions`); omit it with --memories
        session: Option<String>,

        /// Export the memory store instead of a session
        #[arg(long)]
        memories: bool,

        /// With --memories: `global`, or a workspace id (global plus that
        /// workspace). Every memory when omitted.
        #[arg(long, conflicts_with = "session")]
        scope: Option<String>,

        /// Document format
        #[arg(short, long, value_enum, default_value_t = commands::export::ExportFormatArg::Markdown)]
        format: commands::export::ExportFormatArg,

        /// Write here — a file, or a directory that receives the suggested
        /// file name. Prints to stdout when omitted.
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
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

    /// Model Context Protocol server — expose Nanna's tools to an MCP client
    Mcp {
        #[command(subcommand)]
        action: McpAction,
    },
}

#[derive(Subcommand)]
enum McpAction {
    /// Serve Nanna's tools over stdio JSON-RPC (for Claude Code, editors, …).
    ///
    /// stdout carries the protocol, so all logging goes to stderr.
    Serve {
        /// Directory of JS/TS tool skills (default: `[tools] tools_dir`,
        /// `NANNA_TOOLS_DIR`, or the dev tree)
        #[arg(long)]
        tools_dir: Option<std::path::PathBuf>,
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
        #[arg(short = 'H', long, default_value = LOOPBACK_HOST)]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value_t = DEFAULT_IPC_PORT)]
        port: u16,
    },

    /// Stop the running daemon
    Stop,

    /// Check daemon status
    Status,

    /// Restart the daemon
    Restart {
        /// Host to bind to
        #[arg(short = 'H', long, default_value = LOOPBACK_HOST)]
        host: String,

        /// Port to listen on
        #[arg(short, long, default_value_t = DEFAULT_IPC_PORT)]
        port: u16,
    },
}

/// Parse the `--log-level` string into a level, defaulting to INFO for an
/// unrecognised value rather than failing the whole run over a typo.
fn parse_log_level(raw: &str) -> Level {
    match raw.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    }
}

/// Install the global tracing subscriber.
///
/// `logs_to_stderr` exists for `nanna mcp serve`, which speaks JSON-RPC on
/// stdout: ANY log written there corrupts the stream and the client drops the
/// connection with a parse error. Every other command keeps writing to stdout,
/// so this is a per-command decision, not a global one.
fn init_logging(log_level: Level, logs_to_stderr: bool) {
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(log_level.into())
        .from_env_lossy();
    if logs_to_stderr {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .with(filter)
            .init();
    } else {
        tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer())
            .with(filter)
            .init();
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let logs_to_stderr = matches!(cli.command, Some(Commands::Mcp { .. }));
    init_logging(parse_log_level(&cli.log_level), logs_to_stderr);

    // The banner is a log line, so it follows the same writer — it must never
    // land on stdout ahead of the first JSON-RPC response.
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
        Some(Commands::Doctor { online }) => {
            let path = Config::default_config_path()?;
            let worst = commands::doctor::run(&config, &path, online).await;
            // Non-zero on a real fault so this is usable from a script or a
            // health probe, not just by eye.
            if worst == commands::doctor::Severity::Fail {
                std::process::exit(1);
            }
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
        Some(Commands::Mcp { action }) => {
            match action {
                McpAction::Serve { tools_dir } => {
                    commands::mcp::serve(&config, tools_dir).await?;
                }
            }
            return Ok(());
        }
        Some(Commands::Server { host, port }) => {
            // Check for API key, offer quick setup if missing
            let config = ensure_api_key(config)?;
            let port = commands::serve::server_port(port, &config);
            run_server(&config, host, port).await?;
        }
        Some(Commands::Chat { session, model, stream }) => {
            let config = interactive_config(config)?;
            run_cli(&config, session, model, stream).await?;
        }
        Some(Commands::Export {
            session,
            memories,
            scope,
            format,
            output,
        }) => {
            let target = commands::export::ExportTarget::from_cli(session, memories, scope)?;
            commands::export::export(target, format, output).await?;
            return Ok(());
        }
        Some(Commands::Sessions { limit }) => {
            list_sessions(&config, limit).await?;
        }
        Some(Commands::Run { prompt, model }) => {
            let config = ensure_api_key(config)?;
            run_once(&config, &prompt, model).await?;
        }
        None => {
            // Default: interactive chat.
            let config = interactive_config(config)?;
            run_cli(&config, None, None, false).await?;
        }
    }

    Ok(())
}

/// The config an interactive chat starts with: onboarding on a first run,
/// otherwise the loaded config, with quick setup offered if no API key is set.
fn interactive_config(config: Config) -> anyhow::Result<Config> {
    if onboarding::is_first_run() {
        println!("Welcome! Let's get you set up first.\n");
        return onboarding::run_onboarding();
    }
    ensure_api_key(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `nanna server` with no `--port` must parse to `None` so the config
    /// decides the port: a clap `default_value` here is exactly what used to
    /// shadow `[server].port`. The subcommand's flag must also not be
    /// confused with the hidden global daemon `--port`.
    #[test]
    fn the_server_port_flag_is_optional_so_the_config_can_decide() {
        let bare = Cli::try_parse_from(["nanna", "server"]).expect("bare `nanna server` parses");
        match bare.command {
            Some(Commands::Server { port, .. }) => assert_eq!(port, None),
            _ => panic!("expected the server command"),
        }
        let flagged = Cli::try_parse_from(["nanna", "server", "--port", "8080"])
            .expect("`nanna server --port 8080` parses");
        match flagged.command {
            Some(Commands::Server { port, .. }) => assert_eq!(port, Some(8080)),
            _ => panic!("expected the server command"),
        }
        assert_eq!(
            flagged.port, DEFAULT_IPC_PORT,
            "the subcommand's --port does not leak into the global daemon port"
        );
    }

    /// `doctor` stays offline unless asked: the network leg is opt-in.
    #[test]
    fn doctor_is_offline_unless_asked() {
        let bare = Cli::try_parse_from(["nanna", "doctor"]).expect("`nanna doctor` parses");
        assert!(matches!(
            bare.command,
            Some(Commands::Doctor { online: false })
        ));
        let online =
            Cli::try_parse_from(["nanna", "doctor", "--online"]).expect("`--online` parses");
        assert!(matches!(
            online.command,
            Some(Commands::Doctor { online: true })
        ));
    }

    #[test]
    fn export_parses_its_session_format_and_output() {
        let cli = Cli::try_parse_from(["nanna", "export", "abc123", "-f", "md", "-o", "out.md"])
            .expect("`nanna export` parses");
        match cli.command {
            Some(Commands::Export {
                session,
                memories,
                format,
                output,
                ..
            }) => {
                assert_eq!(session.as_deref(), Some("abc123"));
                assert!(!memories, "a session export is not a memory export");
                assert_eq!(
                    format,
                    commands::export::ExportFormatArg::Markdown,
                    "`md` is an alias"
                );
                assert_eq!(output, Some(std::path::PathBuf::from("out.md")));
            }
            _ => panic!("expected the export command"),
        }
        let bare = Cli::try_parse_from(["nanna", "export", "abc123"]).expect("parses");
        match bare.command {
            Some(Commands::Export { format, output, .. }) => {
                assert_eq!(
                    format,
                    commands::export::ExportFormatArg::Markdown,
                    "markdown by default"
                );
                assert_eq!(output, None, "stdout by default");
            }
            _ => panic!("expected the export command"),
        }
    }

    #[test]
    fn export_memories_takes_a_scope_and_no_session() {
        let cli = Cli::try_parse_from(["nanna", "export", "--memories", "--scope", "ws1"])
            .expect("`nanna export --memories` parses");
        match cli.command {
            Some(Commands::Export {
                session,
                memories,
                scope,
                ..
            }) => {
                assert!(memories);
                assert_eq!(session, None);
                assert_eq!(scope.as_deref(), Some("ws1"));
            }
            _ => panic!("expected the export command"),
        }
        assert!(
            Cli::try_parse_from(["nanna", "export", "abc123", "--memories"]).is_err(),
            "a session id and --memories are one or the other"
        );
        assert!(
            Cli::try_parse_from(["nanna", "export", "abc123", "--scope", "ws1"]).is_err(),
            "--scope only means something with --memories"
        );
        assert!(
            Cli::try_parse_from(["nanna", "export"]).is_err(),
            "exporting nothing is refused"
        );
    }
}
