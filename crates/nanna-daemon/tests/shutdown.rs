#![warn(clippy::pedantic, clippy::nursery, clippy::all)]
// An integration test is its own crate root: the daemon crate's
// `recursion_limit` does not reach it, and driving `DaemonServer::run` walks
// the same deep `Send` proof (see `crates/nanna-client/tests/e2e_daemon.rs`).
#![recursion_limit = "256"]

//! A daemon stops when it is asked to, whenever it is asked, against a real
//! `run()`.
//!
//! Two gaps found 2026-09-21 while converging `nanna --daemon-mode` onto this
//! crate's daemon path:
//! - `nanna --daemon-mode` had no signal handler, so `nanna daemon stop`
//!   killed it with SIGTERM's default action. It now installs the same
//!   handlers as the `nanna-daemon` binary, from `nanna_daemon::shutdown`.
//! - A shutdown requested during boot was lost. The serve loop subscribed to
//!   the shutdown broadcast only once the boot was over, and a broadcast
//!   reaches only existing receivers, so a SIGTERM the handler had caught
//!   during boot stopped nothing.
//!
//! Each test ends the same way: `run()` returns, the PID file is released,
//! and the exit record says `clean_shutdown`.

use nanna_daemon::exit_reason::{ExitReasonFile, ExitState, PreviousExit};
use nanna_daemon::server::DaemonBuilder;
use nanna_daemon::DaemonServer;
use std::path::Path;
use std::time::Duration;

/// Scaffolding, not a speed claim: bounds a hang, so a lost shutdown fails
/// the test instead of wedging the suite. Sized far past a hermetic boot and
/// drain, including the drain's own deadlines (stats save, MCP close).
const STOP_CEILING: Duration = Duration::from_secs(60);

/// A daemon that claims nothing beyond a private data dir: an OS-chosen IPC
/// port, no memory, no health or webhook port, no scheduler. The PID file
/// stays ON, since releasing it is part of stopping cleanly.
fn hermetic_daemon(data_dir: &Path) -> DaemonServer {
    DaemonBuilder::new()
        .with_host("127.0.0.1")
        .with_port(0)
        .with_data_dir(data_dir)
        .with_memory(false)
        .with_health_server(false)
        .with_webhook_server(false)
        .with_scheduler(false)
        .with_pid_file(true)
        .with_log_level("warn")
        .build()
}

fn assert_stopped_cleanly(data_dir: &Path) {
    assert!(
        !data_dir.join("nanna-daemon.pid").exists(),
        "the PID file was not released"
    );
    match ExitReasonFile::new(data_dir).read_previous() {
        PreviousExit::Record(record) => {
            assert_eq!(record.state, ExitState::Exited);
            assert_eq!(record.reason.as_deref(), Some("clean_shutdown"));
        }
        other => panic!("no terminal exit record: {other:?}"),
    }
}

#[tokio::test]
async fn a_shutdown_requested_during_boot_stops_the_daemon_once_it_has_booted() {
    let data_dir = tempfile::tempdir().unwrap();
    let mut daemon = hermetic_daemon(data_dir.path());

    // Before `run()`, so nothing inside it has subscribed. This is where a
    // SIGTERM lands when it arrives mid-boot.
    daemon
        .shutdown_handle()
        .send(())
        .expect("the serve loop's receiver is held from construction");

    let result = tokio::time::timeout(STOP_CEILING, daemon.run())
        .await
        .expect("the daemon never stopped: the shutdown request was lost");
    assert!(result.is_ok(), "{result:?}");
    drop(daemon);
    assert_stopped_cleanly(data_dir.path());
}

/// The only test in this binary that raises a signal. The handlers are
/// process-wide once installed, so no other test here may depend on SIGTERM.
#[cfg(unix)]
#[tokio::test]
async fn sigterm_drains_a_running_daemon() {
    let data_dir = tempfile::tempdir().unwrap();
    let mut daemon = hermetic_daemon(data_dir.path());
    nanna_daemon::shutdown::install_signal_handlers(&daemon)
        .expect("the signal handlers install");
    let mut bound = daemon.ipc_bound_addr();

    let result = tokio::time::timeout(STOP_CEILING, async {
        let run = daemon.run();
        tokio::pin!(run);
        // Booted = the IPC listener is bound.
        tokio::select! {
            result = &mut run => panic!("the daemon stopped before it was signalled: {result:?}"),
            ready = bound.wait_for(Option::is_some) => {
                drop(ready.expect("the IPC listener never bound"));
            }
        }
        // What `nanna daemon stop` sends. Without a handler this ends the
        // test process.
        let pid = libc::pid_t::try_from(std::process::id()).expect("a PID fits pid_t");
        // SAFETY: kill(2) on our own PID with a valid signal number; the
        // handler installed above catches it.
        assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0);
        run.await
    })
    .await
    .expect("the daemon never stopped after SIGTERM");

    assert!(result.is_ok(), "{result:?}");
    drop(daemon);
    assert_stopped_cleanly(data_dir.path());
}
