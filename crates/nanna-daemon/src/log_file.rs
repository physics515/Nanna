//! Rotating on-disk daemon logs.
//!
//! The in-memory [`crate::log_buffer`] serves the GUI's recent-logs view; this
//! module adds a *persistent*, size-bounded file log so a long-running daemon
//! leaves a reviewable trail without growing without bound. Files roll daily and
//! old ones are pruned so at most [`LOG_FILES_MAX`] remain on disk.

use std::path::{Path, PathBuf};
use tracing_appender::rolling::{RollingFileAppender, Rotation};

/// Filename prefix for rotated daemon log files (`nanna-daemon.YYYY-MM-DD.log`).
pub const LOG_FILE_PREFIX: &str = "nanna-daemon";

/// Filename suffix (extension) for rotated daemon log files.
pub const LOG_FILE_SUFFIX: &str = "log";

/// Maximum number of rotated log files kept on disk; older files are pruned by
/// the appender on rotation. Bounds unbounded log accumulation (P6).
pub const LOG_FILES_MAX: usize = 7;

/// Resolve the directory rotated logs are written to.
///
/// Precedence: an explicit `cli_log_dir` wins, otherwise `{data_dir}/logs`. The
/// result is always a dedicated sub-directory (never the data root itself) so
/// callers never scatter log files among data files.
#[must_use]
pub fn resolve_log_dir(cli_log_dir: Option<&Path>, data_dir: &Path) -> PathBuf {
    // Precondition: a caller-provided data_dir is required so we never default
    // logs into the process cwd.
    debug_assert!(
        !data_dir.as_os_str().is_empty(),
        "data_dir must be non-empty"
    );

    let dir = match cli_log_dir {
        Some(explicit) if !explicit.as_os_str().is_empty() => explicit.to_path_buf(),
        _ => data_dir.join("logs"),
    };

    // Postcondition: the resolved directory is always non-empty.
    debug_assert!(
        !dir.as_os_str().is_empty(),
        "resolved log dir must be non-empty"
    );
    dir
}

/// Build a daily-rotating file appender for `log_dir`, bounded to
/// [`LOG_FILES_MAX`] files. Creates `log_dir` if it does not exist.
///
/// # Errors
/// Returns an error string if `log_dir` can't be created or the rolling
/// appender can't be initialized, so the caller can fall back to console-only
/// logging instead of aborting startup.
///
/// # Panics
/// Panics if `log_dir` is empty; callers pass a resolved, non-empty path from
/// [`resolve_log_dir`].
pub fn build_appender(log_dir: &Path) -> Result<RollingFileAppender, String> {
    assert!(!log_dir.as_os_str().is_empty(), "log_dir must be non-empty");

    // The appender creates missing parents, but create eagerly so a bad path
    // surfaces here (fallback to console) rather than on the first write.
    std::fs::create_dir_all(log_dir)
        .map_err(|e| format!("create log dir {}: {e}", log_dir.display()))?;

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(LOG_FILES_MAX)
        .build(log_dir)
        .map_err(|e| format!("build rolling appender: {e}"))?;

    Ok(appender)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn resolve_prefers_explicit_dir() {
        let explicit = PathBuf::from("/var/log/nanna");
        let data = PathBuf::from("/data/nanna");
        assert_eq!(resolve_log_dir(Some(&explicit), &data), explicit);
    }

    #[test]
    fn resolve_falls_back_to_data_logs_subdir() {
        let data = PathBuf::from("/data/nanna");
        assert_eq!(resolve_log_dir(None, &data), data.join("logs"));
    }

    #[test]
    fn resolve_ignores_empty_explicit_dir() {
        let empty = PathBuf::new();
        let data = PathBuf::from("/data/nanna");
        // An empty override must not win — fall through to the data subdir.
        assert_eq!(resolve_log_dir(Some(&empty), &data), data.join("logs"));
    }

    #[test]
    fn appender_creates_dir_and_writes_a_prefixed_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let log_dir = tmp.path().join("logs");
        let mut appender = build_appender(&log_dir).expect("appender");
        writeln!(appender, "hello nanna").expect("write");
        appender.flush().expect("flush");

        assert!(log_dir.is_dir(), "log dir should be created");
        let mut names: Vec<String> = std::fs::read_dir(&log_dir)
            .expect("read_dir")
            .filter_map(std::result::Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 1, "exactly one log file, got {names:?}");
        assert!(
            names[0].starts_with(LOG_FILE_PREFIX) && names[0].ends_with(LOG_FILE_SUFFIX),
            "log file name {} should carry the prefix+suffix",
            names[0]
        );
    }
}
