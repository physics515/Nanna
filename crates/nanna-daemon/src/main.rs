#![warn(clippy::pedantic, clippy::nursery, clippy::all)]

//! Nanna Daemon - Main entry point
//!
//! Usage:
//!   nanna-daemon run              Run the daemon in foreground
//!   nanna-daemon start            Start as background service
//!   nanna-daemon stop             Stop the background service
//!   nanna-daemon status           Show daemon status
//!   nanna-daemon install          Install as system service
//!   nanna-daemon uninstall        Uninstall system service
//!   nanna-daemon service          (Windows only) Run as Windows Service

use clap::{Parser, Subcommand};
use nanna_daemon::server::DaemonBuilder;
use nanna_daemon::service::ServiceStatus;
#[cfg(not(windows))]
use nanna_daemon::service::{ServiceConfig, ServiceManager};
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Global log buffer initialized before the tracing subscriber
static LOG_BUFFER: OnceLock<nanna_daemon::log_buffer::LogBuffer> = OnceLock::new();

#[cfg(windows)]
use nanna_daemon::windows_service;

#[derive(Parser)]
#[command(name = "nanna-daemon")]
#[command(about = "Nanna AI assistant background daemon")]
#[command(version)]
// CLI toggles are naturally independent booleans; a bitflags/enum here would
// only obscure the arg surface.
#[allow(clippy::struct_excessive_bools)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    /// WebSocket port to listen on
    #[arg(short, long, default_value = "5149")]
    port: u16,
    
    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    
    /// HTTP health port (default: 5148)
    #[arg(long, default_value = "5148")]
    health_port: u16,
    
    /// Disable HTTP health server
    #[arg(long)]
    no_health_server: bool,
    
    /// Disable PID file (allows multiple instances)
    #[arg(long)]
    no_pid_file: bool,
    
    /// Enable webhook server for inbound messages
    #[arg(long)]
    enable_webhooks: bool,
    
    /// Webhook server port (default: 3000)
    #[arg(long, default_value = "3000")]
    webhook_port: u16,
    
    /// Data directory
    #[arg(long)]
    data_dir: Option<PathBuf>,
    
    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,

    /// Directory for rotating file logs (default: `{data_dir}/logs`).
    /// Logs roll daily; at most 7 files are kept.
    #[arg(long)]
    log_dir: Option<PathBuf>,

    /// Disable rotating file logs (console + in-memory buffer only).
    #[arg(long)]
    no_file_log: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the daemon in foreground
    Run,
    /// Start as background service
    Start,
    /// Stop the background service
    Stop,
    /// Restart the background service
    Restart,
    /// Show daemon status
    Status,
    /// Install as system service
    Install,
    /// Uninstall system service
    Uninstall,
    /// Run as Windows Service (called by SCM)
    #[cfg(windows)]
    Service,
}

/// Resolve the log directory and build a non-blocking, daily-rotating file
/// writer for the daemon. Returns `None` when file logging is disabled
/// (`--no-file-log`) or the appender can't be built — in both cases startup
/// falls back to console + in-memory logging rather than aborting.
fn file_log_writer(
    cli: &Cli,
) -> Option<(
    tracing_appender::non_blocking::NonBlocking,
    tracing_appender::non_blocking::WorkerGuard,
)> {
    if cli.no_file_log {
        return None;
    }

    // Resolve the data dir the same way `run_daemon` does so logs land beside
    // the daemon's data; fall back to the cwd only if that lookup fails.
    let data_dir = nanna_config::Config::default_data_dir().unwrap_or_else(|_| PathBuf::from("."));
    let log_dir = nanna_daemon::log_file::resolve_log_dir(cli.log_dir.as_deref(), &data_dir);
    debug_assert!(
        !log_dir.as_os_str().is_empty(),
        "resolved log dir must be non-empty"
    );

    match nanna_daemon::log_file::build_appender(&log_dir) {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            Some((writer, guard))
        }
        Err(e) => {
            // Non-fatal: console + in-memory logging still work.
            eprintln!("warning: file logging disabled: {e}");
            None
        }
    }
}

fn main() {
    let cli = Cli::parse();
    
    // Special case: Windows Service mode doesn't parse args normally
    #[cfg(windows)]
    if let Commands::Service = cli.command {
        // Minimal logging for service mode
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("info"))
            .with(tracing_subscriber::fmt::layer())
            .init();
        
        if let Err(e) = windows_service::run_as_service() {
            error!("Service failed: {}", e);
            std::process::exit(1);
        }
        return;
    }
    
    // Setup logging for non-service modes with log buffer for GUI
    let log_buffer = nanna_daemon::log_buffer::LogBuffer::new(5000);
    let log_layer = nanna_daemon::log_buffer::LogBufferLayer::new(log_buffer.clone());

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&cli.log_level));

    // Optional rotating file log. `Option<Layer>` is itself a `Layer` (None is a
    // no-op), so this composes cleanly whether or not file logging is enabled.
    // The worker guard is kept as a `main`-scoped local so it drops (flushing the
    // appender) on normal return; a `static` would never be dropped at exit.
    let (file_layer, _file_log_guard) = match file_log_writer(&cli) {
        Some((writer, guard)) => {
            let layer = tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer);
            (Some(layer), Some(guard))
        }
        None => (None, None),
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(log_layer)
        .with(file_layer)
        .init();

    // Store log_buffer so run_daemon can pass it to the DaemonBuilder
    LOG_BUFFER.set(log_buffer).ok();
    
    let result = match cli.command {
        Commands::Run => run_daemon(&cli),
        Commands::Start => start_service(&cli),
        Commands::Stop => stop_service(&cli),
        Commands::Restart => restart_service(&cli),
        Commands::Status => show_status(&cli),
        Commands::Install => install_service(&cli),
        Commands::Uninstall => uninstall_service(&cli),
        #[cfg(windows)]
        Commands::Service => unreachable!(), // Handled above
    };
    
    if let Err(e) = result {
        error!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run_daemon(cli: &Cli) -> Result<(), String> {
    info!("Starting Nanna daemon...");
    
    // Create tokio runtime
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create runtime: {}", e))?;
    
    runtime.block_on(async {
        // Load config from Nanna config file (includes API keys)
        let mut builder = DaemonBuilder::from_nanna_config()
            .map_err(|e| format!("Failed to load config: {}", e))?
            .with_port(cli.port)
            .with_host(&cli.host)
            .with_log_level(&cli.log_level)
            .with_health_port(cli.health_port)
            .with_health_server(!cli.no_health_server)
            .with_pid_file(!cli.no_pid_file)
            .with_webhook_server(cli.enable_webhooks)
            .with_webhook_port(cli.webhook_port);
        
        if let Some(ref data_dir) = cli.data_dir {
            builder = builder.with_data_dir(data_dir);
        }

        // Pass log buffer to daemon for serving logs via control plane
        if let Some(buf) = LOG_BUFFER.get() {
            builder = builder.with_log_buffer(buf.clone());
        }

        let mut daemon = builder.build().await;
        
        // Setup signal handlers
        let shutdown_tx = daemon.shutdown_handle();
        
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sigterm = signal(SignalKind::terminate()).map_err(|e| e.to_string())?;
            let mut sigint = signal(SignalKind::interrupt()).map_err(|e| e.to_string())?;
            
            let shutdown = shutdown_tx.clone();
            tokio::spawn(async move {
                tokio::select! {
                    _ = sigterm.recv() => {
                        info!("Received SIGTERM");
                    }
                    _ = sigint.recv() => {
                        info!("Received SIGINT");
                    }
                }
                let _ = shutdown.send(());
            });
        }
        
        #[cfg(windows)]
        {
            let shutdown = shutdown_tx.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                info!("Received Ctrl+C");
                let _ = shutdown.send(());
            });
        }
        
        daemon.run().await.map_err(|e| e.to_string())
    })
}

#[cfg(windows)]
fn start_service(_cli: &Cli) -> Result<(), String> {
    match windows_service::query_service_status() {
        ServiceStatus::Running => {
            println!("Daemon is already running");
            Ok(())
        }
        _ => {
            println!("Starting daemon...");
            windows_service::start_service()?;
            println!("Daemon started");
            Ok(())
        }
    }
}

#[cfg(not(windows))]
fn start_service(cli: &Cli) -> Result<(), String> {
    let manager = get_service_manager(cli);
    match manager.status() {
        ServiceStatus::Running => {
            println!("Daemon is already running");
            Ok(())
        }
        _ => {
            println!("Starting daemon...");
            manager.start()?;
            println!("Daemon started");
            Ok(())
        }
    }
}

#[cfg(windows)]
fn stop_service(_cli: &Cli) -> Result<(), String> {
    match windows_service::query_service_status() {
        ServiceStatus::Stopped => {
            println!("Daemon is not running");
            Ok(())
        }
        _ => {
            println!("Stopping daemon...");
            windows_service::stop_service()?;
            println!("Daemon stopped");
            Ok(())
        }
    }
}

#[cfg(not(windows))]
fn stop_service(cli: &Cli) -> Result<(), String> {
    let manager = get_service_manager(cli);
    match manager.status() {
        ServiceStatus::Stopped => {
            println!("Daemon is not running");
            Ok(())
        }
        _ => {
            println!("Stopping daemon...");
            manager.stop()?;
            println!("Daemon stopped");
            Ok(())
        }
    }
}

#[cfg(windows)]
fn restart_service(_cli: &Cli) -> Result<(), String> {
    if windows_service::query_service_status() == ServiceStatus::Running {
        println!("Stopping daemon...");
        windows_service::stop_service()?;
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    println!("Starting daemon...");
    windows_service::start_service()?;
    println!("Daemon restarted");
    Ok(())
}

#[cfg(not(windows))]
fn restart_service(cli: &Cli) -> Result<(), String> {
    let manager = get_service_manager(cli);
    if manager.status() == ServiceStatus::Running {
        println!("Stopping daemon...");
        manager.stop()?;
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    println!("Starting daemon...");
    manager.start()?;
    println!("Daemon restarted");
    Ok(())
}

fn show_status(cli: &Cli) -> Result<(), String> {
    #[cfg(windows)]
    let status = windows_service::query_service_status();
    
    #[cfg(not(windows))]
    let status = get_service_manager(cli).status();
    
    println!("Nanna Daemon Status");
    println!("==================");
    println!("Service: {}", match status {
        ServiceStatus::Running => "Running ✓",
        ServiceStatus::Stopped => "Stopped",
        ServiceStatus::Starting => "Starting...",
        ServiceStatus::Stopping => "Stopping...",
        ServiceStatus::Unknown => "Unknown",
    });
    println!("WebSocket Port: {}", cli.port);
    println!("Health Port: {}", cli.health_port);
    println!("Host: {}", cli.host);
    
    // Try to connect to health endpoint
    if status == ServiceStatus::Running || status == ServiceStatus::Unknown {
        let health_url = format!("http://{}:{}/health", cli.host, cli.health_port);
        println!("\nChecking health endpoint: {}", health_url);
        
        // Try to fetch health endpoint
        match std::process::Command::new("curl")
            .args(["-s", "-m", "2", &health_url])
            .output()
        {
            Ok(output) if output.status.success() => {
                let body = String::from_utf8_lossy(&output.stdout);
                if body.contains("ok") {
                    println!("Health: OK ✓");
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) {
                        if let Some(uptime) = json.get("uptime_secs").and_then(|v| v.as_u64()) {
                            let hours = uptime / 3600;
                            let mins = (uptime % 3600) / 60;
                            let secs = uptime % 60;
                            println!("Uptime: {}h {}m {}s", hours, mins, secs);
                        }
                        if let Some(version) = json.get("version").and_then(|v| v.as_str()) {
                            println!("Version: {}", version);
                        }
                    }
                } else {
                    println!("Health: Unknown (response: {})", body.chars().take(50).collect::<String>());
                }
            }
            Ok(_) => {
                println!("Health: Not responding (daemon may be starting or not running)");
            }
            Err(_) => {
                println!("Health: Could not check (curl not available)");
                println!("\nTrying WebSocket at ws://{}:{}...", cli.host, cli.port);
                println!("(WebSocket test not implemented)");
            }
        }
    }
    
    Ok(())
}

#[cfg(windows)]
fn install_service(_cli: &Cli) -> Result<(), String> {
    println!("Installing Nanna daemon as Windows Service...");
    windows_service::install_service()?;
    println!("Service installed successfully");
    println!("\nTo start the daemon, run:");
    println!("  nanna-daemon start");
    Ok(())
}

#[cfg(not(windows))]
fn install_service(cli: &Cli) -> Result<(), String> {
    let manager = get_service_manager(cli);
    println!("Installing Nanna daemon as system service...");
    manager.install()?;
    println!("Service installed successfully");
    println!("\nTo start the daemon, run:");
    println!("  nanna-daemon start");
    Ok(())
}

#[cfg(windows)]
fn uninstall_service(_cli: &Cli) -> Result<(), String> {
    // Stop if running
    if windows_service::query_service_status() == ServiceStatus::Running {
        println!("Stopping running daemon...");
        windows_service::stop_service()?;
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    
    println!("Uninstalling Nanna daemon service...");
    windows_service::uninstall_service()?;
    println!("Service uninstalled successfully");
    Ok(())
}

#[cfg(not(windows))]
fn uninstall_service(cli: &Cli) -> Result<(), String> {
    let manager = get_service_manager(cli);
    
    // Stop if running
    if manager.status() == ServiceStatus::Running {
        println!("Stopping running daemon...");
        manager.stop()?;
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
    
    println!("Uninstalling Nanna daemon service...");
    manager.uninstall()?;
    println!("Service uninstalled successfully");
    Ok(())
}

#[cfg(not(windows))]
fn get_service_manager(_cli: &Cli) -> ServiceManager {
    ServiceManager::new(ServiceConfig::default())
}
