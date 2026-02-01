//! Nanna Daemon - Main entry point
//!
//! Usage:
//!   nanna-daemon run              Run the daemon in foreground
//!   nanna-daemon start            Start as background service
//!   nanna-daemon stop             Stop the background service
//!   nanna-daemon status           Show daemon status
//!   nanna-daemon install          Install as system service
//!   nanna-daemon uninstall        Uninstall system service

use clap::{Parser, Subcommand};
use nanna_daemon::server::{DaemonBuilder, DaemonConfig};
use nanna_daemon::service::{ServiceConfig, ServiceManager, ServiceStatus};
use std::path::PathBuf;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "nanna-daemon")]
#[command(about = "Nanna AI assistant background daemon")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    
    /// Port to listen on
    #[arg(short, long, default_value = "5149")]
    port: u16,
    
    /// Host to bind to
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    
    /// Data directory
    #[arg(long)]
    data_dir: Option<PathBuf>,
    
    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info")]
    log_level: String,
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
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    
    // Setup logging
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&cli.log_level));
    
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
    
    let result = match cli.command {
        Commands::Run => run_daemon(&cli).await,
        Commands::Start => start_service(&cli),
        Commands::Stop => stop_service(&cli),
        Commands::Restart => restart_service(&cli),
        Commands::Status => show_status(&cli),
        Commands::Install => install_service(&cli),
        Commands::Uninstall => uninstall_service(&cli),
    };
    
    if let Err(e) = result {
        error!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn run_daemon(cli: &Cli) -> Result<(), String> {
    info!("Starting Nanna daemon...");
    
    let mut builder = DaemonBuilder::new()
        .with_port(cli.port)
        .with_host(&cli.host)
        .with_log_level(&cli.log_level);
    
    if let Some(ref data_dir) = cli.data_dir {
        builder = builder.with_data_dir(data_dir);
    }
    
    let mut daemon = builder.build();
    
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
}

fn get_service_manager(_cli: &Cli) -> ServiceManager {
    ServiceManager::new(ServiceConfig::default())
}

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

fn restart_service(cli: &Cli) -> Result<(), String> {
    let manager = get_service_manager(cli);
    
    if manager.status() == ServiceStatus::Running {
        println!("Stopping daemon...");
        manager.stop()?;
    }
    
    println!("Starting daemon...");
    manager.start()?;
    println!("Daemon restarted");
    Ok(())
}

fn show_status(cli: &Cli) -> Result<(), String> {
    let manager = get_service_manager(cli);
    let status = manager.status();
    
    println!("Nanna Daemon Status");
    println!("==================");
    println!("Service: {}", match status {
        ServiceStatus::Running => "Running ✓",
        ServiceStatus::Stopped => "Stopped",
        ServiceStatus::Starting => "Starting...",
        ServiceStatus::Stopping => "Stopping...",
        ServiceStatus::Unknown => "Unknown",
    });
    println!("Port: {}", cli.port);
    println!("Host: {}", cli.host);
    
    // Try to connect to running daemon
    if status == ServiceStatus::Running {
        println!("\nTrying to connect to ws://{}:{}...", cli.host, cli.port);
        // TODO: Actually test connection
    }
    
    Ok(())
}

fn install_service(cli: &Cli) -> Result<(), String> {
    let manager = get_service_manager(cli);
    
    println!("Installing Nanna daemon as system service...");
    manager.install()?;
    println!("Service installed successfully");
    println!("\nTo start the daemon, run:");
    println!("  nanna-daemon start");
    
    Ok(())
}

fn uninstall_service(cli: &Cli) -> Result<(), String> {
    let manager = get_service_manager(cli);
    
    // Stop if running
    if manager.status() == ServiceStatus::Running {
        println!("Stopping running daemon...");
        manager.stop()?;
    }
    
    println!("Uninstalling Nanna daemon service...");
    manager.uninstall()?;
    println!("Service uninstalled successfully");
    
    Ok(())
}
