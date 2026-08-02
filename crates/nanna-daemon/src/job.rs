//! Kill-on-close Job Object: children can never outlive the daemon.
//!
//! The exec tool and the acceptance runner deliberately spawn children in a
//! NEW process group (`CREATE_NEW_PROCESS_GROUP`) so a child that signals its
//! group can't take the daemon down with it. The cost of that isolation is
//! that when the daemon *process* dies — `taskkill /F`, a crash, a panic —
//! none of our cleanup code runs and the detached children survive (observed
//! live 2026-08-01: 89 leaked `powershell.exe` accumulated over days of
//! restarts).
//!
//! A Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` closes that hole at
//! the OS level: the daemon assigns *itself* to the job at startup, every
//! process it spawns joins the job automatically (job membership is
//! inherited; nothing in this repo uses `CREATE_BREAKAWAY_FROM_JOB`), and
//! when the daemon exits for ANY reason the kernel closes the daemon's job
//! handle and reaps the entire tree. There is no cleanup code path to forget.
//!
//! Job membership is orthogonal to console process groups, so this composes
//! with the `CREATE_NEW_PROCESS_GROUP` isolation in
//! `nanna-scripting/src/bridge.rs` — verified by
//! [`tests::job_close_kills_child_in_new_process_group`].

use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// Create an anonymous Job Object configured to kill every member process
/// when its last handle closes. Returns `None` if either Win32 call fails.
fn create_kill_on_close_job() -> Option<HANDLE> {
    // SAFETY: plain Win32 calls; the handle is closed on the failure path and
    // ownership is returned to the caller on success.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(info).cast(),
            u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                .expect("JOBOBJECT_EXTENDED_LIMIT_INFORMATION is a few hundred bytes"),
        );
        if ok == 0 {
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
#[must_use]
pub fn adopt_kill_on_close_job() -> bool {
    static ADOPTED: OnceLock<bool> = OnceLock::new();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::time::{Duration, Instant};

    // Mirrors the exec spawn path in nanna-scripting/src/bridge.rs.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// Reap bound: `TerminateProcess` via job close is near-instant; 5s is a
    /// generous ceiling for a loaded CI machine, polled at 50ms.
    const REAP_DEADLINE: Duration = Duration::from_secs(5);

    fn spawn_sleeper() -> std::process::Child {
        // cmd's `ping -n` is the cheapest reliable sleeper on Windows —
        // powershell takes seconds to boot and would dominate the deadline.
        std::process::Command::new("cmd.exe")
            .args(["/C", "ping -n 60 127.0.0.1 >nul"])
            .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn sleeper")
    }

    fn wait_reaped(child: &mut std::process::Child) -> bool {
        let deadline = Instant::now() + REAP_DEADLINE;
        while Instant::now() < deadline {
            if child.try_wait().expect("try_wait").is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }

    /// Closing the job handle must kill an assigned child even though the
    /// child sits in its own console process group — this is the composition
    /// (job ⊥ process group) the whole fix rests on.
    #[test]
    fn job_close_kills_child_in_new_process_group() {
        let job = create_kill_on_close_job().expect("create job");
        let mut child = spawn_sleeper();
        // SAFETY: both handles are valid; Child's raw handle stays owned by Child.
        let assigned = unsafe { AssignProcessToJobObject(job, child.as_raw_handle().cast()) };
        if assigned == 0 {
            let _ = child.kill();
            panic!("AssignProcessToJobObject failed");
        }

        // SAFETY: last handle to the job — close fires KILL_ON_JOB_CLOSE.
        unsafe { CloseHandle(job) };

        let reaped = wait_reaped(&mut child);
        if !reaped {
            let _ = child.kill();
        }
        assert!(reaped, "child survived job-handle close");
    }

    /// Job membership must be inherited: a grandchild spawned *by* a job
    /// member dies with the job too. This is what lets the daemon adopt one
    /// job at startup instead of chasing every spawn site.
    #[test]
    fn job_close_kills_grandchild() {
        let job = create_kill_on_close_job().expect("create job");
        let pid_file = std::env::temp_dir().join(format!(
            "nanna-job-test-grandchild-{}.pid",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&pid_file);

        // cmd spawns a detached ping (grandchild), records its PID, then
        // sleeps. wmic is dead on Win11, so recover the grandchild PID via
        // PowerShell's Get-CimInstance on the parent's children.
        let script = format!(
            "$p = Start-Process -WindowStyle Hidden -PassThru cmd.exe -ArgumentList '/C','ping -n 60 127.0.0.1 >nul'; \
             Set-Content -Path '{}' -Value $p.Id; Start-Sleep -Seconds 60",
            pid_file.display()
        );
        let mut child = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn powershell parent");
        // SAFETY: both handles are valid; Child's raw handle stays owned by Child.
        let assigned = unsafe { AssignProcessToJobObject(job, child.as_raw_handle().cast()) };
        if assigned == 0 {
            let _ = child.kill();
            panic!("AssignProcessToJobObject failed");
        }

        // Bound: powershell cold-start + Start-Process; 20s ceiling, 100ms poll.
        let deadline = Instant::now() + Duration::from_secs(20);
        let grandchild_pid = loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file)
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = std::fs::remove_file(&pid_file);
                panic!("grandchild never reported its PID");
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        assert!(
            crate::health::is_process_running(grandchild_pid),
            "grandchild should be alive before the job closes"
        );

        // SAFETY: last handle to the job — the whole tree must die.
        unsafe { CloseHandle(job) };

        let reaped = wait_reaped(&mut child);
        let deadline = Instant::now() + REAP_DEADLINE;
        let mut grandchild_reaped = false;
        while Instant::now() < deadline {
            if !crate::health::is_process_running(grandchild_pid) {
                grandchild_reaped = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::fs::remove_file(&pid_file);
        if !reaped {
            let _ = child.kill();
        }
        assert!(reaped, "child survived job-handle close");
        assert!(grandchild_reaped, "grandchild survived job-handle close");
    }

    /// Adoption must succeed on every supported Windows (nested jobs are a
    /// Windows 8 guarantee — CI runners already sit inside a job) and must be
    /// idempotent, since `Daemon::run` can be entered more than once in-process
    /// (tests, embedded builds).
    #[test]
    fn adopt_is_idempotent_and_succeeds() {
        let first = adopt_kill_on_close_job();
        let second = adopt_kill_on_close_job();
        assert!(first, "adoption failed — nested jobs unavailable?");
        assert_eq!(first, second, "adoption result must be stable");
    }
}
