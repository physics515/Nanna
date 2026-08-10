//! Bridge between Nanna and scripted tools
//!
//! Exposes controlled Nanna functionality to JavaScript code.

use crate::{Result, ScriptError, tool::ToolPermissions};
use nanna_proc::{ChildJob, kill_process_tree};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};

use std::sync::Mutex as StdMutex;
use std::collections::HashSet as StdHashSet;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

/// A service function callable from scripts via `Nanna.service(name, params)`.
///
/// Each closure receives a JSON `Value` of params and returns a JSON `Value` result.
pub type ServiceFn = Arc<
    dyn Fn(Value) -> Pin<Box<dyn Future<Output = std::result::Result<Value, String>> + Send>>
        + Send
        + Sync,
>;

/// A synchronous ranked tool-search function backing `Nanna.searchTools(query)`.
///
/// Receives `(query, limit)` and returns a JSON array of
/// `{name, description, score}` hits, best first. Implementations must never
/// panic and should return an empty array when the underlying registry is
/// gone — the script-side contract is "empty results, never a throw".
pub type ToolSearchFn = Arc<dyn Fn(&str, usize) -> Value + Send + Sync>;

/// Default number of hits `Nanna.searchTools` returns when the script passes
/// no explicit limit — enough for a discovery response without flooding a
/// small model's context.
pub const DEFAULT_TOOL_SEARCH_LIMIT: usize = 10;

/// Bridge providing Nanna capabilities to scripts
#[derive(Clone)]
pub struct NannaBridge {
    pub permissions: ToolPermissions,
    http_client: reqwest::Client,
    logs: Arc<RwLock<Vec<LogEntry>>>,
    /// Tool definitions for `Nanna.listTools()` — set by the script engine when available
    tool_definitions: Option<Value>,
    /// Ranked tool search for `Nanna.searchTools()` — set by the script engine when available
    tool_search: Option<ToolSearchFn>,
    /// Service functions callable via `Nanna.service(name, params)`
    services: Arc<HashMap<String, ServiceFn>>,
    /// Default working directory for exec commands (overrides home dir fallback)
    default_workdir: Option<PathBuf>,
    /// Current session ID for session-scoped operations
    session_id: Option<String>,
}


/// Global advisory lock set for file writes.
/// Prevents concurrent sub-agents from writing to the same file simultaneously.
static FILE_WRITE_LOCKS: std::sync::LazyLock<StdMutex<StdHashSet<std::path::PathBuf>>> =
    std::sync::LazyLock::new(|| StdMutex::new(StdHashSet::new()));

/// Which Windows shell to run a model-issued command through.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WinShell {
    /// Explicit PowerShell — `powershell`/`pwsh` prefix, `$env:`, or a `Verb-Noun` cmdlet.
    PowerShell,
    /// The default. Models write POSIX/bash commands (`[ -f ]`, `cat`, `tail`, `&&`,
    /// `|`, forward-slash paths); we run those through Git Bash when available.
    Bash,
}

/// Classify a Windows command as explicit PowerShell, else Bash (the default).
///
/// Pure (no IO). `trimmed` must already be left-trimmed. Only *unambiguous*
/// PowerShell markers force PowerShell; everything else defaults to Bash, because
/// the previous `$ && |` heuristic mis-routed ordinary bash (`cat $f | grep`) to
/// PowerShell and broke it.
#[cfg(windows)]
fn classify_windows_command(trimmed: &str) -> WinShell {
    let is_powershell = trimmed.starts_with("powershell")
        || trimmed.starts_with("pwsh")
        || trimmed.contains("$env:")
        || trimmed.contains("Get-")
        || trimmed.contains("Set-")
        || trimmed.contains("New-")
        || trimmed.contains("Remove-")
        || trimmed.contains("Select-")
        || trimmed.contains("Where-Object")
        || trimmed.contains("ForEach-Object");
    if is_powershell {
        WinShell::PowerShell
    } else {
        WinShell::Bash
    }
}

/// Translate `X:\a\b` drive paths into the `/x/a/b` form Git Bash understands.
///
/// In bash a backslash is an ESCAPE, so `cat > D:\Development\ws\minidb.sh`
/// silently becomes `cat > D:Developmentwsminidb.sh` — a file with a garbage
/// name, created without an error anyone can see. Observed live 2026-07-27: a
/// run's largest and newest work (3 KB) landed in a file literally named
/// `D:Developmentnanna-benchrun-qwen3-5-9bminidb.sh`, while the artifact the
/// acceptance tests read sat frozen and half-finished.
///
/// This is the shell-layer half of "handle absolute and relative paths
/// gracefully" — the tool layer already repairs argument paths; a command
/// string needs the same courtesy, because the model cannot tell that the
/// path form it was shown (native, with backslashes) is not the form the
/// shell accepts.
///
/// Narrow by construction:
/// - Only a drive-letter prefix (`X:\` or `X:/`) at a token boundary starts a
///   rewrite, so `\d`, `\n`, `C:` alone, and Windows paths inside SINGLE
///   quotes (bash's literal string) are untouched.
/// - The rewrite ends at whitespace or a shell metacharacter, so redirections
///   and pipes survive.
/// - Forward slashes are already valid; those paths are normalized to `/x/…`
///   too, which is the same location.
#[cfg(windows)]
fn normalize_drive_paths(command: &str) -> std::borrow::Cow<'_, str> {
    let b = command.as_bytes();
    let n = b.len();
    // Fast path: a drive path needs a colon.
    if !command.contains(':') {
        return std::borrow::Cow::Borrowed(command);
    }

    let mut out = String::with_capacity(n);
    let mut changed = false;
    let mut i = 0usize;
    let mut single_quote = false;
    let mut prev_boundary = true;

    while i < n {
        let c = b[i];
        if c == b'\'' {
            single_quote = !single_quote;
            out.push('\'');
            i += 1;
            prev_boundary = false;
            continue;
        }
        // A literal single-quoted string is exactly what a careful author uses
        // to protect a Windows path; never second-guess it.
        if single_quote {
            out.push(c as char);
            i += 1;
            continue;
        }

        let starts_drive = prev_boundary
            && c.is_ascii_alphabetic()
            && i + 2 < n
            && b[i + 1] == b':'
            && (b[i + 2] == b'\\' || b[i + 2] == b'/');

        if starts_drive {
            let drive = (c as char).to_ascii_lowercase();
            out.push('/');
            out.push(drive);
            i += 2; // skip "X:"
            while i < n {
                let p = b[i];
                if p == b' ' || p == b'\t' || p == b'"' || p == b'\'' || p == b'|'
                    || p == b'>' || p == b'<' || p == b'&' || p == b';' || p == b'\n'
                {
                    break;
                }
                out.push(if p == b'\\' { '/' } else { p as char });
                i += 1;
            }
            changed = true;
            prev_boundary = false;
            continue;
        }

        out.push(c as char);
        prev_boundary = matches!(c, b' ' | b'\t' | b'"' | b'|' | b'&' | b';' | b'(' | b'\n' | b'=');
        i += 1;
    }

    if changed {
        std::borrow::Cow::Owned(out)
    } else {
        std::borrow::Cow::Borrowed(command)
    }
}

#[cfg(not(windows))]
fn normalize_drive_paths(command: &str) -> std::borrow::Cow<'_, str> {
    std::borrow::Cow::Borrowed(command)
}

/// Rewrite the small set of cmd.exe idioms that Git Bash rejects.
///
/// Currently only `cd /d <path>` → `cd <path>`. `/d` is a cmd.exe-only flag
/// ("also change drive"); Git Bash's `cd` treats it as a second argument and
/// dies with "cd: too many arguments" (observed in a real run log). We strip
/// `/d` (case-insensitive) ONLY when it is a whole token immediately after a
/// command-boundary `cd` AND a further token follows — the exact cmd idiom. A
/// bare `cd /d`, `cd /data`, `cd /d/x`, a quoted occurrence, or a non-boundary
/// `cd` are all left alone, so no currently-working command changes. Handles the
/// idiom at every command boundary in a compound command (`cd /d D:\x && …`).
///
/// Pure (no IO). Windows-only: on Unix `cd /d` legitimately means dir `/d`.
#[cfg(windows)]
fn normalize_cmdisms(command: &str) -> std::borrow::Cow<'_, str> {
    if !(command.contains("/d") || command.contains("/D")) {
        return std::borrow::Cow::Borrowed(command);
    }
    let b = command.as_bytes();
    let n = b.len();
    let mut out: Vec<u8> = Vec::with_capacity(n);
    let mut changed = false;
    let mut i = 0usize;
    let mut at_boundary = true; // start of string is a command boundary
    let mut quote: u8 = 0; // 0 = none, else b'\'' / b'"'
    while i < n {
        let c = b[i];
        if quote != 0 {
            out.push(c);
            if c == quote {
                quote = 0;
            }
            i += 1;
            continue;
        }
        if c == b'\'' || c == b'"' {
            quote = c;
            out.push(c);
            i += 1;
            at_boundary = false;
            continue;
        }
        if at_boundary {
            if c == b' ' || c == b'\t' {
                out.push(c);
                i += 1;
                continue;
            }
            if let Some(path_off) = match_cd_slash_d(&b[i..]) {
                out.extend_from_slice(b"cd ");
                i += path_off; // resume at the path token
                changed = true;
                at_boundary = false;
                continue;
            }
            at_boundary = false;
        }
        out.push(c);
        // These open a new command boundary for the NEXT char.
        if matches!(c, b';' | b'\n' | b'(' | b'|' | b'&') {
            at_boundary = true;
        }
        i += 1;
    }
    if changed {
        // Safe: we only ever copied original bytes verbatim plus ASCII "cd ".
        std::borrow::Cow::Owned(String::from_utf8(out).unwrap_or_else(|_| command.to_string()))
    } else {
        std::borrow::Cow::Borrowed(command)
    }
}

/// If `s` begins with the cmd idiom `cd /d <path…>`, return the byte offset of
/// the path token (so the caller can emit `cd ` then resume there). Requires
/// `cd`+ws, then `/d` or `/D` as a whole token (ws after), then a following
/// token. Otherwise `None`.
#[cfg(windows)]
fn match_cd_slash_d(s: &[u8]) -> Option<usize> {
    // `cd` (case-insensitive) then whitespace.
    if s.len() < 3 {
        return None;
    }
    if !(s[0] | 0x20 == b'c' && s[1] | 0x20 == b'd') {
        return None;
    }
    if !(s[2] == b' ' || s[2] == b'\t') {
        return None;
    }
    let mut j = 3;
    while j < s.len() && (s[j] == b' ' || s[j] == b'\t') {
        j += 1;
    }
    // `/d` or `/D` as a whole token.
    if j + 2 >= s.len() {
        return None;
    }
    if s[j] != b'/' || (s[j + 1] | 0x20) != b'd' {
        return None;
    }
    if !(s[j + 2] == b' ' || s[j + 2] == b'\t') {
        return None; // must be a whole token, not `/data` or `/d/x`
    }
    let mut k = j + 3;
    while k < s.len() && (s[k] == b' ' || s[k] == b'\t') {
        k += 1;
    }
    if k >= s.len() {
        return None; // no path token follows → not the idiom
    }
    Some(k)
}

/// Locate Git-for-Windows `bash.exe`, cached. Explicitly **not** WSL's
/// `C:\Windows\System32\bash.exe` (different filesystem semantics), only a real
/// Git Bash under a `Git\bin` install. Returns `None` if Git Bash isn't installed.
#[cfg(windows)]
fn git_bash_path() -> Option<&'static Path> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let mut candidates: Vec<PathBuf> = Vec::new();
            for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                if let Ok(base) = std::env::var(var) {
                    candidates.push(PathBuf::from(base).join("Git").join("bin").join("bash.exe"));
                }
            }
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                candidates
                    .push(PathBuf::from(local).join("Programs").join("Git").join("bin").join("bash.exe"));
            }
            candidates.into_iter().find(|p| p.is_file())
        })
        .as_deref()
}

/// Default per-command exec timeout, in seconds, for an unremarkable command.
pub const EXEC_DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Auto-detected exec timeout, in seconds, for build/VCS tools (git, cargo,
/// npm, …) which routinely exceed [`EXEC_DEFAULT_TIMEOUT_SECS`] on large repos.
/// This is the widest window the bridge picks on its own; a caller can still
/// request more explicitly.
pub const EXEC_MAX_AUTODETECT_SECS: u64 = 120;

/// The per-command execution timeout (seconds) to use when the caller gives no
/// explicit override, auto-detected from the command text.
///
/// Pure (no IO). Extracted from `exec_with_timeout` so the outer script-engine
/// deadline can be sized against the *same* value the bridge will actually
/// enforce, and so the classification is unit-testable.
#[must_use]
pub fn default_exec_timeout_secs(command: &str) -> u64 {
    let cmd_lower = command.to_lowercase();
    if cmd_lower.starts_with("git ")
        || cmd_lower.starts_with("cargo ")
        || cmd_lower.starts_with("npm ")
        || cmd_lower.starts_with("pnpm ")
        || cmd_lower.starts_with("yarn ")
        || cmd_lower.starts_with("pip ")
        || cmd_lower.starts_with("make ")
        || cmd_lower.starts_with("cmake ")
        || cmd_lower.starts_with("dotnet ")
        || cmd_lower.starts_with("mvn ")
        || cmd_lower.starts_with("gradle ")
        || cmd_lower.contains("cargo ")
        || cmd_lower.contains("git checkout")
        || cmd_lower.contains("git clone")
        || cmd_lower.contains("git fetch")
        || cmd_lower.contains("git pull")
        || cmd_lower.contains("cargo build")
        || cmd_lower.contains("cargo check")
        || cmd_lower.contains("cargo clippy")
        || cmd_lower.contains("cargo test")
    {
        EXEC_MAX_AUTODETECT_SECS
    } else {
        EXEC_DEFAULT_TIMEOUT_SECS
    }
}

impl NannaBridge {
    /// Create a new bridge with the given permissions
    pub fn new(permissions: ToolPermissions) -> Self {
        Self {
            permissions,
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            logs: Arc::new(RwLock::new(Vec::new())),
            tool_definitions: None,
            tool_search: None,
            services: Arc::new(HashMap::new()),
            default_workdir: None,
            session_id: None,
        }
    }

    /// Set the default working directory for exec commands.
    /// When set, this overrides the home directory fallback.
    #[must_use]
    pub fn with_default_workdir(mut self, workdir: impl Into<PathBuf>) -> Self {
        self.default_workdir = Some(workdir.into());
        self
    }

    /// Set the session ID for session-scoped operations.
    #[must_use]
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Get the current session ID.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    /// Get the default working directory as a string.
    pub fn workdir(&self) -> Option<&str> {
        self.default_workdir.as_ref().and_then(|p| p.to_str())
    }

    /// Set tool definitions for `Nanna.listTools()`
    #[must_use]
    pub fn with_tool_definitions(mut self, defs: Value) -> Self {
        self.tool_definitions = Some(defs);
        self
    }

    /// Set the ranked tool-search function for `Nanna.searchTools()`
    #[must_use]
    pub fn with_tool_search(mut self, search: ToolSearchFn) -> Self {
        self.tool_search = Some(search);
        self
    }

    /// Resolve a path: expand `~` to home directory, resolve relative paths against
    /// the default working directory (if set) or home dir.
    /// Works on Windows, macOS, Linux, Android, and iOS.
    /// Falls back to the raw path if no base directory can be determined.
    fn resolve_path_with_workdir(path: &str, default_workdir: Option<&Path>) -> PathBuf {
        let path = path.trim();
        
        // Expand ~ or ~/ (but not ~username which we don't support)
        if path == "~" || path.starts_with("~/") || path.starts_with("~\\") {
            if let Some(home) = Self::home_dir() {
                let rest = path.strip_prefix("~/")
                    .or_else(|| path.strip_prefix("~\\"))
                    .unwrap_or("");
                return home.join(rest);
            }
        }
        
        let p = Path::new(path);
        if p.is_relative() && !path.is_empty() {
            // Resolve relative to the active workspace working directory. The
            // registry seeds `default_workdir` from the active workspace (at boot,
            // on switch, and per-chat), so bare relative paths land in the workspace
            // the user is in — not the home dir. Home is only a last resort when
            // there is genuinely no active workspace (global mode). `~`-prefixed
            // paths were already expanded above.
            if let Some(wd) = default_workdir {
                let literal = wd.join(p);
                return Self::repair_redundant_prefix(p, wd).unwrap_or(literal);
            }
            if let Some(home) = Self::home_dir() {
                return home.join(p);
            }
        }
        p.to_path_buf()
    }

    /// Repair a relative path that redundantly repeats the tail of the working
    /// directory, e.g. `minidb-gui-qwen35/minidb` given a workdir that already
    /// ends in `minidb-gui-qwen35`.
    ///
    /// A model that learns its location (from `pwd`, a prompt line, or an
    /// earlier tool result) will sometimes address files by re-appending part
    /// of that path to a cwd that already includes it. Taken literally that
    /// silently creates a shadow tree one level deeper — observed live
    /// 2026-07-26: an hour of edits landed in `<ws>/<ws-leaf>/minidb` while the
    /// acceptance tests read `<ws>/minidb`, and the run scored 0.
    ///
    /// Rules, in order, so this never overrides a real file:
    /// 1. If the literal join exists, it wins — a genuine nested directory of
    ///    the same name is still addressable.
    /// 2. Otherwise, for the longest overlap first, if the de-duplicated
    ///    target exists, use it (reads and edits find the intended file).
    /// 3. Otherwise, if the de-duplicated *parent* exists while the literal
    ///    parent does not, use it (a write lands beside the files it belongs
    ///    with instead of conjuring a duplicate directory).
    ///
    /// Returns `None` when nothing needs repairing, leaving the literal join.
    fn repair_redundant_prefix(rel: &Path, workdir: &Path) -> Option<PathBuf> {
        let literal = workdir.join(rel);
        if literal.exists() {
            return None;
        }

        let rel_parts: Vec<_> = rel.components().collect();
        let wd_parts: Vec<_> = workdir.components().collect();
        // The overlap can cover at most all but the last component of the
        // relative path (something must remain to name the target), and at
        // most the whole working directory.
        let max_overlap = rel_parts.len().saturating_sub(1).min(wd_parts.len());

        let mut fallback = None;
        for k in (1..=max_overlap).rev() {
            let head = &rel_parts[..k];
            let wd_tail = &wd_parts[wd_parts.len() - k..];
            if head != wd_tail {
                continue;
            }
            let remainder: PathBuf = rel_parts[k..].iter().collect();
            let candidate = workdir.join(&remainder);
            if candidate.exists() {
                return Some(candidate);
            }
            if fallback.is_none() {
                let literal_parent_missing =
                    literal.parent().is_some_and(|p| !p.exists());
                let candidate_parent_exists =
                    candidate.parent().is_some_and(Path::exists);
                if literal_parent_missing && candidate_parent_exists {
                    fallback = Some(candidate);
                }
            }
        }
        fallback
    }

    /// Resolve a path using the instance's default working directory.
    fn resolve_path(&self, path: &str) -> PathBuf {
        Self::resolve_path_with_workdir(path, self.default_workdir.as_deref())
    }

    /// Get the user's home directory, cross-platform.
    /// Uses `directories` crate which supports Windows, macOS, Linux.
    /// On Android/iOS, falls back to current directory.
    fn home_dir() -> Option<PathBuf> {
        directories::BaseDirs::new()
            .map(|d| d.home_dir().to_path_buf())
            .or_else(|| std::env::current_dir().ok())
    }

    /// Set service functions for `Nanna.service()`
    #[must_use]
    pub fn with_services(mut self, services: HashMap<String, ServiceFn>) -> Self {
        self.services = Arc::new(services);
        self
    }

    /// Get stored tool definitions (for `Nanna.listTools()`)
    pub fn list_tools(&self) -> Option<&Value> {
        self.tool_definitions.as_ref()
    }

    /// Run the ranked tool search (for `Nanna.searchTools()`).
    ///
    /// Returns `None` when no search function was attached (older embedding
    /// contexts, test harnesses) — the caller maps that to an empty array,
    /// never a thrown error.
    #[must_use]
    pub fn search_tools(&self, query: &str, limit: usize) -> Option<Value> {
        self.tool_search.as_ref().map(|f| f(query, limit))
    }

    /// Call a registered service by name
    pub async fn call_service(&self, name: &str, params: Value) -> std::result::Result<Value, String> {
        let service = self.services.get(name)
            .ok_or_else(|| format!("Service not found: {name}"))?;
        service(params).await
    }

    /// Execute a shell command (if permitted)
    ///
    /// `timeout_secs`: optional override for the execution timeout (default: 30s).
    pub async fn exec(&self, command: &str, workdir: Option<&str>) -> Result<ExecResponse> {
        self.exec_with_timeout(command, workdir, None).await
    }

    /// Execute a shell command with an optional timeout override.
    pub async fn exec_with_timeout(&self, command: &str, workdir: Option<&str>, timeout_secs: Option<u64>) -> Result<ExecResponse> {
        tracing::debug!(target: "bridge", "exec called: command={}, run_permission={}", command, self.permissions.run);
        
        if !self.permissions.run {
            tracing::warn!(target: "bridge", "Shell execution denied - run permission is false");
            return Err(ScriptError::Permission(
                "Shell execution not permitted. Tool needs 'run' permission.".to_string(),
            ));
        }

        // On mobile platforms, shell exec is not available
        if cfg!(target_os = "android") || cfg!(target_os = "ios") {
            return Err(ScriptError::Permission(
                "Shell execution is not available on mobile platforms".to_string(),
            ));
        }

        // Rewrite cmd.exe idioms Git Bash rejects (e.g. `cd /d`) before routing,
        // so a model that emits cmd.exe habits doesn't hit "cd: too many arguments".
        // Every downstream use of `command` (python detect, classify, shell args,
        // timeout auto-detect, tracing) transparently sees the normalized string.
        #[cfg(windows)]
        let normalized = normalize_cmdisms(command);
        #[cfg(windows)]
        let command: &str = normalized.as_ref();
        // Then translate `D:\ws\file` → `/d/ws/file`, because bash reads those
        // backslashes as escapes and would write a garbage-named file instead
        // of failing loudly.
        #[cfg(windows)]
        let drive_normalized = normalize_drive_paths(command);
        #[cfg(windows)]
        let command: &str = drive_normalized.as_ref();

        // Determine shell based on OS.
        // On Windows, route commands to the appropriate shell:
        // - `python[3] -c "..."` one-liners → python directly (cmd/quote mangling)
        // - explicit PowerShell (`$env:`, `Verb-Noun` cmdlets, `powershell`/`pwsh`) → powershell.exe
        // - everything else → Git Bash (models write POSIX/bash by default), falling
        //   back to cmd.exe only when Git Bash isn't installed.
        let mut cmd = if cfg!(windows) {
            let trimmed = command.trim_start();

            // Detect `python[3] -c "..."` one-liners (route directly to avoid quote
            // mangling). Compound commands (&&, ||) are handled by the shell below.
            let is_python_c = (trimmed.starts_with("python3 -c") || trimmed.starts_with("python -c"))
                && !trimmed.contains("&&")
                && !trimmed.contains("||");

            if is_python_c {
                // Parse: "python[3] -c 'code'" or 'python[3] -c "code"'
                let (exe, rest) = if trimmed.starts_with("python3") {
                    ("python3", trimmed.strip_prefix("python3 -c").unwrap_or("").trim())
                } else {
                    ("python", trimmed.strip_prefix("python -c").unwrap_or("").trim())
                };
                // Strip outer quotes if present
                let code = if (rest.starts_with('"') && rest.ends_with('"'))
                    || (rest.starts_with('\'') && rest.ends_with('\''))
                {
                    &rest[1..rest.len() - 1]
                } else {
                    rest
                };
                let mut c = tokio::process::Command::new(exe);
                c.args(["-c", code]);
                c
            } else {
                match classify_windows_command(trimmed) {
                    WinShell::PowerShell => {
                        let mut c = tokio::process::Command::new("powershell.exe");
                        c.args(["-NoProfile", "-NonInteractive", "-Command", command]);
                        c
                    }
                    WinShell::Bash => {
                        // The model writes POSIX/bash — run through Git Bash if present
                        // (handles `[ -f ]`, cat, tail, &&, |, quoting, forward-slash
                        // paths); fall back to cmd.exe only when Git Bash isn't installed.
                        if let Some(bash) = git_bash_path() {
                            let mut c = tokio::process::Command::new(bash);
                            c.args(["-c", command]);
                            c
                        } else {
                            let mut c = tokio::process::Command::new("cmd");
                            c.args(["/S", "/C", command]);
                            c
                        }
                    }
                }
            }
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", command]);
            c
        };

        if let Some(wd) = workdir {
            cmd.current_dir(self.resolve_path(wd));
        } else if let Some(ref wd) = self.default_workdir {
            // No explicit workdir: run in the active workspace directory (the
            // registry seeds `default_workdir` from it). This is what makes exec
            // run "in the workspace you're in" rather than the home dir.
            cmd.current_dir(wd);
        } else if let Some(home) = Self::home_dir() {
            // Last resort only when there is genuinely no active workspace.
            cmd.current_dir(home);
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Tell well-behaved tools to skip color output.
        // We also strip ANSI escapes from the output as a fallback.
        cmd.env("NO_COLOR", "1");
        cmd.env("TERM", "dumb");

        // Smart default timeout: longer for known slow commands.
        // Git, cargo, npm, pip, and build tools regularly exceed 30s on large repos.
        let timeout = timeout_secs.unwrap_or_else(|| default_exec_timeout_secs(command));

        // Reap our direct child if this future is dropped (e.g. the outer script
        // engine deadline fires). The per-child Job Object below widens this
        // backstop to the whole subtree on Windows.
        cmd.kill_on_drop(true);

        // Isolate the child from OUR process group.
        //
        // Without this the child shares the parent's console/process group, so a
        // script that terminates its group takes the agent down with it.
        // Observed live (2026-07-25): once models were told to run their own
        // acceptance checks, `exec sh tests/test_NN.sh` killed the harness
        // outright — exit 0xffffffff, no panic — reproduced across two models,
        // each dying seconds after its first self-check. The very same command
        // is safe through the acceptance runner, which spawns detached.
        //
        // This is not eval-specific: any agent that writes a script and runs it
        // can otherwise kill the daemon executing it. Isolation also makes the
        // timeout path honest — the tree-kill below now bounds exactly the
        // subtree we created, and can never walk up into us.
        #[cfg(windows)]
        {
            // CREATE_NEW_PROCESS_GROUP: console events (Ctrl+C / Ctrl+Break)
            // raised by or for the child stop at the child.
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
        }
        #[cfg(unix)]
        {
            // setsid(): new session + process group, so a child that signals its
            // group (or dies on a terminal signal) cannot reach us.
            cmd.process_group(0);
        }

        let child = cmd
            .spawn()
            .map_err(|e| ScriptError::Bridge(format!("Failed to execute command: {e}")))?;
        // Capture the pid *before* the wait future consumes the child, so a timeout
        // can kill the whole process tree rooted here (not just the shell).
        let pid = child.id();
        // Windows: put the child in its OWN kill-on-close Job Object before it
        // can fork. The taskkill walk below only reaches descendants of a pid
        // it can still see — `foo &` leaves a grandchild holding our pipes
        // after the shell died, invisible to the walk (2026-08-08: such a
        // survivor pinned a blocking-pool pipe read and hung teardown 25+
        // min). Job membership is inherited, so terminating the job reaps the
        // whole subtree regardless; dropping this guard (future cancelled)
        // does the same. Unix needs none of this: the process group IS the
        // subtree, dead shell or not.
        let mut job = ChildJob::assign(&child);
        let wait = child.wait_with_output();
        tokio::pin!(wait);
        let output = tokio::select! {
            res = &mut wait => {
                let output = res
                    .map_err(|e| ScriptError::Bridge(format!("Failed to execute command: {e}")))?;
                // Completed: pipes EOF'd, shell exited. Anything still running
                // is a deliberately detached background process (output
                // redirected — e.g. a server the agent started on purpose).
                // Spare it; the daemon-wide Job Object still bounds it.
                if let Some(job) = job.take() {
                    job.disarm();
                }
                output
            },
            () = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                // Overran the deadline. Kill the entire tree — the shell *and* any
                // long-running grandchild like cargo/git — so it can't outlive the
                // tool call and hold a workspace/build lock (the failure mode that
                // deadlocked repeated exec calls against each other). Walk first
                // while `wait` still owns the child (covers a live tree, including
                // anything spawned before the job assignment landed), then
                // terminate the job to sweep descendants the walk can't see.
                if let Some(pid) = pid {
                    kill_process_tree(pid).await;
                }
                if let Some(job) = job.take() {
                    job.terminate();
                }
                return Err(ScriptError::Timeout(timeout * 1000));
            }
        };

        let stdout = strip_ansi_escapes(&String::from_utf8_lossy(&output.stdout));
        let stderr = strip_ansi_escapes(&String::from_utf8_lossy(&output.stderr));

        Ok(ExecResponse {
            success: output.status.success(),
            code: output.status.code(),
            stdout,
            stderr,
        })
    }

    /// Get the current platform (windows, darwin, linux)
    #[must_use]
    pub fn platform() -> &'static str {
        if cfg!(windows) {
            "win32"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(target_os = "android") {
            "android"
        } else if cfg!(target_os = "ios") {
            "ios"
        } else {
            "linux"
        }
    }

    /// Fetch a URL (if permitted)
    pub async fn fetch(&self, url: &str, options: Option<FetchOptions>) -> Result<FetchResponse> {
        // Parse URL to check host
        let parsed = url::Url::parse(url)
            .map_err(|e| ScriptError::Bridge(format!("Invalid URL: {e}")))?;
        
        let host = parsed.host_str().unwrap_or("");
        
        if !self.permissions.allows_net(host) {
            return Err(ScriptError::Permission(format!(
                "Network access to '{host}' not permitted"
            )));
        }

        let options = options.unwrap_or_default();
        
        let mut request = match options.method.as_deref().unwrap_or("GET") {
            "GET" => self.http_client.get(url),
            "POST" => self.http_client.post(url),
            "PUT" => self.http_client.put(url),
            "DELETE" => self.http_client.delete(url),
            "PATCH" => self.http_client.patch(url),
            m => return Err(ScriptError::Bridge(format!("Unsupported method: {m}"))),
        };

        // Add headers
        if let Some(headers) = options.headers {
            for (key, value) in headers {
                request = request.header(&key, &value);
            }
        }

        // Add body
        if let Some(body) = options.body {
            request = request.body(body);
        }

        let response = request
            .send()
            .await
            .map_err(|e| ScriptError::Bridge(format!("Fetch failed: {e}")))?;

        let status = response.status().as_u16();
        let headers: std::collections::HashMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
            .collect();
        
        let body = response
            .text()
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to read body: {e}")))?;

        Ok(FetchResponse {
            status,
            headers,
            body,
        })
    }

    /// Read a file (if permitted)
    pub async fn read_file(&self, path: &str) -> Result<String> {
        let path = self.resolve_path(path);
        
        if !self.permissions.allows_read(&path) {
            return Err(ScriptError::Permission(format!(
                "Read access to '{}' not permitted",
                path.display()
            )));
        }

        tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to read '{}': {e}", path.display())))
    }

    /// Write a file (if permitted)
    pub async fn write_file(&self, path: &str, content: &str) -> Result<()> {
        // Advisory file lock: prevent concurrent writes to the same path
        let canonical = self.resolve_path(path).canonicalize().unwrap_or_else(|_| self.resolve_path(path));
        {
            let mut locks = FILE_WRITE_LOCKS.lock().unwrap_or_else(|e| e.into_inner());
            if locks.contains(&canonical) {
                return Err(ScriptError::Bridge(
                    format!("File is being written by another agent: {}", path)
                ));
            }
            locks.insert(canonical.clone());
        }
        // Ensure lock is released even on error
        struct FileGuard(std::path::PathBuf);
        impl Drop for FileGuard {
            fn drop(&mut self) {
                if let Ok(mut locks) = FILE_WRITE_LOCKS.lock() {
                    locks.remove(&self.0);
                }
            }
        }
        let _guard = FileGuard(canonical);

        let path = self.resolve_path(path);
        
        if !self.permissions.allows_write(&path) {
            return Err(ScriptError::Permission(format!(
                "Write access to '{}' not permitted",
                path.display()
            )));
        }

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                tokio::fs::create_dir_all(parent).await.ok();
            }
        }

        tokio::fs::write(&path, content)
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to write '{}': {e}", path.display())))
    }

    /// Log a message
    pub async fn log(&self, level: LogLevel, message: String) {
        let entry = LogEntry {
            level,
            message,
            timestamp: chrono::Utc::now(),
        };

        // Also emit to tracing
        match entry.level {
            LogLevel::Debug => tracing::debug!(target: "script", "{}", entry.message),
            LogLevel::Info => tracing::info!(target: "script", "{}", entry.message),
            LogLevel::Warn => tracing::warn!(target: "script", "{}", entry.message),
            LogLevel::Error => tracing::error!(target: "script", "{}", entry.message),
        }

        self.logs.write().await.push(entry);
    }

    /// List directory contents (if permitted).
    ///
    /// `max_entries` makes this a bounded query: at most that many entries are
    /// collected and the walk stops immediately once the bound is reached.
    /// This is NOT silent truncation — the caller asked for a bound and is
    /// responsible for announcing it (the convention is to request `budget + 1`
    /// so an overflow-length return proves more entries exist). The bound
    /// exists because every returned entry is marshalled into the script
    /// engine one JS object at a time; unbounded listings of real workspaces
    /// (hundreds of thousands of entries under node_modules/.git/target) blow
    /// the 30s script deadline before the script runs a single line.
    pub async fn list_dir(
        &self,
        path: &str,
        recursive: bool,
        max_entries: Option<usize>,
    ) -> Result<Vec<DirEntry>> {
        let path = self.resolve_path(path);

        if !self.permissions.allows_read(&path) {
            return Err(ScriptError::Permission(format!(
                "Read access to '{}' not permitted",
                path.display()
            )));
        }

        const IGNORE_DIRS: &[&str] = &[
            "node_modules", "target", ".git", "__pycache__", ".venv",
            "venv", "dist", "build", ".next", ".nuxt", ".cache",
        ];

        let cap = max_entries.unwrap_or(usize::MAX);
        let mut entries = Vec::new();

        if recursive {
            for result in walkdir::WalkDir::new(&path)
                .max_depth(10)
                .into_iter()
                .filter_entry(|e| {
                    e.file_name()
                        .to_str()
                        .map_or(true, |name| !IGNORE_DIRS.contains(&name))
                })
            {
                if entries.len() >= cap {
                    break;
                }
                let entry = match result {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                // Skip the root directory itself
                if entry.depth() == 0 {
                    continue;
                }
                if let Some(de) = dir_entry_from_walkdir(&entry) {
                    entries.push(de);
                }
            }
        } else {
            let mut read_dir = tokio::fs::read_dir(&path)
                .await
                .map_err(|e| ScriptError::Bridge(format!("Failed to read directory: {e}")))?;

            while let Some(entry) = read_dir
                .next_entry()
                .await
                .map_err(|e| ScriptError::Bridge(format!("Failed to read entry: {e}")))?
            {
                if entries.len() >= cap {
                    break;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                let metadata = entry.metadata().await.ok();
                let entry_type = metadata.as_ref().map_or("unknown", |m| {
                    if m.is_dir() { "dir" } else if m.is_symlink() { "link" } else { "file" }
                }).to_string();
                let size = metadata.as_ref().map_or(0, |m| m.len());
                let modified = metadata
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                entries.push(DirEntry {
                    name,
                    entry_type,
                    size,
                    modified,
                });
            }
        }

        Ok(entries)
    }

    /// Get file/directory metadata (if permitted)
    pub async fn stat(&self, path: &str) -> Result<FileStat> {
        let path = self.resolve_path(path);

        if !self.permissions.allows_read(&path) {
            return Err(ScriptError::Permission(format!(
                "Read access to '{}' not permitted",
                path.display()
            )));
        }

        let metadata = tokio::fs::metadata(&path)
            .await
            .map_err(|e| ScriptError::Bridge(format!("Failed to stat '{}': {e}", path.display())))?;

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs());

        Ok(FileStat {
            size: metadata.len(),
            is_file: metadata.is_file(),
            is_dir: metadata.is_dir(),
            modified,
        })
    }

    /// Get environment variable (if permitted)
    pub fn get_env(&self, key: &str) -> Result<Option<String>> {
        if !self.permissions.env {
            return Err(ScriptError::Permission(
                "Environment variable access not permitted".to_string(),
            ));
        }
        Ok(std::env::var(key).ok())
    }

    /// Get all logs from this execution
    pub async fn get_logs(&self) -> Vec<LogEntry> {
        self.logs.read().await.clone()
    }

    /// Clear logs
    pub async fn clear_logs(&self) {
        self.logs.write().await.clear();
    }
}

/// Exec response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecResponse {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Fetch request options
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FetchOptions {
    pub method: Option<String>,
    pub headers: Option<std::collections::HashMap<String, String>>,
    pub body: Option<String>,
}

/// Fetch response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchResponse {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: String,
}

impl FetchResponse {
    /// Parse body as JSON
    pub fn json<T: serde::de::DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_str(&self.body).map_err(Into::into)
    }
}

/// Directory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub entry_type: String,
    pub size: u64,
    pub modified: Option<u64>,
}

/// File stat result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStat {
    pub size: u64,
    pub is_file: bool,
    pub is_dir: bool,
    pub modified: Option<u64>,
}

/// Convert a walkdir entry to our DirEntry
fn dir_entry_from_walkdir(entry: &walkdir::DirEntry) -> Option<DirEntry> {
    let name = entry.path().to_string_lossy().to_string();
    let metadata = entry.metadata().ok()?;
    let entry_type = if metadata.is_dir() {
        "dir"
    } else if metadata.is_symlink() {
        "link"
    } else {
        "file"
    }
    .to_string();
    let size = metadata.len();
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    Some(DirEntry {
        name,
        entry_type,
        size,
        modified,
    })
}

/// Log level
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// Log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Strip ANSI escape sequences from text.
/// Tools like cargo, git, and npm emit color codes that break text matching
/// (e.g., `findstr "error:"` fails because the actual text is `\x1b[31merror\x1b[0m:`).
fn strip_ansi_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip CSI sequence: ESC [ ... <final byte>
            if let Some(next) = chars.next() {
                if next == '[' {
                    // Consume until we hit a letter (the terminator)
                    for seq_char in chars.by_ref() {
                        if seq_char.is_ascii_alphabetic() {
                            break;
                        }
                    }
                }
                // else: other escape (ESC without [) — just skip the ESC and next char
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------
    // Tree-kill: the timeout path must reap the shell AND its grandchildren.
    // The walk itself (`kill_process_tree`) and its unit tests live in
    // nanna-proc; what belongs here is the exec-level contract — including
    // the shape the walk alone cannot cover.
    // ---------------------------------------------------------------------

    /// Reap bound: taskkill / `TerminateJobObject` is near-instant; 5s is a
    /// generous ceiling for a loaded CI machine, polled at 50ms.
    #[allow(dead_code)] // used by the platform-specific test below
    const REAP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

    /// Liveness via `tasklist` — fine for a test; the daemon's runtime checks
    /// use Win32 directly (see nanna-daemon health.rs for why).
    #[cfg(windows)]
    async fn windows_pid_alive(pid: u32) -> bool {
        tokio::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .await
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(&pid.to_string()))
    }

    /// REGRESSION (2026-08-08, ministral endurance teardown hang): `foo &` —
    /// the shell exits immediately, the backgrounded grandchild inherits our
    /// stdout/stderr write handles, `wait_with_output` never sees EOF, and
    /// the timeout fires with the shell already DEAD. The parent-walk
    /// tree-kill (`taskkill /T` from the dead pid) finds nothing, so pre-fix
    /// the grandchild survived holding the pipe — and because tokio
    /// child-pipe reads on Windows are synchronous reads on the blocking
    /// pool, it pinned a blocking thread until process exit (25+ min hung
    /// teardown). The per-child Job Object must reap the grandchild anyway,
    /// EOFing the pipe so those reads complete and runtime shutdown is
    /// prompt.
    #[cfg(windows)]
    #[test]
    fn exec_timeout_reaps_detached_grandchild_and_frees_pipe_reads() {
        if git_bash_path().is_none() {
            eprintln!(
                "skipping exec_timeout_reaps_detached_grandchild_and_frees_pipe_reads: Git Bash not installed"
            );
            return;
        }
        // A dedicated runtime so the test can measure its teardown — the
        // production symptom was Runtime::drop joining the pinned read.
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let dir = tempfile::tempdir().expect("tempdir");
        let pid_file = dir.path().join("grandchild.pid");
        // Bash-native forward slashes: this path never goes through the
        // model-facing drive normalization (that rewrites `D:\x` shapes).
        let pid_path = pid_file.display().to_string().replace('\\', "/");
        // Git Bash `$!` is an MSYS pid; /proc/<msys-pid>/winpid maps it to
        // the real Windows pid so tasklist can probe it.
        let command =
            format!("ping -n 60 127.0.0.1 & cat /proc/$!/winpid > '{pid_path}'; exit 0");

        let mut perms = ToolPermissions::none();
        perms.run = true;
        let bridge = NannaBridge::new(perms);

        let res = rt.block_on(bridge.exec_with_timeout(&command, None, Some(2)));
        assert!(
            matches!(res, Err(ScriptError::Timeout(_))),
            "held pipes must force the timeout, got {res:?}"
        );

        // The shell wrote the pid file before exiting, well inside the 2s
        // deadline — readable by the time the call returns.
        let text = std::fs::read_to_string(&pid_file).expect("grandchild pid file");
        let grandchild_pid: u32 = text.trim().parse().expect("windows pid");

        rt.block_on(async {
            let deadline = tokio::time::Instant::now() + REAP_DEADLINE;
            while windows_pid_alive(grandchild_pid).await {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "detached grandchild survived the exec timeout"
                );
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });

        // With the grandchild dead the inherited pipe EOF'd and the blocking
        // reads finished, so teardown is prompt. Pre-fix this join waited out
        // the sleeper's full 60s.
        let teardown = std::time::Instant::now();
        drop(rt);
        assert!(
            teardown.elapsed() < REAP_DEADLINE,
            "runtime teardown pinned by a leaked pipe read: {:?}",
            teardown.elapsed()
        );
    }

    // ---------------------------------------------------------------------
    // Path resolution: absolute and relative must BOTH work, and a relative
    // path that redundantly repeats the workspace tail must not silently
    // create a shadow tree (2026-07-26: an hour of edits landed in
    // <ws>/<ws-leaf>/minidb while the tests read <ws>/minidb; run scored 0).
    // ---------------------------------------------------------------------

    #[test]
    fn bare_relative_path_resolves_into_the_workspace() {
        let ws = tempfile::tempdir().expect("tempdir");
        let resolved =
            NannaBridge::resolve_path_with_workdir("minidb", Some(ws.path()));
        assert_eq!(resolved, ws.path().join("minidb"));
    }

    #[test]
    fn absolute_path_is_used_as_given() {
        let ws = tempfile::tempdir().expect("tempdir");
        let target = ws.path().join("sub").join("minidb");
        let resolved = NannaBridge::resolve_path_with_workdir(
            &target.to_string_lossy(),
            Some(ws.path()),
        );
        assert_eq!(resolved, target, "an absolute path must not be re-rooted");
    }

    #[test]
    fn redundant_workspace_prefix_finds_the_existing_file() {
        let ws = tempfile::tempdir().expect("tempdir");
        let root = ws.path().join("minidb-bench");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("minidb"), "#!/bin/sh\n").expect("write");

        // The model addresses the file as if it were one level up.
        let resolved =
            NannaBridge::resolve_path_with_workdir("minidb-bench/minidb", Some(&root));
        assert_eq!(
            resolved,
            root.join("minidb"),
            "a redundantly prefixed path must resolve to the real file, \
             not <ws>/minidb-bench/minidb"
        );
    }

    #[test]
    fn redundant_prefix_write_lands_beside_existing_files() {
        // Nothing exists at either candidate yet (a fresh write). The target
        // whose PARENT exists is the intended one; the literal join would
        // conjure a duplicate directory.
        let ws = tempfile::tempdir().expect("tempdir");
        let root = ws.path().join("bench-ws");
        std::fs::create_dir_all(&root).expect("mkdir");

        let resolved =
            NannaBridge::resolve_path_with_workdir("bench-ws/newfile.sh", Some(&root));
        assert_eq!(resolved, root.join("newfile.sh"));
    }

    #[test]
    fn a_real_nested_directory_of_the_same_name_still_wins() {
        // The repair must never shadow a genuine file: if the literal path
        // exists, it is what the caller meant.
        let ws = tempfile::tempdir().expect("tempdir");
        let root = ws.path().join("dup");
        let nested = root.join("dup");
        std::fs::create_dir_all(&nested).expect("mkdir");
        std::fs::write(nested.join("f.txt"), "nested").expect("write");
        std::fs::write(root.join("f.txt"), "outer").expect("write");

        let resolved = NannaBridge::resolve_path_with_workdir("dup/f.txt", Some(&root));
        assert_eq!(
            resolved,
            nested.join("f.txt"),
            "an existing literal target must take precedence over the repair"
        );
    }

    #[test]
    fn multi_segment_overlap_is_repaired() {
        let ws = tempfile::tempdir().expect("tempdir");
        let root = ws.path().join("a").join("b");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(root.join("f.txt"), "x").expect("write");

        let resolved = NannaBridge::resolve_path_with_workdir("a/b/f.txt", Some(&root));
        assert_eq!(resolved, root.join("f.txt"));
    }

    #[test]
    fn unrelated_relative_path_is_left_alone() {
        let ws = tempfile::tempdir().expect("tempdir");
        let root = ws.path().join("ws");
        std::fs::create_dir_all(&root).expect("mkdir");

        let resolved =
            NannaBridge::resolve_path_with_workdir("tests/test_01.sh", Some(&root));
        assert_eq!(
            resolved,
            root.join("tests").join("test_01.sh"),
            "a normal subdirectory path must be untouched"
        );
    }

    /// REGRESSION (2026-07-25): once models were told to run their own
    /// acceptance checks, `exec sh tests/test_NN.sh` killed the HARNESS —
    /// exit 0xffffffff, no panic — reproduced across two models, each dying
    /// seconds after its first self-check. The child shared our process
    /// group, so a script that signalled its group reached us.
    ///
    /// This drives the real `exec` path with a child that deliberately
    /// terminates its own process group. Surviving to read the result IS the
    /// assertion: without isolation this test process dies with the child.
    #[tokio::test]
    async fn a_child_that_kills_its_process_group_cannot_kill_us() {
        let bridge = NannaBridge::new(ToolPermissions {
            run: true,
            ..ToolPermissions::default()
        });

        // Kill the whole group the child belongs to. Isolated, that group is
        // the child's own and we are untouched; unisolated, it is ours too.
        #[cfg(windows)]
        let suicidal = "taskkill /T /F /PID $$ 2>/dev/null; kill -TERM 0 2>/dev/null; exit 7";
        #[cfg(unix)]
        let suicidal = "kill -TERM 0; exit 7";

        let _ = bridge.exec_with_timeout(suicidal, None, Some(20)).await;

        // Reaching here at all means we survived. Prove we are still healthy
        // by running another command through the same bridge.
        let after = bridge
            .exec_with_timeout("echo still_alive", None, Some(20))
            .await
            .expect("bridge still usable after a self-killing child");
        assert!(
            after.stdout.contains("still_alive"),
            "parent survived but the bridge is broken: {after:?}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn normalize_strips_cd_slash_d_idiom() {
        for (input, want) in [
            ("cd /d D:/x", "cd D:/x"),
            ("cd /D D:/x", "cd D:/x"),
            (
                "cd /d D:/Development/nanna && cargo build",
                "cd D:/Development/nanna && cargo build",
            ),
            ("  cd /d D:/x", "  cd D:/x"),                 // leading ws preserved
            ("git fetch && cd /d D:/x", "git fetch && cd D:/x"),
            ("cd /d D:\\x", "cd D:\\x"),                   // backslash path still stripped
        ] {
            assert_eq!(normalize_cmdisms(input).as_ref(), want, "input: {input}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn normalize_leaves_legit_commands_untouched() {
        for input in [
            "cd /d",                           // bare: POSIX cd into dir /d
            "cd /data",                        // path starting /d, not the /d token
            "cd /d/x",                         // dir /d/x
            "ls -la | head -20",
            "cargo build",
            "echo cd /d not-command-initial",  // cd not at a boundary
            "echo \"cd /d x\"",                // inside quotes
        ] {
            assert_eq!(normalize_cmdisms(input).as_ref(), input, "input: {input}");
            // Borrowed (no allocation) when unchanged.
            assert!(
                matches!(normalize_cmdisms(input), std::borrow::Cow::Borrowed(_)),
                "input: {input}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_bash_style_commands_route_to_bash() {
        // The exact shapes the model wrote that used to fail in cmd.exe.
        for cmd in [
            "if [ -f \"D:/x/y.lock\" ]; then echo LOCK; cat \"D:/x/y.lock\"; else echo NO; fi",
            "cd D:/Development/nanna && git fetch origin 2>&1 | tail -5",
            "ls -la | head -20",
            "cat $file | grep foo", // used to be stolen by the `$ && |` heuristic
            "grep -r 'pattern' . && echo done",
        ] {
            assert_eq!(
                classify_windows_command(cmd.trim_start()),
                WinShell::Bash,
                "should route to bash: {cmd}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_explicit_powershell_routes_to_powershell() {
        for cmd in [
            "Get-ChildItem -Path .",
            "powershell -Command \"echo hi\"",
            "pwsh -c ls",
            "Write-Output $env:PATH",
            "Get-Content foo.txt | Select-Object -First 5",
            "Remove-Item bar -Recurse",
        ] {
            assert_eq!(
                classify_windows_command(cmd.trim_start()),
                WinShell::PowerShell,
                "should route to powershell: {cmd}"
            );
        }
    }

    #[test]
    fn test_permission_check() {
        let bridge = NannaBridge::new(
            ToolPermissions::none()
                .with_net(["api.example.com"])
                .with_read(["/tmp"]),
        );

        assert!(bridge.permissions.allows_net("api.example.com"));
        assert!(!bridge.permissions.allows_net("evil.com"));
    }

    #[test]
    fn relative_path_resolves_against_active_workspace() {
        // A bare relative path resolves against the active workspace directory
        // (the registry seeds `default_workdir` from it) — the behavior that keeps
        // tools operating in the workspace the user is in, not the home dir.
        let ws = std::path::Path::new(if cfg!(windows) { "D:\\ws" } else { "/ws" });
        let resolved = NannaBridge::resolve_path_with_workdir("a/b.txt", Some(ws));
        assert_eq!(resolved, ws.join("a/b.txt"));
    }

    #[test]
    fn tilde_path_still_expands_to_home() {
        // `~` remains explicitly home-relative regardless of the CWD fallback.
        let home = NannaBridge::home_dir().unwrap();
        let resolved = NannaBridge::resolve_path_with_workdir("~/notes.txt", None);
        assert_eq!(resolved, home.join("notes.txt"));
    }

    #[test]
    fn absolute_path_is_unchanged() {
        let abs = if cfg!(windows) { "C:\\tmp\\x.txt" } else { "/tmp/x.txt" };
        let resolved = NannaBridge::resolve_path_with_workdir(abs, None);
        assert_eq!(resolved, std::path::PathBuf::from(abs));
    }

    #[test]
    fn default_exec_timeout_classifies_build_and_vcs_tools() {
        // Build/VCS tools get the wide window; everything else the default.
        for slow in [
            "cargo test",
            "cargo build --release",
            "git status",
            "git clone https://x",
            "npm install",
            "pnpm build",
            "make -j8",
            "cd repo && cargo check",
        ] {
            assert_eq!(
                default_exec_timeout_secs(slow),
                EXEC_MAX_AUTODETECT_SECS,
                "expected wide window for {slow:?}"
            );
        }
        for fast in ["ls -la", "echo hi", "cat file.txt", "grep -r foo ."] {
            assert_eq!(
                default_exec_timeout_secs(fast),
                EXEC_DEFAULT_TIMEOUT_SECS,
                "expected default window for {fast:?}"
            );
        }
    }

    /// A command that overruns its (explicit) timeout must return `Timeout`
    /// promptly — not run to completion. Regression guard for the exec timeout
    /// actually being honored and the select-based deadline firing.
    #[cfg(unix)]
    #[tokio::test]
    async fn exec_times_out_and_returns_promptly() {
        let mut perms = ToolPermissions::none();
        perms.run = true;
        let bridge = NannaBridge::new(perms);

        let start = std::time::Instant::now();
        let res = bridge.exec_with_timeout("sleep 10", None, Some(1)).await;
        let elapsed = start.elapsed();

        assert!(matches!(res, Err(ScriptError::Timeout(_))), "got {res:?}");
        // Must fire near the 1s deadline, well before the 10s sleep would end.
        assert!(elapsed < std::time::Duration::from_secs(5), "took {elapsed:?}");
    }

    /// Same guard on Windows, routed through Git Bash (skipped if absent).
    #[cfg(windows)]
    #[tokio::test]
    async fn exec_times_out_and_returns_promptly() {
        if git_bash_path().is_none() {
            eprintln!("skipping exec_times_out_and_returns_promptly: Git Bash not installed");
            return;
        }
        let mut perms = ToolPermissions::none();
        perms.run = true;
        let bridge = NannaBridge::new(perms);

        let start = std::time::Instant::now();
        let res = bridge.exec_with_timeout("sleep 10", None, Some(1)).await;
        let elapsed = start.elapsed();

        assert!(matches!(res, Err(ScriptError::Timeout(_))), "got {res:?}");
        assert!(elapsed < std::time::Duration::from_secs(5), "took {elapsed:?}");
    }

    /// End-to-end: a POSIX/bash command with a pipe + `tail` (which used to fail in
    /// cmd.exe with an empty error) actually runs and succeeds via Git Bash.
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_exec_runs_posix_pipe_command_via_git_bash() {
        if git_bash_path().is_none() {
            eprintln!("skipping windows_exec_runs_posix_pipe_command_via_git_bash: Git Bash not installed");
            return;
        }
        let mut perms = ToolPermissions::none();
        perms.run = true;
        let bridge = NannaBridge::new(perms);

        // pipe + `tail` + POSIX `printf` — none of which cmd.exe can run.
        let out = bridge
            .exec("printf 'a\\nb\\nc\\n' | tail -1", None)
            .await
            .expect("exec should not error");
        assert!(out.success, "command should succeed, got {out:?}");
        assert_eq!(out.stdout.trim(), "c", "stdout was {:?}", out.stdout);
    }

    // ---------------------------------------------------------------------
    // list_dir bounded queries: max_entries makes listings safe to marshal
    // into the script engine (unbounded workspace walks blew the 30s Boa
    // deadline — explore tool, observed live 2026-08-02).
    // ---------------------------------------------------------------------

    /// Seed `count` files into `dir`, plus a `sub/` directory holding one file.
    fn seed_tree(dir: &std::path::Path, count: usize) {
        for i in 0..count {
            std::fs::write(dir.join(format!("f{i}.txt")), "x").expect("seed file");
        }
        std::fs::create_dir(dir.join("sub")).expect("seed subdir");
        std::fs::write(dir.join("sub").join("inner.txt"), "y").expect("seed inner");
    }

    fn read_bridge(dir: &std::path::Path) -> NannaBridge {
        NannaBridge::new(ToolPermissions::none().with_read([dir]))
    }

    #[tokio::test]
    async fn list_dir_unbounded_returns_everything() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_tree(dir.path(), 5);
        let bridge = read_bridge(dir.path());

        let flat = bridge
            .list_dir(&dir.path().to_string_lossy(), false, None)
            .await
            .expect("flat listing");
        assert_eq!(flat.len(), 6, "5 files + sub/");

        let deep = bridge
            .list_dir(&dir.path().to_string_lossy(), true, None)
            .await
            .expect("recursive listing");
        assert_eq!(deep.len(), 7, "5 files + sub/ + sub/inner.txt");
    }

    #[tokio::test]
    async fn list_dir_max_entries_bounds_flat_listing() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_tree(dir.path(), 10);
        let bridge = read_bridge(dir.path());

        let bounded = bridge
            .list_dir(&dir.path().to_string_lossy(), false, Some(3))
            .await
            .expect("bounded flat listing");
        assert_eq!(bounded.len(), 3, "must stop at the bound");

        // budget + 1 convention: an overflow-length return proves more exist.
        let probe = bridge
            .list_dir(&dir.path().to_string_lossy(), false, Some(12))
            .await
            .expect("probe listing");
        assert_eq!(probe.len(), 11, "10 files + sub/ fit under a 12-entry bound");
    }

    #[tokio::test]
    async fn list_dir_max_entries_bounds_recursive_walk() {
        let dir = tempfile::tempdir().expect("tempdir");
        seed_tree(dir.path(), 10);
        let bridge = read_bridge(dir.path());

        let bounded = bridge
            .list_dir(&dir.path().to_string_lossy(), true, Some(4))
            .await
            .expect("bounded recursive listing");
        assert_eq!(bounded.len(), 4, "recursive walk must stop at the bound");
    }
}

#[cfg(all(test, windows))]
mod drive_path_tests {
    use super::normalize_drive_paths;

    /// REGRESSION (2026-07-27): `cat > D:\ws\minidb.sh << 'EOF'` wrote a file
    /// named `D:wsminidb.sh` — bash ate the backslashes as escapes, no error,
    /// and 3 KB of the run's newest work went into a garbage name while the
    /// real artifact sat frozen.
    #[test]
    fn windows_paths_become_posix_paths() {
        for (input, want) in [
            (
                r"cat > D:\Development\ws\minidb.sh",
                "cat > /d/Development/ws/minidb.sh",
            ),
            (r"cd D:\ws && ls", "cd /d/ws && ls"),
            (r"sh C:\tools\run.sh", "sh /c/tools/run.sh"),
            // Forward-slash drive paths are valid already but normalize the same.
            ("cat D:/ws/f.txt", "cat /d/ws/f.txt"),
        ] {
            assert_eq!(normalize_drive_paths(input).as_ref(), want, "input: {input}");
        }
    }

    #[test]
    fn ordinary_commands_are_untouched() {
        for input in [
            "ls -la tests/",
            "sh tests/test_01.sh && echo PASS",
            r"grep -E '\d+' file.txt",          // a regex escape, not a path
            r"echo 'D:\ws\literal'",          // single-quoted: the author meant it
            r"printf 'a\tb\n'",
            "curl http://localhost:11434/api/tags",
        ] {
            assert_eq!(
                normalize_drive_paths(input).as_ref(),
                input,
                "must not rewrite: {input}"
            );
            assert!(
                matches!(normalize_drive_paths(input), std::borrow::Cow::Borrowed(_)),
                "no allocation for untouched input: {input}"
            );
        }
    }

    #[test]
    fn redirections_and_pipes_survive() {
        assert_eq!(
            normalize_drive_paths(r"cat D:\ws\a.txt | grep x > D:\ws\b.txt").as_ref(),
            "cat /d/ws/a.txt | grep x > /d/ws/b.txt"
        );
    }
}
