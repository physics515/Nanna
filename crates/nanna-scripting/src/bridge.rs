//! Bridge between Nanna and scripted tools
//!
//! Exposes controlled Nanna functionality to JavaScript code.

use crate::{Result, ScriptError, tool::ToolPermissions};
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

/// Bridge providing Nanna capabilities to scripts
#[derive(Clone)]
pub struct NannaBridge {
    pub permissions: ToolPermissions,
    http_client: reqwest::Client,
    logs: Arc<RwLock<Vec<LogEntry>>>,
    /// Tool definitions for `Nanna.listTools()` — set by the script engine when available
    tool_definitions: Option<Value>,
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
            // Resolve relative to workspace working directory, then home dir
            if let Some(wd) = default_workdir {
                return wd.join(p);
            }
            if let Some(home) = Self::home_dir() {
                return home.join(p);
            }
        }
        p.to_path_buf()
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
            cmd.current_dir(wd);
        } else if let Some(home) = Self::home_dir() {
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
        let timeout = timeout_secs.unwrap_or_else(|| {
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
                120 // 2 minutes for build/VCS tools
            } else {
                30 // default
            }
        });
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(timeout),
            cmd.output(),
        )
        .await
        .map_err(|_| ScriptError::Timeout(timeout * 1000))?
        .map_err(|e| ScriptError::Bridge(format!("Failed to execute command: {e}")))?;

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

    /// List directory contents (if permitted)
    pub async fn list_dir(&self, path: &str, recursive: bool) -> Result<Vec<DirEntry>> {
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
}
