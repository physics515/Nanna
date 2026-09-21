//! `daemon` subcommand handlers and background process management.
//!
//! The daemon owns its single-instance PID file: it claims the file under a
//! lock at startup and removes it on the way out. These commands only read
//! it, judged by the daemon's own [`PidFile::state`]. They never write or
//! delete it: a write from here ran outside the daemon's lock (and could
//! overwrite a live daemon's record with a child about to be refused), and a
//! liveness-only check of their own read a PID reused by another program as
//! a running daemon, which `stop` would then signal.

use crate::DaemonAction;
use nanna_config::Config;
use nanna_config::bind::LOOPBACK_HOST;
use nanna_daemon::DEFAULT_IPC_PORT;
use nanna_daemon::health::{DAEMON_MODE_FLAG, PidFile, PidFileState, ProcessProbe};
use std::path::PathBuf;
use tracing::info;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Handle daemon subcommands
pub async fn handle_daemon_command(action: DaemonAction, _config: &Config) -> anyhow::Result<()> {
    let pid_file = PidFile::new(&Config::default_data_dir()?);

    match action {
        DaemonAction::Start { host, port } => {
            println!("🌙 Starting Nanna daemon...\n");
            if let Some(pid) = pid_file.state()?.live_daemon() {
                println!("⚠️  Daemon is already running (PID {pid})");
                println!("   Run 'nanna daemon status' to check details");
                println!("   Run 'nanna daemon stop' to stop it");
                return Ok(());
            }
            let (pid, log_file) = spawn_daemon_process(&host, port)?;
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
            if pid_file.state()?.live_daemon().is_some() {
                println!("Stopping current daemon...");
                stop_daemon_process(&pid_file)?;
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            }
            println!("Starting daemon...");
            if let Some(pid) = pid_file.state()?.live_daemon() {
                println!("⚠️  Daemon is already running (PID {pid})");
                return Ok(());
            }
            let (pid, _) = spawn_daemon_process(&host, port)?;
            println!("✅ Daemon restarted!");
            println!("   PID: {pid}");
            println!("   Address: ws://{host}:{port}/ws");
        }
    }

    Ok(())
}

/// Spawn a daemon process in the background. Returns (PID, log file path).
///
/// The child claims the PID file itself, under the daemon's lock, once it
/// starts — or is refused if another daemon won the role in the meantime.
fn spawn_daemon_process(host: &str, port: u16) -> anyhow::Result<(u32, PathBuf)> {
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
        .arg(DAEMON_MODE_FLAG)
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
        .arg(DAEMON_MODE_FLAG)
        .arg("--host")
        .arg(host)
        .arg("--port")
        .arg(port.to_string())
        .stdout(Stdio::from(log_handle.try_clone()?))
        .stderr(Stdio::from(log_handle))
        .spawn()?;

    Ok((child.id(), log_file))
}

/// Stop the daemon the PID file names — only a process the daemon's own probe
/// calls a daemon: a PID since reused by another program is never signalled.
fn stop_daemon_process(pid_file: &PidFile) -> anyhow::Result<()> {
    let state = pid_file.state()?;
    let Some(pid) = state.live_daemon() else {
        println!("❌ No running daemon found");
        match stale_note(&state) {
            Some(note) => println!("   ({note})"),
            None => println!("   No PID file at {}", pid_file.path().display()),
        }
        return Ok(());
    };

    #[cfg(windows)]
    {
        use std::process::Command;
        // /T kills the daemon's whole tree, not just the daemon: without it,
        // in-flight exec children (powershell.exe/bash.exe) survived every
        // `nanna daemon stop`/`restart` and accumulated across dev cycles.
        // A current daemon's kill-on-close Job Object reaps them anyway;
        // /T is belt-and-braces for daemons where adoption failed.
        let status = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
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
        // A PID file value beyond `pid_t` is no process this command started;
        // wrapping it negative would signal a whole process group (-1: every
        // process we may signal), so it is reported instead of sent.
        match libc::pid_t::try_from(pid) {
            // SAFETY: kill(2) is safe to call with a valid PID and signal number
            Ok(target) if unsafe { libc::kill(target, libc::SIGTERM) } == 0 => {
                println!("✅ Daemon stopped (PID {pid})");
            }
            Ok(_) => {
                let err = std::io::Error::last_os_error();
                println!("⚠️  Failed to stop daemon (PID {pid}): {err}");
                println!("   It may have already been terminated");
            }
            Err(_) => {
                println!("⚠️  Failed to stop daemon (PID {pid}): not a valid process id");
                println!("   It may have already been terminated");
            }
        }
    }

    // The PID file is the daemon's to remove on its way out. One left behind
    // by a hard kill records a dead PID, which the next start takes over.
    Ok(())
}

/// Print daemon status information.
async fn print_daemon_status(pid_file: &PidFile) -> anyhow::Result<()> {
    use nanna_client::{Client, ClientConfig};

    println!("🌙 Nanna Daemon Status\n");

    let state = pid_file.state()?;
    let Some(pid) = state.live_daemon() else {
        match stale_note(&state) {
            Some(note) => println!("   Status: ❌ Not running ({note})"),
            None => println!("   Status: Not running"),
        }
        println!("   Start with: nanna daemon start");
        return Ok(());
    };

    println!("   Status: ✅ Running");
    println!("   PID: {pid}");
    if matches!(state, PidFileState::Recorded(_, ProcessProbe::Unknown)) {
        println!("   (alive, but its program could not be identified — treated as the daemon)");
    }

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
    Ok(())
}

/// Why a PID file that names no live daemon is stale; `None` when there is no
/// file, or when it does name a live daemon. A stale file is left in place:
/// the next daemon to start takes it over under the daemon's lock.
fn stale_note(state: &PidFileState) -> Option<String> {
    match state {
        PidFileState::Unparseable(content) => {
            Some(format!("stale PID file: it holds no PID ({content:?})"))
        }
        PidFileState::OwnPid(pid) | PidFileState::Recorded(pid, ProcessProbe::Dead) => {
            Some(format!("stale PID file: process {pid} has exited"))
        }
        PidFileState::Recorded(pid, ProcessProbe::Other) => {
            Some(format!("stale PID file: PID {pid} now belongs to another program"))
        }
        PidFileState::Absent
        | PidFileState::Recorded(_, ProcessProbe::Daemon | ProcessProbe::Unknown) => None,
    }
}
