// An integration test is its own crate root: the daemon crate's
// `recursion_limit` does not reach it, and driving `DaemonServer::run` walks
// the same deep `Send` proof (see `crates/nanna-client/tests/e2e_daemon.rs`).
#![recursion_limit = "256"]

//! The daemon's single-instance guard, against real processes and a real `run()`.
//!
//! Incident (2026-09-16/17, Linux): a duplicate daemon was correctly refused,
//! then deleted the live daemon's PID file on its way out. Every later start
//! "acquired" the guard beside the running daemon, opened `nanna.db` against
//! its exclusive lock, built the LLM router and memory service, and died only
//! at the IPC bind. These tests pin the fix end to end: the probe tells a live
//! daemon from a dead or reused PID, a refused instance leaves the live
//! daemon's PID file alone, and `run()` refuses before it touches storage.
//!
//! The probe verdicts themselves (and ownership-aware release) are unit-tested
//! with an injected probe in `health.rs`; these tests stage the real thing.

use nanna_daemon::server::DaemonBuilder;
use nanna_daemon::{DaemonError, DaemonServer};
use std::path::Path;

/// A daemon that claims nothing beyond the guard under test: no memory, no
/// health or webhook ports, a private data dir. The PID file stays ON.
async fn hermetic_daemon(data_dir: &Path, port: u16) -> DaemonServer {
    DaemonBuilder::new()
        .with_host("127.0.0.1")
        .with_port(port)
        .with_data_dir(data_dir)
        .with_memory(false)
        .with_health_server(false)
        .with_webhook_server(false)
        .with_pid_file(true)
        .with_log_level("warn")
        .build()
        .await
}

/// A refused instance must leave no trace of having started: no store opened
/// (turso creates the file on open) and no exit record armed over the live
/// daemon's.
fn assert_refused_before_storage(data_dir: &Path) {
    for name in ["nanna.db", "nanna-daemon.exit.json"] {
        assert!(
            !data_dir.join(name).exists(),
            "{name} exists: the refused instance got past the instance claim"
        );
    }
}

#[tokio::test]
async fn run_refuses_a_held_ipc_port_before_touching_storage() {
    // Something else — typically a live daemon the PID file cannot vouch for —
    // listens on the IPC port.
    let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = holder.local_addr().unwrap().port();
    let data_dir = tempfile::tempdir().unwrap();

    let mut duplicate = hermetic_daemon(data_dir.path(), port).await;
    let result = duplicate.run().await;
    drop(duplicate);

    assert!(matches!(result, Err(DaemonError::Ipc(_))), "got {result:?}");
    assert_refused_before_storage(data_dir.path());
    // It did acquire the (empty) PID file — and, owning it, removed it.
    assert!(!data_dir.path().join("nanna-daemon.pid").exists());
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use nanna_daemon::health::{probe_process, PidFile, PidFileError, ProcessProbe};
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    /// Scaffolding, not a speed claim: bounds a hang waiting on the kernel to
    /// settle a killed child, sized far past any scheduling delay.
    const SETTLE_CEILING: Duration = Duration::from_secs(30);

    fn find_on_path(program: &str) -> PathBuf {
        let path = std::env::var_os("PATH").unwrap_or_default();
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
            .unwrap_or_else(|| panic!("`{program}` must be on PATH for these tests"))
    }

    /// A long-lived process whose executable FILE is named `name`: a copy of
    /// `sleep`. The kernel names a process (`comm`) after the executed file,
    /// while argv[0] stays `sleep` so multi-call coreutils builds still
    /// dispatch.
    struct Staged {
        child: Child,
        _dir: tempfile::TempDir,
    }

    impl Staged {
        fn spawn(name: &str) -> Self {
            // Beside the build output, which is executable by construction
            // (a system temp dir may be mounted noexec).
            let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
            let exe = dir.path().join(name);
            std::fs::copy(find_on_path("sleep"), &exe).unwrap();

            // Executing a just-written binary fails with ETXTBSY while a
            // sibling test thread's fork still holds the copy's descriptor
            // (until that child execs) — transient by construction.
            let deadline = Instant::now() + SETTLE_CEILING;
            let child = loop {
                match Command::new(&exe).arg0("sleep").arg("600").spawn() {
                    Ok(child) => break child,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(e) => panic!("spawning {exe:?} failed: {e}"),
                }
            };
            Self { child, _dir: dir }
        }

        fn pid(&self) -> u32 {
            self.child.id()
        }
    }

    impl Drop for Staged {
        fn drop(&mut self) {
            // Both are no-ops on a child already waited on.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn wait_for_proc_state(pid: u32, state: char) {
        let deadline = Instant::now() + SETTLE_CEILING;
        loop {
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
            let current = stat
                .rsplit(')')
                .next()
                .and_then(|rest| rest.trim_start().chars().next());
            if current == Some(state) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "process {pid} never reached state {state}: {stat:?}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn probe_names_a_live_daemon_by_its_executable() {
        let daemon = Staged::spawn("nanna-daemon");
        assert_eq!(probe_process(daemon.pid()), ProcessProbe::Daemon);

        // A sidecar that kept its target triple is a daemon too (comm keeps
        // only 15 bytes of the name, which still covers the prefix).
        let sidecar = Staged::spawn("nanna-daemon-x86_64-unknown-linux-gnu");
        assert_eq!(probe_process(sidecar.pid()), ProcessProbe::Daemon);
    }

    #[test]
    fn probe_reads_a_pid_reused_by_another_program_as_other() {
        let other = Staged::spawn("sleep");
        assert_eq!(probe_process(other.pid()), ProcessProbe::Other);
    }

    #[test]
    fn probe_reads_an_unreaped_zombie_and_a_reaped_pid_as_dead() {
        let mut daemon = Staged::spawn("nanna-daemon");
        let pid = daemon.pid();
        daemon.child.kill().unwrap();

        // Not reaped yet: a zombie still answers kill(pid, 0), which is how a
        // liveness-only check would read a crashed sidecar as a live daemon.
        wait_for_proc_state(pid, 'Z');
        assert_eq!(probe_process(pid), ProcessProbe::Dead);

        daemon.child.wait().unwrap();
        assert_eq!(probe_process(pid), ProcessProbe::Dead);
    }

    #[test]
    fn a_live_daemon_refuses_acquisition_and_keeps_its_pid_file() {
        let daemon = Staged::spawn("nanna-daemon");
        let data_dir = tempfile::tempdir().unwrap();
        let pid_path = data_dir.path().join("nanna-daemon.pid");
        std::fs::write(&pid_path, daemon.pid().to_string()).unwrap();

        let duplicate = PidFile::new(data_dir.path());
        assert!(matches!(
            duplicate.acquire(),
            Err(PidFileError::AlreadyRunning(pid)) if pid == daemon.pid()
        ));
        // Dropping the refused handle is the path that deleted the live
        // daemon's file in the incident.
        drop(duplicate);

        assert_eq!(std::fs::read_to_string(&pid_path).unwrap(), daemon.pid().to_string());
    }

    #[test]
    fn a_dead_daemons_pid_file_is_taken_over() {
        let mut daemon = Staged::spawn("nanna-daemon");
        let dead_pid = daemon.pid();
        daemon.child.kill().unwrap();
        daemon.child.wait().unwrap();

        let data_dir = tempfile::tempdir().unwrap();
        let pid_path = data_dir.path().join("nanna-daemon.pid");
        std::fs::write(&pid_path, dead_pid.to_string()).unwrap();

        let successor = PidFile::new(data_dir.path());
        successor.acquire().unwrap();
        assert_eq!(
            std::fs::read_to_string(&pid_path).unwrap(),
            std::process::id().to_string()
        );

        // The successor owns it now, so its shutdown removes it.
        drop(successor);
        assert!(!pid_path.exists());
    }

    #[tokio::test]
    async fn run_refuses_a_live_daemon_before_touching_storage() {
        let daemon = Staged::spawn("nanna-daemon");
        let data_dir = tempfile::tempdir().unwrap();
        let pid_path = data_dir.path().join("nanna-daemon.pid");
        std::fs::write(&pid_path, daemon.pid().to_string()).unwrap();

        // Hold the IPC port too: a refusal that reports the PID conflict (not
        // the port) proves the PID claim runs first.
        let holder = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = holder.local_addr().unwrap().port();

        let mut duplicate = hermetic_daemon(data_dir.path(), port).await;
        let result = duplicate.run().await;
        drop(duplicate);

        assert!(matches!(result, Err(DaemonError::AlreadyRunning)), "got {result:?}");
        assert_refused_before_storage(data_dir.path());
        assert_eq!(std::fs::read_to_string(&pid_path).unwrap(), daemon.pid().to_string());
    }
}
