//! Windows Service implementation
//!
//! Runs nanna-daemon as a proper Windows Service using the windows-service crate.

#![cfg(windows)]

use crate::service::ServiceConfig;
use std::ffi::OsString;
use std::sync::mpsc;
use std::time::Duration;
// use tokio::sync::broadcast;
use tracing::{error, info};
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

/// The name this process reports to the SCM when it runs *as* a service.
///
/// Only the runtime side uses this (the dispatcher and the control handler); the
/// management functions below take the name from `ServiceConfig` instead. For an
/// `OWN_PROCESS` service the SCM ignores the dispatch-table name, so a service
/// installed under a different name still runs correctly.
const SERVICE_NAME: &str = "nanna-daemon";
const SERVICE_TYPE: ServiceType = ServiceType::OWN_PROCESS;

/// Entry point for Windows Service Control Manager
pub fn run_as_service() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|e| format!("Failed to start service dispatcher: {}", e))
}

// Generate the Windows service boilerplate
define_windows_service!(ffi_service_main, service_main);

fn service_main(arguments: Vec<OsString>) {
    if let Err(e) = run_service(arguments) {
        error!("Service failed: {}", e);
    }
}

fn run_service(_arguments: Vec<OsString>) -> Result<(), String> {
    // Create a channel to receive shutdown signals
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();

    // Register service control handler
    let event_handler = move |control_event| -> ServiceControlHandlerResult {
        match control_event {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                let _ = shutdown_tx.send(());
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    };

    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)
        .map_err(|e| format!("Failed to register service control handler: {}", e))?;

    // Report running status
    status_handle
        .set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Running,
            controls_accepted: ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
        .map_err(|e| format!("Failed to set service status: {}", e))?;

    info!("Nanna daemon service started");

    // Create tokio runtime and run the daemon
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create tokio runtime: {}", e))?;

    runtime.block_on(async {
        // Create daemon using builder (loads config from file)
        let mut daemon = crate::server::DaemonBuilder::from_nanna_config()
            .unwrap_or_else(|_| crate::server::DaemonBuilder::new())
            .build()
            .await;
        let daemon_shutdown = daemon.shutdown_handle();

        // Spawn the daemon
        let daemon_handle = tokio::spawn(async move {
            if let Err(e) = daemon.run().await {
                error!("Daemon error: {}", e);
            }
        });

        // Wait for shutdown signal from SCM
        let _ = shutdown_rx.recv();
        info!("Service stop requested");

        // Signal daemon to shutdown
        let _ = daemon_shutdown.send(());

        // Wait for daemon to finish (with timeout)
        let _ = tokio::time::timeout(Duration::from_secs(10), daemon_handle).await;
    });

    // Report stopped status
    status_handle
        .set_service_status(ServiceStatus {
            service_type: SERVICE_TYPE,
            current_state: ServiceState::Stopped,
            controls_accepted: ServiceControlAccept::empty(),
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
        .ok();

    info!("Nanna daemon service stopped");
    Ok(())
}

/// Install the Windows Service described by `config`.
///
/// The SCM starts this executable with `config.arguments`, which on Windows must
/// reach `run_as_service` (the `service` subcommand) rather than the console
/// runner — a service that never calls `StartServiceCtrlDispatcher` is killed by
/// the SCM for not reporting status. `ServiceConfig::default()` already selects
/// the right argument per platform; see its definition.
pub fn install_service(config: &ServiceConfig) -> Result<(), String> {
    use std::process::Command;
    use windows_service::{
        service::{ServiceAccess, ServiceErrorControl, ServiceInfo, ServiceStartType},
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|e| format!("Failed to connect to service manager: {}", e))?;

    let service_info = ServiceInfo {
        name: OsString::from(&config.name),
        display_name: OsString::from(&config.display_name),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: config.executable.clone(),
        launch_arguments: config.arguments.iter().map(OsString::from).collect(),
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };

    let _service = manager
        .create_service(&service_info, ServiceAccess::CHANGE_CONFIG)
        .map_err(|e| format!("Failed to create service: {}", e))?;

    // `create_service` cannot set the description, and the SCM API for it is not
    // exposed by windows-service; shelling out to `sc` is the pragmatic route.
    // A failure here leaves a working service with a blank description, so it
    // warns rather than failing the install.
    if let Ok(output) = Command::new("sc")
        .args(["description", &config.name, &config.description])
        .output()
    {
        if !output.status.success() {
            tracing::warn!("Failed to set service description");
        }
    }

    info!("Service '{}' installed successfully", config.name);
    Ok(())
}

/// Uninstall the Windows Service named by `config`.
pub fn uninstall_service(config: &ServiceConfig) -> Result<(), String> {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    )
    .map_err(|e| format!("Failed to connect to service manager: {}", e))?;

    let service = manager
        .open_service(&config.name, ServiceAccess::DELETE | ServiceAccess::STOP)
        .map_err(|e| format!("Failed to open service: {}", e))?;

    // Stop the service if running
    let _ = service.stop();
    std::thread::sleep(Duration::from_secs(2));

    // Delete the service
    service
        .delete()
        .map_err(|e| format!("Failed to delete service: {}", e))?;

    info!("Service '{}' uninstalled successfully", config.name);
    Ok(())
}

/// Start the Windows Service named by `config`.
pub fn start_service(config: &ServiceConfig) -> Result<(), String> {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    )
    .map_err(|e| format!("Failed to connect to service manager: {}", e))?;

    let service = manager
        .open_service(&config.name, ServiceAccess::START)
        .map_err(|e| format!("Failed to open service: {}", e))?;

    service
        .start::<String>(&[])
        .map_err(|e| format!("Failed to start service: {}", e))?;

    info!("Service '{}' started", config.name);
    Ok(())
}

/// Stop the Windows Service named by `config`.
pub fn stop_service(config: &ServiceConfig) -> Result<(), String> {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    )
    .map_err(|e| format!("Failed to connect to service manager: {}", e))?;

    let service = manager
        .open_service(&config.name, ServiceAccess::STOP)
        .map_err(|e| format!("Failed to open service: {}", e))?;

    service
        .stop()
        .map_err(|e| format!("Failed to stop service: {}", e))?;

    info!("Service '{}' stopped", config.name);
    Ok(())
}

/// Query the status of the Windows Service named by `config`.
///
/// A service that cannot be opened is reported `Stopped` rather than `Unknown`:
/// the common cause is that it is not installed, and callers act on that the same
/// way they act on a stopped service. An SCM that cannot be reached at all is
/// genuinely `Unknown`.
pub fn query_service_status(config: &ServiceConfig) -> crate::service::ServiceStatus {
    use windows_service::{
        service::ServiceAccess,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    let manager = match ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT,
    ) {
        Ok(m) => m,
        Err(_) => return crate::service::ServiceStatus::Unknown,
    };

    let service = match manager.open_service(&config.name, ServiceAccess::QUERY_STATUS) {
        Ok(s) => s,
        Err(_) => return crate::service::ServiceStatus::Stopped,
    };

    match service.query_status() {
        Ok(status) => match status.current_state {
            ServiceState::Running => crate::service::ServiceStatus::Running,
            ServiceState::Stopped => crate::service::ServiceStatus::Stopped,
            ServiceState::StartPending => crate::service::ServiceStatus::Starting,
            ServiceState::StopPending => crate::service::ServiceStatus::Stopping,
            _ => crate::service::ServiceStatus::Unknown,
        },
        Err(_) => crate::service::ServiceStatus::Unknown,
    }
}
