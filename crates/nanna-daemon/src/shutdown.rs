//! How a running daemon is told to stop from outside the process.
//!
//! Every daemon entry point installs these handlers: the `nanna-daemon` binary
//! (the GUI's sidecar) and `nanna --daemon-mode` (what `nanna daemon start`
//! launches). Until 2026-09-21 they lived in the binary's `main`, so the second
//! entry point had none. `nanna daemon stop` sends SIGTERM, which killed that
//! daemon with the default action: no drain, no PID-file release, and an exit
//! record left reading `running`.

use crate::server::DaemonServer;
use tracing::info;

/// Turn the process's termination requests into a graceful shutdown of
/// `daemon`: SIGTERM and SIGINT on Unix, Ctrl+C on Windows.
///
/// Each handler records `signal` (with the signal's name) in the exit-reason
/// file BEFORE requesting the drain. If the process is killed mid-drain, the
/// record says `signal`. If the drain completes, `run` overwrites it with
/// `clean_shutdown`. Either way it never reads `running` for a death a signal
/// started. (Recording is a no-op until `run` arms the file, so a duplicate
/// instance stopped while losing the instance claim cannot clobber the live
/// daemon's record.)
///
/// Call inside a Tokio runtime, before [`DaemonServer::run`]. A request that
/// arrives before `run` reaches its serve loop is kept for that loop, not lost
/// (see [`DaemonServer::shutdown_handle`]).
///
/// # Errors
///
/// On Unix, when a handler cannot be registered with the runtime's signal
/// driver.
pub fn install_signal_handlers(daemon: &DaemonServer) -> std::io::Result<()> {
    let shutdown = daemon.shutdown_handle();
    let exit_reason = daemon.exit_reason_handle();

    // Registered here, not in the task: once `signal` returns, the default
    // action is replaced, so a signal that arrives before the task first runs
    // is queued for it rather than killing the process.
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;

        tokio::spawn(async move {
            let name = tokio::select! {
                _ = sigterm.recv() => "SIGTERM",
                _ = sigint.recv() => "SIGINT",
            };
            info!("Received {name}");
            exit_reason.record_exit("signal", Some(name));
            let _ = shutdown.send(());
        });
    }

    #[cfg(windows)]
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        info!("Received Ctrl+C");
        exit_reason.record_exit("signal", Some("ctrl_c"));
        let _ = shutdown.send(());
    });

    Ok(())
}
