//! Tool directory resolution, loading, and bootstrapping helpers.
//!
//! Tools are loaded dynamically from the filesystem at runtime.
//! Default skills are embedded at compile time (via build.rs) and extracted
//! to the tools directory on first run in release builds.
//!
//! # Resolution Order
//!
//! 1. `NANNA_TOOLS_DIR` environment variable (for development)
//! 2. `config_tools_dir` (explicit configuration)
//! 3. Caller-provided fallback (typically `{data_dir}/tools/`)

use std::path::{Path, PathBuf};

// Include the build-script-generated embedded skills (all default-skills/ files).
include!(concat!(env!("OUT_DIR"), "/embedded_skills.rs"));

/// Parse a semver version string into (major, minor, patch) tuple.
/// Returns None if the string is not a valid semver triple.
#[cfg_attr(debug_assertions, allow(dead_code))]
fn parse_semver(v: &str) -> Option<(u64, u64, u64)> {
    // Strip leading 'v' if present
    let v = v.strip_prefix('v').unwrap_or(v);
    // Strip pre-release/build metadata (everything after - or +)
    let v = v.split(['-', '+']).next().unwrap_or(v);
    let parts: Vec<&str> = v.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

/// Returns true if `embedded` version is strictly greater than `installed`.
#[cfg_attr(debug_assertions, allow(dead_code))]
fn is_newer_version(embedded: &str, installed: &str) -> bool {
    match (parse_semver(embedded), parse_semver(installed)) {
        (Some(e), Some(i)) => e > i,
        // If either fails to parse, don't overwrite
        _ => false,
    }
}

/// Extract the version field from a tool.ts source string.
/// Looks for `version: "x.y.z"` or `version: 'x.y.z'` in the source.
#[cfg_attr(debug_assertions, allow(dead_code))]
fn extract_version_from_source(source: &str) -> Option<String> {
    // Reuse the same pattern as extract_string_field in nanna-scripting
    let patterns = [
        r#"version: ""#,
        r#"version: '"#,
        r#"version:""#,
        r#"version:'"#,
    ];
    for pattern in &patterns {
        if let Some(start) = source.find(pattern) {
            let quote = if pattern.ends_with('"') { '"' } else { '\'' };
            let value_start = start + pattern.len();
            if let Some(end) = source[value_start..].find(quote) {
                return Some(source[value_start..value_start + end].to_string());
            }
        }
    }
    None
}

/// In debug builds, fall back to the source tree's default-skills directory.
/// This is resolved at compile time relative to the nanna-tools crate.
#[cfg(debug_assertions)]
pub const DEV_TOOLS_DIR: Option<&str> = Some(concat!(env!("CARGO_MANIFEST_DIR"), "\\default-skills"));

#[cfg(not(debug_assertions))]
pub const DEV_TOOLS_DIR: Option<&str> = None;

/// Default permissions for built-in TS skills.
/// These need broader permissions than typical user tools.
pub const DEFAULT_PERMISSIONS_JSON: &str = r#"{
    "read": ["*"],
    "write": ["*"],
    "run": true,
    "net": ["*"],
    "env": true
}"#;

/// Resolve the tools directory from environment, config, or fallback.
///
/// Resolution order:
/// 1. `NANNA_TOOLS_DIR` environment variable
/// 2. `config_tools_dir` parameter (from config file)
/// 3. `fallback` parameter (typically `{data_dir}/tools/`)
///
/// Returns `None` only if no valid path can be determined.
pub fn resolve_tools_dir(config_tools_dir: Option<&Path>) -> Option<PathBuf> {
    // 1. Environment variable (highest priority — useful for development)
    if let Ok(env_dir) = std::env::var("NANNA_TOOLS_DIR") {
        let p = PathBuf::from(env_dir);
        if p.is_dir() {
            tracing::info!("Using tools directory from NANNA_TOOLS_DIR: {:?}", p);
            return Some(p);
        }
        tracing::warn!("NANNA_TOOLS_DIR set but directory does not exist: {:?}", p);
    }

    // 2. Explicit config value
    if let Some(dir) = config_tools_dir {
        if dir.is_dir() {
            tracing::info!("Using tools directory from config: {:?}", dir);
            return Some(dir.to_path_buf());
        }
        // Return it even if it doesn't exist yet (caller may create it)
        tracing::info!("Using configured tools_dir (may not exist yet): {:?}", dir);
        return Some(dir.to_path_buf());
    }

    // 3. Development fallback: source tree's default-skills directory
    if let Some(dev_dir) = DEV_TOOLS_DIR {
        let p = PathBuf::from(dev_dir);
        if p.is_dir() {
            tracing::info!("Using development tools directory: {:?}", p);
            return Some(p);
        }
    }

    None
}

/// Bootstrap the tools directory by copying bundled default skills into it.
///
/// In debug builds this is a no-op (we load directly from the source tree).
/// In release builds, default skills are embedded at compile time and extracted
/// to the target directory on first run.
///
/// Returns the number of skills bootstrapped (0 if already present).
pub fn bootstrap_default_skills(tools_dir: &Path) -> usize {
    // Create the tools directory if it doesn't exist
    if !tools_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(tools_dir) {
            tracing::error!("Failed to create tools directory {:?}: {}", tools_dir, e);
            return 0;
        }
        tracing::info!("Created tools directory: {:?}", tools_dir);
    }

    // In debug builds, tools are loaded directly from the source tree via DEV_TOOLS_DIR.
    // Only bootstrap in release builds where we need to populate {data_dir}/tools/.
    #[cfg(debug_assertions)]
    {
        let _ = tools_dir; // suppress unused warning
        return 0;
    }

    #[cfg(not(debug_assertions))]
    {
        let mut count = 0;
        for entry in DEFAULT_SKILLS {
            let tool_dir = tools_dir.join(entry.skill_name);
            let target = tool_dir.join(entry.file_name);

            if target.exists() {
                // Only overwrite tool.ts files (not permissions.json etc.) when
                // the embedded version is strictly newer than the installed one.
                if entry.file_name == "tool.ts" || entry.file_name == "tool.js" {
                    let embedded_ver = extract_version_from_source(entry.content);
                    let installed_source = std::fs::read_to_string(&target).unwrap_or_default();
                    let installed_ver = extract_version_from_source(&installed_source);

                    match (&embedded_ver, &installed_ver) {
                        (Some(e), Some(i)) if is_newer_version(e, i) => {
                            tracing::info!(
                                "Upgrading default skill {}/{}: {} → {}",
                                entry.skill_name, entry.file_name, i, e
                            );
                            // Fall through to write
                        }
                        (Some(e), None) => {
                            // Installed tool has no version — embedded does. Upgrade it.
                            tracing::info!(
                                "Upgrading unversioned skill {}/{} to {}",
                                entry.skill_name, entry.file_name, e
                            );
                            // Fall through to write
                        }
                        _ => {
                            // Same version, older embedded, or both unversioned — skip
                            continue;
                        }
                    }
                } else {
                    // Non-tool files (permissions.json etc.) — don't overwrite
                    continue;
                }
            }

            if let Err(e) = std::fs::create_dir_all(&tool_dir) {
                tracing::warn!("Failed to create skill directory {:?}: {}", tool_dir, e);
                continue;
            }

            if let Err(e) = std::fs::write(&target, entry.content) {
                tracing::warn!("Failed to write {:?}: {}", target, e);
                continue;
            }

            tracing::info!("Bootstrapped default skill: {}/{}", entry.skill_name, entry.file_name);
            count += 1;
        }

        if count > 0 {
            // Ensure permissions are set for newly created skills
            ensure_permissions(tools_dir);
            tracing::info!("Bootstrapped {} default skills into {:?}", count, tools_dir);
        }

        count
    }
}

/// Ensure a tools directory has correct permissions.json files for all subdirectories.
///
/// Writes `permissions.json` into any tool subdirectory that lacks one.
pub fn ensure_permissions(tools_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(tools_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let perms = path.join("permissions.json");
            if !perms.exists() {
                if let Err(e) = std::fs::write(&perms, DEFAULT_PERMISSIONS_JSON) {
                    tracing::debug!("Could not write permissions.json to {:?}: {}", path, e);
                }
            }
        }
    }
}

/// Load the `discover_tools` skill source from a tools directory.
///
/// Returns `None` if the file doesn't exist.
pub fn load_discover_tools_source(tools_dir: &Path) -> Option<String> {
    let path = tools_dir.join("discover_tools").join("tool.ts");
    match std::fs::read_to_string(&path) {
        Ok(source) => Some(source),
        Err(e) => {
            tracing::warn!("Could not load discover_tools from {:?}: {}", path, e);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_resolve_tools_dir_from_env() {
        let dir = tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // Temporarily set env var
        unsafe { std::env::set_var("NANNA_TOOLS_DIR", &path); }
        let resolved = resolve_tools_dir(None);
        unsafe { std::env::remove_var("NANNA_TOOLS_DIR"); }

        assert_eq!(resolved, Some(path));
    }

    #[test]
    fn test_resolve_tools_dir_from_config() {
        let dir = tempdir().unwrap();
        let path = dir.path().to_path_buf();

        // Make sure env var doesn't interfere
        unsafe { std::env::remove_var("NANNA_TOOLS_DIR"); }

        let resolved = resolve_tools_dir(Some(&path));
        assert_eq!(resolved, Some(path));
    }

    #[test]
    fn test_ensure_permissions() {
        let dir = tempdir().unwrap();
        let tool_dir = dir.path().join("my_tool");
        std::fs::create_dir_all(&tool_dir).unwrap();
        std::fs::write(tool_dir.join("tool.ts"), "// test").unwrap();

        // No permissions.json yet
        assert!(!tool_dir.join("permissions.json").exists());

        ensure_permissions(dir.path());

        // Now it should exist
        assert!(tool_dir.join("permissions.json").exists());
    }

    #[test]
    fn test_load_discover_tools_source() {
        let dir = tempdir().unwrap();
        let dt_dir = dir.path().join("discover_tools");
        std::fs::create_dir_all(&dt_dir).unwrap();
        std::fs::write(dt_dir.join("tool.ts"), "export default { name: 'discover_tools' }").unwrap();

        let source = load_discover_tools_source(dir.path());
        assert!(source.is_some());
        assert!(source.unwrap().contains("discover_tools"));
    }

    #[test]
    fn test_load_discover_tools_source_missing() {
        let dir = tempdir().unwrap();
        let source = load_discover_tools_source(dir.path());
        assert!(source.is_none());
    }

    #[test]
    fn test_parse_semver() {
        assert_eq!(parse_semver("0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_semver("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_semver("v1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_semver("1.0.0-beta.1"), Some((1, 0, 0)));
        assert_eq!(parse_semver("1.0.0+build.123"), Some((1, 0, 0)));
        assert_eq!(parse_semver("not-a-version"), None);
        assert_eq!(parse_semver("1.0"), None);
        assert_eq!(parse_semver(""), None);
    }

    #[test]
    fn test_is_newer_version() {
        assert!(is_newer_version("0.2.0", "0.1.0"));
        assert!(is_newer_version("1.0.0", "0.9.9"));
        assert!(is_newer_version("0.1.1", "0.1.0"));
        assert!(!is_newer_version("0.1.0", "0.1.0")); // same = not newer
        assert!(!is_newer_version("0.1.0", "0.2.0")); // older
        assert!(!is_newer_version("bad", "0.1.0"));    // unparseable
    }

    #[test]
    fn test_extract_version_from_source() {
        let source = r#"export default {
  name: "exec",
  version: "0.1.0",
  description: "Run stuff",
}"#;
        assert_eq!(extract_version_from_source(source), Some("0.1.0".to_string()));

        let source_single = r#"export default {
  name: 'exec',
  version: '1.2.3',
}"#;
        assert_eq!(extract_version_from_source(source_single), Some("1.2.3".to_string()));

        let no_version = r#"export default { name: "exec" }"#;
        assert_eq!(extract_version_from_source(no_version), None);
    }
}
