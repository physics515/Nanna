//! Workspace discovery and auto-detection

use crate::{WorkspaceError, AGENTS_FILE, SOUL_FILE, WORKSPACE_MARKER_DIR};
use std::path::{Path, PathBuf};
use tracing::debug;

/// Markers that indicate a workspace root
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMarker {
    /// `.nanna/` directory exists
    NannaDir,
    /// `AGENTS.md` file exists
    AgentsFile,
    /// `SOUL.md` file exists
    SoulFile,
    /// `.git/` directory (treat git roots as potential workspaces)
    GitDir,
}

impl WorkspaceMarker {
    /// Get the path component to check for this marker
    #[must_use]
    pub const fn path(&self) -> &'static str {
        match self {
            Self::NannaDir => WORKSPACE_MARKER_DIR,
            Self::AgentsFile => AGENTS_FILE,
            Self::SoulFile => SOUL_FILE,
            Self::GitDir => ".git",
        }
    }

    /// Check if this marker exists at the given path
    #[must_use]
    pub fn exists_at(&self, dir: &Path) -> bool {
        dir.join(self.path()).exists()
    }

    /// Priority of this marker (higher = stronger signal)
    #[must_use]
    pub const fn priority(&self) -> u8 {
        match self {
            Self::NannaDir => 100,    // Explicit Nanna workspace
            Self::AgentsFile => 90,   // Has AGENTS.md
            Self::SoulFile => 80,     // Has SOUL.md
            Self::GitDir => 10,       // Git root (weak signal)
        }
    }
}

/// All markers to check, in priority order
const MARKERS: &[WorkspaceMarker] = &[
    WorkspaceMarker::NannaDir,
    WorkspaceMarker::AgentsFile,
    WorkspaceMarker::SoulFile,
    WorkspaceMarker::GitDir,
];

/// Find the workspace root by walking up from a starting directory
///
/// Checks for workspace markers (`.nanna/`, `AGENTS.md`, `SOUL.md`, `.git/`)
/// and returns the first directory that contains one.
///
/// # Arguments
/// * `start` - Directory to start searching from
///
/// # Returns
/// The workspace root path and the marker that was found
pub fn find_workspace_root(start: &Path) -> Option<(PathBuf, WorkspaceMarker)> {
    let mut current = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };

    // Walk up the directory tree
    loop {
        // Check markers in priority order
        for marker in MARKERS {
            if marker.exists_at(&current) {
                debug!(
                    "Found workspace marker {:?} at {}",
                    marker,
                    current.display()
                );
                return Some((current.clone(), *marker));
            }
        }

        // Move to parent directory
        if let Some(parent) = current.parent() {
            if parent == current {
                // Reached filesystem root
                break;
            }
            current = parent.to_path_buf();
        } else {
            break;
        }
    }

    None
}

/// Discover workspace from current directory or explicit path
///
/// # Arguments
/// * `explicit_path` - If `Some`, use this path directly; if `None`, search from cwd
///
/// # Errors
/// Returns `WorkspaceError::NotFound` if no workspace can be found
pub fn discover_workspace(explicit_path: Option<&Path>) -> Result<PathBuf, WorkspaceError> {
    if let Some(path) = explicit_path {
        // Explicit path provided - verify it looks like a workspace
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        if !abs_path.exists() {
            return Err(WorkspaceError::NotFound(abs_path));
        }

        // Check if this path itself is a workspace
        for marker in MARKERS {
            if marker.exists_at(&abs_path) {
                return Ok(abs_path);
            }
        }

        // Maybe it's a file in a workspace? Try finding root from here
        if let Some((root, _)) = find_workspace_root(&abs_path) {
            return Ok(root);
        }

        // Accept the path anyway - user explicitly asked for it
        // They might be initializing a new workspace
        Ok(abs_path)
    } else {
        // No explicit path - search from current directory
        let cwd = std::env::current_dir()?;
        find_workspace_root(&cwd)
            .map(|(path, _)| path)
            .ok_or_else(|| WorkspaceError::NotFound(cwd))
    }
}

/// Check if a directory is a valid Nanna workspace
#[must_use]
pub fn is_workspace(path: &Path) -> bool {
    MARKERS.iter().any(|m| m.exists_at(path))
}

/// Get all markers present in a directory
#[must_use]
pub fn get_markers(path: &Path) -> Vec<WorkspaceMarker> {
    MARKERS.iter().filter(|m| m.exists_at(path)).copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_find_workspace_with_agents_file() {
        let dir = tempdir().unwrap();
        let agents_path = dir.path().join(AGENTS_FILE);
        fs::write(&agents_path, "# AGENTS.md").unwrap();

        let result = find_workspace_root(dir.path());
        assert!(result.is_some());
        let (path, marker) = result.unwrap();
        assert_eq!(path, dir.path());
        assert_eq!(marker, WorkspaceMarker::AgentsFile);
    }

    #[test]
    fn test_find_workspace_nested() {
        let dir = tempdir().unwrap();
        let agents_path = dir.path().join(AGENTS_FILE);
        fs::write(&agents_path, "# AGENTS.md").unwrap();

        // Create nested directory
        let nested = dir.path().join("src").join("deep");
        fs::create_dir_all(&nested).unwrap();

        // Should find workspace root from nested dir
        let result = find_workspace_root(&nested);
        assert!(result.is_some());
        let (path, _) = result.unwrap();
        assert_eq!(path, dir.path());
    }

    #[test]
    fn test_no_workspace_found() {
        let dir = tempdir().unwrap();
        // Empty directory with no markers
        let result = find_workspace_root(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_marker_priority() {
        assert!(WorkspaceMarker::NannaDir.priority() > WorkspaceMarker::AgentsFile.priority());
        assert!(WorkspaceMarker::AgentsFile.priority() > WorkspaceMarker::GitDir.priority());
    }
}
