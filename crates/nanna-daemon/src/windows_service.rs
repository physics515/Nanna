//! Windows Service implementation
//!
//! Runs nanna-daemon as a proper Windows Service using the windows-service crate.

#![cfg(windows)]

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
            .build();
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

/// Install the Windows Service
pub fn install_service() -> Result<(), String> {
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

    let executable_path = std::env::current_exe()
        .map_err(|e| format!("Failed to get executable path: {}", e))?;

    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from("Nanna Daemon"),
        service_type: SERVICE_TYPE,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path,
        launch_arguments: vec![OsString::from("service")],
        dependencies: vec![],
        account_name: None, // LocalSystem
        account_password: None,
    };

    let _service = manager
        .create_service(&service_info, ServiceAccess::CHANGE_CONFIG)
        .map_err(|e| format!("Failed to create service: {}", e))?;

    // Set description
    if let Ok(output) = Command::new("sc")
        .args([
            "description",
            SERVICE_NAME,
            "Nanna AI assistant background service",
        ])
        .output()
    {
        if !output.status.success() {
            tracing::warn!("Failed to set service description");
        }
    }

    info!("Service installed successfully");
    Ok(())
}

/// Uninstall the Windows Service
pub fn uninstall_service() -> Result<(), String> {
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
        .open_service(SERVICE_NAME, ServiceAccess::DELETE | ServiceAccess::STOP)
        .map_err(|e| format!("Failed to open service: {}", e))?;

    // Stop the service if running
    let _ = service.stop();
    std::thread::sleep(Duration::from_secs(2));

    // Delete the service
    service
        .delete()
        .map_err(|e| format!("Failed to delete service: {}", e))?;

    info!("Service uninstalled successfully");
    Ok(())
}

/// Start the Windows Service
pub fn start_service() -> Result<(), String> {
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
        .open_service(SERVICE_NAME, ServiceAccess::START)
        .map_err(|e| format!("Failed to open service: {}", e))?;

    service
        .start::<String>(&[])
        .map_err(|e| format!("Failed to start service: {}", e))?;

    info!("Service started");
    Ok(())
}

/// Stop the Windows Service
pub fn stop_service() -> Result<(), String> {
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
        .open_service(SERVICE_NAME, ServiceAccess::STOP)
        .map_err(|e| format!("Failed to open service: {}", e))?;

    service
        .stop()
        .map_err(|e| format!("Failed to stop service: {}", e))?;

    info!("Service stopped");
    Ok(())
}

/// Query Windows Service status
pub fn query_service_status() -> crate::service::ServiceStatus {
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

    let service = match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
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
