// SPDX-License-Identifier: MIT OR Apache-2.0
// Nanna — webview2.rs
//
// WebView2 runtime detection + install-prompt logic for Windows.
// Tauri 2 requires the Microsoft Edge WebView2 Runtime to be present.
// This module handles detection, prompting, and graceful fallback paths.

use std::path::{Path, PathBuf};

/// WebView2 runtime version constants (mirrors Microsoft's published versions).
pub const WEBVIEW2_MIN_VERSION: &str = "100.0.1202.64";
pub const WEBVIEW2_LATEST_STABLE: &str = "131.0.2903.102";

/// Known WebView2 installation locations on Windows.
pub mod paths {
    /// Local machine install (all users).
    pub const LOCAL_MACHINE: &str = r#"C:\Program Files\Microsoft\Edge\Application"#;
    /// User install.
    pub const USER: &str = r#"%LOCALAPPDATA%\Microsoft\Edge\Application"#;
    /// Portable / sidecar location next to the Tauri binary.
    pub const SIDE_CARBIN: &str = "webview2-runtime";
}

/// WebView2 runtime status returned by detection.
#[derive(Debug, Clone, PartialEq)]
pub enum WebView2Status {
    /// Runtime is installed and meets the minimum version.
    Installed { version: String },
    /// Runtime is installed but below the minimum required version.
    Outdated { installed: String, required: String },
    /// Runtime binary not found in any known location.
    Missing,
    /// Detection failed (e.g., non-Windows platform).
    Unsupported,
}

/// Detect whether WebView2 is installed and at what version.
///
/// On Windows this probes the registry and filesystem. On other platforms it
/// returns `Unsupported` so the GUI can degrade gracefully.
pub fn detect() -> WebView2Status {
    #[cfg(target_os = "windows")]
    {
        // 1. Check local machine install.
        if let Ok(ver) = check_registry_or_file(paths::LOCAL_MACHINE) {
            return parse_version_status(&ver, WEBVIEW2_MIN_VERSION);
        }
        // 2. Check user install.
        if let Ok(ver) = check_registry_or_file(paths::USER) {
            return parse_version_status(&ver, WEBVIEW2_MIN_VERSION);
        }
        WebView2Status::Missing
    }
    #[cfg(not(target_os = "windows"))]
    {
        WebView2Status::Unsupported
    }
}

/// Probe a registry key or fall back to checking the filesystem path.
#[cfg(target_os = "windows")]
fn check_registry_or_file(install_path: &str) -> Result<String, ()> {
    // Try registry first (fastest on Windows).
    if let Ok(key) = windows_registry_read(install_path) {
        return Ok(key);
    }
    // Fall back to filesystem probe.
    let p = Path::new(install_path);
    if p.exists() {
        // Read version from the Edge executable's embedded resource or
        // from the `version.txt` file inside the install directory.
        let version_file = p.join("version.txt");
        if version_file.exists() {
            let content = std::fs::read_to_string(&version_file)
                .map_err(|_| ())?;
            return Ok(content.trim().to_string());
        }
        // Try the executable itself.
        let exe = p.join("msedge.exe");
        if exe.exists() {
            return Ok(exe.to_string_lossy().into_owned());
        }
    }
    Err(())
}

/// Read a registry value from `HKLM\SOFTWARE\Microsoft\EdgeUpdate\Angled\` or
/// `HKCU\SOFTWARE\Microsoft\EdgeUpdate\Angled\`. Returns the version string
/// if present, otherwise `None`.
#[cfg(target_os = "windows")]
fn windows_registry_read(install_path: &str) -> Result<String, ()> {
    // We avoid pulling in a full registry crate; instead we use the
    // `reg` command via std::process. This keeps the dependency surface
    // minimal for a GUI-only feature.
    let reg_cmd = format!(
        "reg query \"HKLM\\SOFTWARE\\Microsoft\\EdgeUpdate\\Angled\" /v \"InstallPath\" 2>nul",
    );
    let output = std::process::Command::new("reg")
        .arg("query")
        .arg(r"HKLM\SOFTWARE\Microsoft\EdgeUpdate\Angled")
        .arg("/v")
        .arg("InstallPath")
        .output();

    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains(install_path) {
                return Ok(install_path.to_string());
            }
        }
        Err(_) => {}
    }

    // Try HKCU as fallback.
    let reg_cmd2 = format!(
        "reg query \"HKCU\\SOFTWARE\\Microsoft\\EdgeUpdate\\Angled\" /v \"InstallPath\" 2>nul",
    );
    let output2 = std::process::Command::new("reg")
        .arg("query")
        .arg(r"HKCU\SOFTWARE\Microsoft\Microsoft\EdgeUpdate\Angled")
        .arg("/v")
        .arg("InstallPath")
        .output();

    match output2 {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains(install_path) {
                return Ok(install_path.to_string());
            }
        }
        Err(_) => {}
    }

    Err(())
}

/// Parse a version string and compare against the minimum required.
fn parse_version_status(installed: &str, required: &str) -> WebView2Status {
    let inst = installed.trim();
    if inst.is_empty() {
        return WebView2Status::Missing;
    }
    // Try to extract a numeric version from the path or version.txt.
    let num = extract_version(installed);
    match num {
        Some(v) => {
            if v >= required {
                WebView2Status::Installed {
                    version: v,
                }
            } else {
                WebView2Status::Outdated {
                    installed: v,
                    required: required.to_string(),
                }
            }
        }
        None => WebView2Status::Missing,
    }
}

/// Extract a semver-like number from a path or version string.
fn extract_version(s: &str) -> Option<String> {
    // Match patterns like "100.0.1202.64" or "131.0.2903.102".
    let re = regex_pattern();
    // Simple manual extraction — avoids pulling in a regex crate.
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() >= 4 {
        let major = parts[0].trim().parse::<u64>().ok()?;
        let minor = parts[1].trim().parse::<u64>().ok()?;
        let patch = parts[2].trim().parse::<u64>().ok()?;
        // Build a comparable string: "major.minor.patch.0"
        Some(format!("{}.{}.{}.0", major, minor, patch))
    } else {
        None
    }
}

/// Prompt the user to install WebView2 if missing or outdated.
///
/// Returns `true` if the user agreed to install, `false` otherwise.
pub fn prompt_install() -> bool {
    #[cfg(target_os = "windows")]
    {
        // Open the Microsoft Edge download page in the default browser.
        let url = "https://go.microsoft.com/fwlink?linkid=809248";
        if let Err(e) = open_url(url) {
            log::warn!("Failed to open WebView2 installer page: {}", e);
            return false;
        }
        true
    }
    #[cfg(not(target_os = "windows"))]
    {
        log::info!("WebView2 not required on this platform");
        true
    }
}

/// Open a URL in the default browser.
#[cfg(target_os = "windows")]
fn open_url(url: &str) -> Result<(), std::io::Error> {
    use std::process::Command;
    Command::new("cmd")
        .args(["/c", "start"])
        .arg(url)
        .output()?;
    Ok(())
}

/// Check whether WebView2 is available for the current Tauri instance.
///
/// Returns a human-readable message and an action recommendation.
pub fn check_for_gui() -> (String, Action) {
    let status = detect();
    match status {
        WebView2Status::Installed { version } => (
            format!("WebView2 runtime v{} is installed.", version),
            Action::None,
        ),
        WebView2Status::Outdated { installed, required } => (
            format!(
                "WebView2 runtime v{} is outdated (required: {}).",
                installed, required
            ),
            Action::Install,
        ),
        WebView2Status::Missing => (
            "WebView2 runtime is not installed.".to_string(),
            Action::Install,
        ),
        WebView2Status::Unsupported => (
            "WebView2 detection is only available on Windows.".to_string(),
            Action::None,
        ),
    }
}

/// Recommended action based on the detection result.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Everything is fine.
    None,
    /// Prompt the user to install WebView2.
    Install,
    /// Force-install (skip prompt).
    ForceInstall,
}

/// Return the path to the bundled WebView2 runtime next to the Tauri binary.
pub fn bundled_runtime_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent()
        .map(|p| p.join(paths::SIDE_CARBIN))
        .unwrap_or_else(|| PathBuf::from(paths::SIDE_CARBIN))
}

/// Return the path to the WebView2 runtime executable if bundled.
pub fn bundled_runtime_exe() -> Option<PathBuf> {
    let runtime_dir = bundled_runtime_path();
    let exe = runtime_dir.join("MicrosoftEdgeWebView2RuntimeInstallerX64.exe");
    if exe.exists() {
        Some(exe)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_version() {
        assert_eq!(extract_version("100.0.1202.64"), Some("100.0.1202.0".to_string()));
        assert_eq!(extract_version("131.0.2903.102"), Some("131.0.2903.0".to_string()));
        assert_eq!(extract_version("not-a-version"), None);
    }

    #[test]
    fn test_parse_version_status_installed() {
        let status = parse_version_status("131.0.2903.102", WEBVIEW2_MIN_VERSION);
        assert!(matches!(status, WebView2Status::Installed { .. }));
    }

    #[test]
    fn test_parse_version_status_outdated() {
        let status = parse_version_status("99.0.1199.0", WEBVIEW2_MIN_VERSION);
        assert!(matches!(status, WebView2Status::Outdated { .. }));
    }
}
