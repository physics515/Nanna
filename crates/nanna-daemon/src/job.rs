//! Kill-on-close Job Object: children can never outlive the daemon.
//!
//! The daemon assigns *itself* to a kill-on-close Job Object at startup
//! (`Daemon::run`), so every exec/acceptance child inherits membership and
//! the kernel reaps the whole tree when the daemon dies for ANY reason —
//! observed live 2026-08-01: 89 leaked `powershell.exe` accumulated over
//! days of restarts before this existed.
//!
//! The implementation (and its composition tests with the
//! `CREATE_NEW_PROCESS_GROUP` spawn isolation) lives in `nanna-proc`, shared
//! with the per-child `ChildJob` containment used by the exec tool
//! (`nanna-scripting/src/bridge.rs`) and the acceptance runner
//! (`nanna-agent/src/harness.rs`). This module remains so
//! `crate::job::adopt_kill_on_close_job` keeps resolving.

pub use nanna_proc::adopt_kill_on_close_job;
