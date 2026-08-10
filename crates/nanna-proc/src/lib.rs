//! Child-process lifetime containment, shared by the exec tool
//! (`nanna-scripting`), the acceptance runner (`nanna-agent`), and the daemon
//! (`nanna-daemon`). One implementation because the contract is subtle and a
//! previous copy-pair ("keep the two in sync") is exactly how the shapes below
//! drift apart.
//!
//! Three layers, one per failure mode:
//!
//! 1. [`kill_process_tree`] — the timeout path's walk: `taskkill /T /F` on
//!    Windows, `kill(-pgid, SIGKILL)` on Unix. The Windows walk follows
//!    parent links from the pid we spawned, so it is bounded by what it can
//!    see: once that shell is dead it finds nothing.
//! 2. [`ChildJob`] — a per-spawn kill-on-close Job Object (Windows-only; on
//!    Unix the process group already covers this). Job membership is
//!    inherited at process creation, so terminating the job reaps every
//!    descendant *including* one whose parent already exited — the `foo &`
//!    shape the walk cannot reach: the shell exits immediately, the
//!    backgrounded grandchild inherits the stdout/stderr write handles,
//!    `wait_with_output` never sees EOF, the timeout fires, and
//!    `taskkill /T` walks a dead pid. Observed live 2026-08-08: the
//!    surviving grandchild held the pipe, and because tokio's child-pipe
//!    reads on Windows are synchronous reads on the blocking pool, it pinned
//!    a blocking thread and hung runtime teardown for the sleeper's whole
//!    lifetime (25+ minutes).
//! 3. [`adopt_kill_on_close_job`] — the process-wide backstop: the daemon
//!    assigns *itself* to a kill-on-close job at startup, so no child ever
//!    outlives an unclean daemon death (`taskkill /F`, crash, panic).
//!
//! Job membership is orthogonal to console process groups, so all of this
//! composes with the `CREATE_NEW_PROCESS_GROUP` isolation at the spawn sites
//! (contract documented in `nanna-scripting/src/bridge.rs`) — verified by
//! `tests::job_close_kills_child_in_new_process_group`.

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// Kill a process and its descendants, best-effort.
///
/// On Windows this shells out to `taskkill /T /F`, which walks the tree from
/// `pid` — necessary because killing only the shell we spawned would leave a
/// grandchild (`cargo`, `git`) running and holding a workspace/build lock. On
/// Unix the child is its own process-group leader (`process_group(0)` at
/// spawn), so signalling the *negative* pid reaps the shell and every
/// descendant in one call — `kill_on_drop` alone only reached the direct
/// child and left grandchildren running.
///
/// The Windows walk cannot reach a descendant whose parent already exited
/// (the `foo &` shape); pair it with a [`ChildJob`] to cover that. The Unix
/// group kill has no such gap: background jobs of a non-interactive shell
/// stay in the shell's process group even after the shell dies.
pub async fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
    }
    #[cfg(not(windows))]
    {
        // pid is a fresh process-group id from the kernel: it fits i32, and a
        // failed signal (group already gone) is exactly the no-op we want.
        #[allow(clippy::cast_possible_wrap)]
        let pgid = -(pid as i32);
        // SAFETY: kill(2) with a negative pgid signals a process group we
        // created; `process_group(0)` at spawn put the child in its own
        // group, so this can never reach our own.
        unsafe {
            libc::kill(pgid, libc::SIGKILL);
        }
    }
}

/// Set a job's extended limit flags (the only limit this crate uses is
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`; pass 0 to clear). `true` on success.
#[cfg(windows)]
fn set_job_limit_flags(job: HANDLE, flags: u32) -> bool {
    // SAFETY: `job` is a valid job handle; zeroed extended info with only
    // LimitFlags set is the documented way to (re)configure limits.
    unsafe {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = flags;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(info).cast(),
            u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                .expect("JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a few hundred bytes"),
        ) != 0
    }
}

/// Create an anonymous Job Object configured to kill every member process
/// when its last handle closes. Returns `None` if either Win32 call fails.
#[cfg(windows)]
fn create_kill_on_close_job() -> Option<HANDLE> {
    // SAFETY: plain Win32 calls; the handle is closed on the failure path and
    // ownership is returned to the caller on success.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        if !set_job_limit_flags(job, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE) {
            CloseHandle(job);
            return None;
        }
        Some(job)
    }
}

/// Assign the current process to a kill-on-close Job Object.
///
/// Call once at daemon startup, before anything can spawn a child. Returns
/// whether containment is active; idempotent (repeat calls return the first
/// result). Failure is survivable — the daemon still runs, children just
/// aren't reaped on an unclean death — which is why callers log rather than
/// abort. The only realistic failure is `AssignProcessToJobObject` being
/// refused when the process already sits in a job on a pre-Windows-8 kernel
/// (no nested jobs); every supported platform nests fine.
#[cfg(windows)]
#[must_use]
pub fn adopt_kill_on_close_job() -> bool {
    static ADOPTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ADOPTED.get_or_init(|| {
        let Some(job) = create_kill_on_close_job() else {
            return false;
        };
        // SAFETY: `job` is a valid handle we own; the pseudo-handle from
        // GetCurrentProcess needs no closing.
        let ok = unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) };
        if ok == 0 {
            // SAFETY: we still own `job` on this path.
            unsafe { CloseHandle(job) };
            return false;
        }
        // Deliberately leak the handle: it must stay open for the whole
        // process lifetime, because the kernel closing it (at process exit)
        // is exactly the trigger that reaps our children.
        true
    })
}

/// A kill-on-close Job Object scoped to ONE spawned child and its descendants.
///
/// Assign immediately after spawn, before the child can realistically fork
/// (process initialization alone takes milliseconds; assignment is a single
/// syscall) — every process the child then starts inherits membership, and
/// nested jobs mean this composes with the daemon-wide job from
/// [`adopt_kill_on_close_job`]. Exactly one of three exits:
///
/// - [`ChildJob::terminate`] (timeout): every member dies now, including a
///   detached grandchild whose parent shell already exited — the tree
///   [`kill_process_tree`]'s parent-walk cannot reach. Held pipe handles
///   close, so a blocked `wait_with_output` read completes instead of
///   pinning a blocking-pool thread.
/// - [`ChildJob::disarm`] (normal completion): the command finished and its
///   pipes EOF'd, so anything still running is a deliberately detached
///   background process (output redirected — e.g. a dev server the agent
///   started on purpose). Let it run; the daemon-wide job still bounds it.
/// - drop (future cancelled, or an error path): kill-on-close fires and
///   reaps the whole tree — strictly better than the shell-only
///   `kill_on_drop` backstop, which left grandchildren running.
#[cfg(windows)]
pub struct ChildJob {
    handle: HANDLE,
}

// SAFETY: a Job Object HANDLE is a process-wide kernel handle, valid from any
// thread; nothing about ChildJob is tied to the thread that created it. The
// guard lives across `.await` points in the exec/acceptance futures, which
// must be Send.
#[cfg(windows)]
unsafe impl Send for ChildJob {}
// SAFETY: all methods take `self`/`&self` on an immutable handle; the kernel
// serializes job operations.
#[cfg(windows)]
unsafe impl Sync for ChildJob {}

#[cfg(windows)]
impl ChildJob {
    /// Create a kill-on-close job and assign `child` to it. `None` if the
    /// child already exited or a Win32 call fails — callers fall back to
    /// [`kill_process_tree`] alone (the pre-job behavior).
    #[must_use]
    pub fn assign(child: &tokio::process::Child) -> Option<Self> {
        let raw = child.raw_handle()?;
        let job = create_kill_on_close_job()?;
        // SAFETY: both handles are valid; the child's raw handle stays owned
        // by `child`.
        let ok = unsafe { AssignProcessToJobObject(job, raw.cast()) };
        if ok == 0 {
            // SAFETY: we own `job`, and no process joined it, so closing it
            // kills nothing.
            unsafe { CloseHandle(job) };
            return None;
        }
        Some(Self { handle: job })
    }

    /// Kill every process in the job, now. The exit code (1) is never
    /// observed — the caller is abandoning `wait_with_output`.
    pub fn terminate(self) {
        // SAFETY: `handle` is a live job handle we own; Drop closes it, which
        // (kill-on-close still set) doubles as a redundant kill trigger.
        let ok = unsafe { TerminateJobObject(self.handle, 1) };
        if ok == 0 {
            tracing::warn!("TerminateJobObject failed; relying on kill-on-close at handle close");
        }
    }

    /// Let surviving members run on: clear kill-on-close, then close the
    /// handle (the job object dissolves when its last member exits).
    pub fn disarm(self) {
        if !set_job_limit_flags(self.handle, 0) {
            // Closing now would kill survivors the caller chose to spare.
            // Leak the handle instead: it closes at OUR process exit, which
            // is the same bound the daemon-wide job already imposes.
            tracing::warn!("SetInformationJobObject failed; leaking job handle to spare survivors");
            std::mem::forget(self);
        }
    }
}

#[cfg(windows)]
impl Drop for ChildJob {
    fn drop(&mut self) {
        // SAFETY: we own the handle. With kill-on-close set, this close reaps
        // any remaining members — the cancellation path relies on it.
        unsafe { CloseHandle(self.handle) };
    }
}

/// On Unix the per-spawn containment is the process group created at spawn
/// (`process_group(0)`): background jobs of a non-interactive shell stay in
/// that group, so [`kill_process_tree`]'s `kill(-pgid, SIGKILL)` reaps them
/// even after the shell itself has exited. No job object exists or is
/// needed; [`ChildJob::assign`] always returns `None` and callers keep the
/// group kill as the single path.
#[cfg(not(windows))]
pub struct ChildJob {}

#[cfg(not(windows))]
impl ChildJob {
    /// Always `None`: see the type docs — the Unix process group already
    /// covers the detached-grandchild shape.
    #[must_use]
    pub fn assign(_child: &tokio::process::Child) -> Option<Self> {
        None
    }

    /// Unreachable (no value of `ChildJob` is ever constructed on Unix).
    pub fn terminate(self) {}

    /// Unreachable (no value of `ChildJob` is ever constructed on Unix).
    pub fn disarm(self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Reap bound: SIGKILL / taskkill / `TerminateJobObject` is near-instant;
    /// 5s is a generous ceiling for a loaded CI machine, polled at 50ms.
    const REAP_DEADLINE: Duration = Duration::from_secs(5);

    /// Grandchild-startup bound: powershell/sh must write its child's pid
    /// file well within this; dominated by powershell cold-start on Windows.
    const SPAWN_DEADLINE: Duration = Duration::from_secs(20);

    // Mirrors the exec/acceptance spawn paths (bridge.rs / harness.rs).
    #[cfg(windows)]
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    #[cfg(windows)]
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Liveness via `tasklist` — fine for a test; the daemon's runtime checks
    /// use Win32 directly (see nanna-daemon health.rs for why).
    #[cfg(windows)]
    fn windows_pid_alive(pid: u32) -> bool {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(&pid.to_string()))
    }

    /// Poll until `pid` is gone, bounded by [`REAP_DEADLINE`].
    #[cfg(windows)]
    fn wait_pid_dead(pid: u32) -> bool {
        let deadline = std::time::Instant::now() + REAP_DEADLINE;
        while std::time::Instant::now() < deadline {
            if !windows_pid_alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// PowerShell script that detaches a 60s ping (grandchild), records its
    /// Windows pid to `pid_file`, then runs `tail` (e.g. `Start-Sleep -Seconds
    /// 60` to stay alive, or `exit 0` to die immediately — the `foo &` shape).
    #[cfg(windows)]
    fn detach_ping_script(pid_file: &std::path::Path, tail: &str) -> String {
        format!(
            "$p = Start-Process -WindowStyle Hidden -PassThru cmd.exe -ArgumentList '/C','ping -n 60 127.0.0.1 >nul'; \
             Set-Content -Path '{}' -Value $p.Id; {tail}",
            pid_file.display()
        )
    }

    #[cfg(windows)]
    fn read_pid(pid_file: &std::path::Path) -> u32 {
        let deadline = std::time::Instant::now() + SPAWN_DEADLINE;
        loop {
            if let Ok(text) = std::fs::read_to_string(pid_file)
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                return pid;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "grandchild never reported its pid"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    // -----------------------------------------------------------------
    // kill_process_tree: the walk (moved from nanna-scripting bridge.rs
    // when the implementation moved here).
    // -----------------------------------------------------------------

    #[cfg(windows)]
    #[tokio::test]
    async fn kill_process_tree_reaps_shell_and_grandchild() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("grandchild.pid");

        // PowerShell parent spawns a detached ping sleeper (grandchild),
        // records its Windows pid, then sleeps — same shape as an exec'd
        // build command that forks a worker. The parent stays ALIVE, so the
        // taskkill walk can find the grandchild through it.
        let script = detach_ping_script(&pid_file, "Start-Sleep -Seconds 60");
        let mut cmd = tokio::process::Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        cmd.kill_on_drop(true);
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        let mut child = cmd.spawn().expect("spawn powershell parent");
        let pid = child.id().expect("pid of live child");

        let grandchild_pid = read_pid(&pid_file);

        kill_process_tree(pid).await;

        let deadline = tokio::time::Instant::now() + REAP_DEADLINE;
        loop {
            let shell_dead = child.try_wait().expect("try_wait").is_some();
            let grandchild_dead = !windows_pid_alive(grandchild_pid);
            if shell_dead && grandchild_dead {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "tree survived kill_process_tree (shell_dead={shell_dead}, grandchild_dead={grandchild_dead})"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn kill_process_tree_reaps_process_group() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("grandchild.pid");

        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(format!(
            "sleep 60 & echo $! > '{}'; sleep 60",
            pid_file.display()
        ));
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        cmd.kill_on_drop(true);
        // Same isolation the exec path uses — and what gives us a pgid to kill.
        cmd.process_group(0);
        let mut child = cmd.spawn().expect("spawn sh parent");
        let pid = child.id().expect("pid of live child");

        let deadline = tokio::time::Instant::now() + SPAWN_DEADLINE;
        let grandchild_pid = loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file)
                && let Ok(gc) = text.trim().parse::<i32>()
            {
                break gc;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "grandchild never reported its pid"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        };

        kill_process_tree(pid).await;

        // Reap the shell (a zombie would still answer kill(pid, 0)).
        let status = child.wait().await.expect("wait");
        assert!(!status.success(), "shell must have died by signal");

        // The grandchild reparents to init and is reaped there; poll bounded.
        let deadline = tokio::time::Instant::now() + REAP_DEADLINE;
        loop {
            // SAFETY: signal 0 probes existence only.
            let alive = unsafe { libc::kill(grandchild_pid, 0) } == 0;
            if !alive {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "grandchild survived process-group kill"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // -----------------------------------------------------------------
    // Job Object primitives (moved from nanna-daemon job.rs when the
    // implementation moved here).
    // -----------------------------------------------------------------

    /// Closing the job handle must kill an assigned child even though the
    /// child sits in its own console process group — this is the composition
    /// (job ⊥ process group) the whole design rests on.
    #[cfg(windows)]
    #[test]
    fn job_close_kills_child_in_new_process_group() {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::CommandExt;

        let job = create_kill_on_close_job().expect("create job");
        // cmd's `ping -n` is the cheapest reliable sleeper on Windows —
        // powershell takes seconds to boot and would dominate the deadline.
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/C", "ping -n 60 127.0.0.1 >nul"])
            .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn sleeper");
        // SAFETY: both handles are valid; Child's raw handle stays owned by Child.
        let assigned = unsafe { AssignProcessToJobObject(job, child.as_raw_handle().cast()) };
        if assigned == 0 {
            let _ = child.kill();
            panic!("AssignProcessToJobObject failed");
        }

        // SAFETY: last handle to the job — close fires KILL_ON_JOB_CLOSE.
        unsafe { CloseHandle(job) };

        let deadline = std::time::Instant::now() + REAP_DEADLINE;
        let mut reaped = false;
        while std::time::Instant::now() < deadline {
            if child.try_wait().expect("try_wait").is_some() {
                reaped = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if !reaped {
            let _ = child.kill();
        }
        assert!(reaped, "child survived job-handle close");
    }

    /// Adoption must succeed on every supported Windows (nested jobs are a
    /// Windows 8 guarantee — CI runners already sit inside a job) and must be
    /// idempotent, since `Daemon::run` can be entered more than once
    /// in-process (tests, embedded builds).
    #[cfg(windows)]
    #[test]
    fn adopt_is_idempotent_and_succeeds() {
        let first = adopt_kill_on_close_job();
        let second = adopt_kill_on_close_job();
        assert!(first, "adoption failed — nested jobs unavailable?");
        assert_eq!(first, second, "adoption result must be stable");
    }

    // -----------------------------------------------------------------
    // ChildJob: the per-spawn guard.
    // -----------------------------------------------------------------

    /// Spawn a tokio powershell child running `script`, with the same flags
    /// as the exec/acceptance spawn paths, and assign it a [`ChildJob`].
    #[cfg(windows)]
    fn spawn_with_job(script: &str) -> (tokio::process::Child, ChildJob) {
        let mut cmd = tokio::process::Command::new("powershell.exe");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        cmd.kill_on_drop(true);
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        let child = cmd.spawn().expect("spawn powershell");
        let job = ChildJob::assign(&child).expect("assign ChildJob");
        (child, job)
    }

    /// THE bug shape (2026-08-08): the shell detaches a grandchild and
    /// exits. `taskkill /T` from the dead shell's pid finds nothing —
    /// [`ChildJob::terminate`] must reap the grandchild anyway, because job
    /// membership was inherited when the shell was still a member.
    #[cfg(windows)]
    #[tokio::test]
    async fn terminate_reaps_detached_grandchild_of_dead_shell() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("grandchild.pid");
        let script = detach_ping_script(&pid_file, "exit 0");
        let (mut child, job) = spawn_with_job(&script);

        let grandchild_pid = read_pid(&pid_file);
        // The shell must be DEAD before we kill — that's what breaks the walk.
        let status = child.wait().await.expect("wait shell");
        assert!(status.success(), "detaching shell should exit cleanly");
        assert!(
            windows_pid_alive(grandchild_pid),
            "grandchild should have outlived its shell (test setup)"
        );

        // Deliberately NO kill_process_tree here: the walk is blind once the
        // shell is dead, and calling it would blur which mechanism killed the
        // grandchild. terminate() alone must be sufficient.
        job.terminate();

        assert!(
            wait_pid_dead(grandchild_pid),
            "detached grandchild survived ChildJob::terminate"
        );
    }

    /// The success path must NOT kill deliberate background survivors:
    /// [`ChildJob::disarm`] clears kill-on-close before the handle closes.
    #[cfg(windows)]
    #[tokio::test]
    async fn disarm_spares_detached_survivor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("grandchild.pid");
        let script = detach_ping_script(&pid_file, "exit 0");
        let (mut child, job) = spawn_with_job(&script);

        let grandchild_pid = read_pid(&pid_file);
        child.wait().await.expect("wait shell");

        job.disarm();
        // Give a kill (if disarm were broken) time to land before probing.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let survived = windows_pid_alive(grandchild_pid);
        // Cleanup before asserting so a failure doesn't leak a 60s ping.
        kill_process_tree(grandchild_pid).await;
        assert!(
            survived,
            "disarm must spare a deliberate background survivor"
        );
    }

    /// Dropping the guard (future cancelled / error path) must reap members —
    /// the exec/acceptance cancellation paths rely on this.
    #[cfg(windows)]
    #[tokio::test]
    async fn drop_reaps_live_member() {
        let (mut child, job) = spawn_with_job("Start-Sleep -Seconds 60");
        drop(job);
        let deadline = tokio::time::Instant::now() + REAP_DEADLINE;
        loop {
            if child.try_wait().expect("try_wait").is_some() {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "member survived ChildJob drop"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// On Unix the guard is deliberately absent; the call sites branch on
    /// `None` and keep the process-group kill as the single path.
    #[cfg(not(windows))]
    #[tokio::test]
    async fn assign_is_none_on_unix() {
        let child = tokio::process::Command::new("sh")
            .args(["-c", "true"])
            .spawn()
            .expect("spawn");
        assert!(ChildJob::assign(&child).is_none());
    }
}
