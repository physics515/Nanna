//! `daemon` subcommand handlers and background process management.

use crate::DaemonAction;
use nanna_config::Config;
use nanna_config::bind::LOOPBACK_HOST;
use nanna_daemon::DEFAULT_IPC_PORT;
use std::path::PathBuf;
use tracing::info;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Handle daemon subcommands
pub async fn handle_daemon_command(action: DaemonAction, _config: &Config) -> anyhow::Result<()> {
    let pid_file = Config::default_data_dir()?.join("nanna-daemon.pid");

    match action {
        DaemonAction::Start { host, port } => {
            println!("🌙 Starting Nanna daemon...\n");
            if is_daemon_running(&pid_file) {
                println!("⚠️  Daemon is already running");
                println!("   Run 'nanna daemon status' to check details");
                println!("   Run 'nanna daemon stop' to stop it");
                return Ok(());
            }
            let (pid, log_file) = spawn_daemon_process(&host, port, &pid_file)?;
            println!("✅ Daemon started!");
            println!("   PID: {pid}");
            println!("   Address: ws://{host}:{port}/ws");
            println!("   Logs: {}", log_file.display());
            println!("\n   Use 'nanna daemon status' to check status");
            println!("   Use 'nanna daemon stop' to stop the daemon");
        }
        DaemonAction::Stop => {
            println!("🌙 Stopping Nanna daemon...\n");
            stop_daemon_process(&pid_file)?;
        }
        DaemonAction::Status => {
            print_daemon_status(&pid_file).await?;
        }
        DaemonAction::Restart { host, port } => {
            println!("🌙 Restarting Nanna daemon...\n");
            if is_daemon_running(&pid_file) {
                println!("Stopping current daemon...");
                stop_daemon_process(&pid_file)?;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            println!("Starting daemon...");
            if is_daemon_running(&pid_file) {
                println!("⚠️  Daemon is already running");
                return Ok(());
            }
            let (pid, _) = spawn_daemon_process(&host, port, &pid_file)?;
            println!("✅ Daemon restarted!");
            println!("   PID: {pid}");
            println!("   Address: ws://{host}:{port}/ws");
        }
    }

    Ok(())
}

/// Spawn a daemon process in the background. Returns (PID, log file path).
fn spawn_daemon_process(host: &str, port: u16, pid_file: &std::path::Path) -> anyhow::Result<(u32, PathBuf)> {
    use std::fs;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe()?;
    let log_dir = Config::default_data_dir()?;
    fs::create_dir_all(&log_dir)?;
    let log_file = log_dir.join("daemon.log");

    let log_handle = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;

    info!("Spawning daemon process...");

    #[cfg(windows)]
    let child = Command::new(exe)
        .arg("--daemon-mode")
        .arg("--host")
        .arg(host)
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::from(log_handle.try_clone()?))
        .stderr(Stdio::from(log_handle))
        .creation_flags(0x0800_0000)
        .spawn()?;

    #[cfg(not(windows))]
    let child = Command::new(exe)
        .arg("--daemon-mode")
        .arg("--host")
        .arg(host)
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::from(log_handle.try_clone()?))
        .stderr(Stdio::from(log_handle))
        .spawn()?;

    let pid = child.id();
    fs::write(pid_file, pid.to_string())?;
    Ok((pid, log_file))
}

/// Stop the daemon process identified by the PID file.
fn stop_daemon_process(pid_file: &std::path::Path) -> anyhow::Result<()> {
    use std::fs;

    if !pid_file.exists() {
        println!("❌ No daemon PID file found");
        println!("   Daemon may not be running");
        return Ok(());
    }

    let pid_str = fs::read_to_string(pid_file)?;
    let pid: u32 = pid_str.trim().parse()?;

    #[cfg(windows)]
    {
        use std::process::Command;
        let status = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .status()?;

        if status.success() {
            println!("✅ Daemon stopped (PID {pid})");
        } else {
            println!("⚠️  Failed to stop daemon (PID {pid})");
            println!("   It may have already been terminated");
        }
    }

    #[cfg(not(windows))]
    {
        // SAFETY: kill(2) is safe to call with a valid PID and signal number
        let ret = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if ret == 0 {
            println!("✅ Daemon stopped (PID {pid})");
        } else {
            let err = std::io::Error::last_os_error();
            println!("⚠️  Failed to stop daemon (PID {pid}): {err}");
            println!("   It may have already been terminated");
        }
    }

    let _ = fs::remove_file(pid_file);
    Ok(())
}

/// Print daemon status information.
async fn print_daemon_status(pid_file: &std::path::Path) -> anyhow::Result<()> {
    use nanna_client::{Client, ClientConfig};

    println!("🌙 Nanna Daemon Status\n");

    if !pid_file.exists() {
        println!("   Status: Not running");
        println!("   Start with: nanna daemon start");
        return Ok(());
    }

    let pid_str = std::fs::read_to_string(pid_file)?;
    let pid: u32 = pid_str.trim().parse()?;
    let is_running = is_process_alive(pid);

    if is_running {
        println!("   Status: ✅ Running");
        println!("   PID: {pid}");

        // Same constant `daemon start` binds, so status can never probe a different port
        // than the one the daemon was launched on — which is exactly what used to happen.
        let address = format!("ws://{LOOPBACK_HOST}:{DEFAULT_IPC_PORT}");
        let client_config = ClientConfig::new(&address);
        if let Ok(Ok(_)) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            Client::connect(client_config)
        ).await {
            println!("   Connection: ✅ Healthy");
            println!("   Address: {address}");
        } else {
            println!("   Connection: ⚠️  Not responding");
            println!("   (Daemon may be starting up or misconfigured)");
        }
    } else {
        println!("   Status: ❌ Not running (stale PID file)");
        println!("   Cleaning up...");
        let _ = std::fs::remove_file(pid_file);
        println!("   Start with: nanna daemon start");
    }
    Ok(())
}

/// Check if daemon is running based on PID file
fn is_daemon_running(pid_file: &PathBuf) -> bool {
    if !pid_file.exists() {
        return false;
    }

    if let Ok(pid_str) = std::fs::read_to_string(pid_file)
        && let Ok(pid) = pid_str.trim().parse::<u32>() {
            return is_process_alive(pid);
        }

    false
}

/// Check if a process with given PID is alive
fn is_process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        use std::process::Command;
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .is_ok_and(|output| {
                let stdout = String::from_utf8_lossy(&output.stdout);
                stdout.contains(&pid.to_string())
            })
    }

    #[cfg(not(windows))]
    {
        // SAFETY: kill(2) with signal 0 checks process existence without sending a signal
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
}
