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

            // Skip if this skill already exists (don't overwrite user modifications)
            if target.exists() {
                continue;
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
}
