//! Workspace discovery and auto-detection

use crate::{WorkspaceError, AGENTS_FILE, README_FILE, ROADMAP_FILE, WORKSPACE_MARKER_DIR};
use std::path::{Path, PathBuf};
use tracing::debug;

/// Markers that indicate a workspace root
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceMarker {
    /// `.git/` directory
    GitDir,
    /// Root `AGENTS.md`
    AgentsFile,
    /// Root `ROADMAP.md`
    RoadmapFile,
    /// Root `README.md`
    ReadmeFile,
    /// `Cargo.toml`
    CargoToml,
    /// `package.json`
    PackageJson,
    /// `pyproject.toml`
    PyprojectToml,
    /// `go.mod`
    GoMod,
    /// Legacy / local-state `.nanna/` directory (weak)
    NannaDir,
}

impl WorkspaceMarker {
    /// Get the path component to check for this marker
    #[must_use]
    pub const fn path(&self) -> &'static str {
        match self {
            Self::GitDir => ".git",
            Self::AgentsFile => AGENTS_FILE,
            Self::RoadmapFile => ROADMAP_FILE,
            Self::ReadmeFile => README_FILE,
            Self::CargoToml => "Cargo.toml",
            Self::PackageJson => "package.json",
            Self::PyprojectToml => "pyproject.toml",
            Self::GoMod => "go.mod",
            Self::NannaDir => WORKSPACE_MARKER_DIR,
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
            Self::GitDir => 100,
            Self::AgentsFile => 90,
            Self::RoadmapFile => 80,
            Self::CargoToml | Self::PackageJson | Self::PyprojectToml | Self::GoMod => 70,
            Self::ReadmeFile => 60,
            Self::NannaDir => 20, // weak legacy / local-state
        }
    }
}

/// All markers to check, in priority order
const MARKERS: &[WorkspaceMarker] = &[
    WorkspaceMarker::GitDir,
    WorkspaceMarker::AgentsFile,
    WorkspaceMarker::RoadmapFile,
    WorkspaceMarker::CargoToml,
    WorkspaceMarker::PackageJson,
    WorkspaceMarker::PyprojectToml,
    WorkspaceMarker::GoMod,
    WorkspaceMarker::ReadmeFile,
    WorkspaceMarker::NannaDir,
];

/// Directories that terminate the upward workspace walk. We never treat the user's
/// home directory or the system temp directory (or their ancestors) as a workspace
/// root — they are shared/parent locations that may contain stray project markers
/// (e.g. a `ROADMAP.md` in `%TEMP%`) and are not project roots.
fn walk_stop_points() -> Vec<PathBuf> {
    let mut stops = Vec::new();
    if let Some(home) = dirs_home() {
        stops.push(home);
    }
    let tmp = std::env::temp_dir();
    if !tmp.as_os_str().is_empty() {
        stops.push(tmp);
    }
    stops
}

/// Resolve the user's home directory without adding a hard dependency on `dirs`.
fn dirs_home() -> Option<PathBuf> {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    None
}

/// Find the workspace root by walking up from a starting directory
pub fn find_workspace_root(start: &Path) -> Option<(PathBuf, WorkspaceMarker)> {
    let stops = walk_stop_points();
    let mut current = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };

    loop {
        // Prefer strongest marker at this level
        let mut best: Option<WorkspaceMarker> = None;
        for marker in MARKERS {
            if marker.exists_at(&current) {
                best = Some(match best {
                    Some(b) if b.priority() >= marker.priority() => b,
                    _ => *marker,
                });
            }
        }
        if let Some(marker) = best {
            // Never report a stop point (home/temp) as a workspace root.
            if !stops.iter().any(|s| s.as_path() == current) {
                debug!(
                    "Found workspace marker {:?} at {}",
                    marker,
                    current.display()
                );
                return Some((current.clone(), marker));
            }
        }

        if let Some(parent) = current.parent() {
            if parent == current {
                break;
            }
            // Do not walk above a stop point.
            if stops.iter().any(|s| s.as_path() == parent) {
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
pub fn discover_workspace(explicit_path: Option<&Path>) -> Result<PathBuf, WorkspaceError> {
    if let Some(path) = explicit_path {
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        if !abs_path.exists() {
            return Err(WorkspaceError::NotFound(abs_path));
        }

        for marker in MARKERS {
            if marker.exists_at(&abs_path) {
                return Ok(abs_path);
            }
        }

        if let Some((root, _)) = find_workspace_root(&abs_path) {
            return Ok(root);
        }

        // Accept explicit path — user may be initializing
        Ok(abs_path)
    } else {
        let cwd = std::env::current_dir()?;
        find_workspace_root(&cwd)
            .map(|(path, _)| path)
            .ok_or_else(|| WorkspaceError::NotFound(cwd))
    }
}

/// Check if a directory is a valid workspace
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
        fs::write(dir.path().join(AGENTS_FILE), "# AGENTS.md").unwrap();

        let result = find_workspace_root(dir.path());
        assert!(result.is_some());
        let (path, marker) = result.unwrap();
        assert_eq!(path, dir.path());
        assert_eq!(marker, WorkspaceMarker::AgentsFile);
    }

    #[test]
    fn test_find_workspace_nested_git() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        let nested = dir.path().join("src").join("deep");
        fs::create_dir_all(&nested).unwrap();

        let result = find_workspace_root(&nested);
        assert!(result.is_some());
        let (path, marker) = result.unwrap();
        assert_eq!(path, dir.path());
        assert_eq!(marker, WorkspaceMarker::GitDir);
    }

    #[test]
    fn test_no_workspace_found() {
        let dir = tempdir().unwrap();
        let result = find_workspace_root(dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn test_marker_priority() {
        assert!(WorkspaceMarker::GitDir.priority() > WorkspaceMarker::AgentsFile.priority());
        assert!(WorkspaceMarker::AgentsFile.priority() > WorkspaceMarker::NannaDir.priority());
    }
}
