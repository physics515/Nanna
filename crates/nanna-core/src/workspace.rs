//! Workspace management for Nanna
//!
//! A workspace is a project directory. Context comes from the project's *standard*
//! files (`README.md`, root `AGENTS.md`, `CONTRIBUTING.md`, `ROADMAP.md`, …) — not a
//! pile of bespoke `.nanna/*.md` agent sidecar files. Persona / user profile live in
//! global agent config; memory lives in the Turso store.
//!
//! `.nanna/` may still hold non-markdown local state (workspace id / config.toml).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::fs;
use tracing::{debug, info};

/// Workspace-related errors
#[derive(Error, Debug)]
pub enum WorkspaceError {
    #[error("Workspace not found: {0}")]
    NotFound(PathBuf),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid workspace: {0}")]
    Invalid(String),
}

/// Local non-md workspace state directory (workspace id / config.toml).
/// Markdown context does **not** live here.
pub const NANNA_FOLDER: &str = ".nanna";

/// Standard project context files (at workspace root)
pub const AGENTS_FILE: &str = "AGENTS.md";
pub const README_FILE: &str = "README.md";
pub const ROADMAP_FILE: &str = "ROADMAP.md";
pub const CONTRIBUTING_FILE: &str = "CONTRIBUTING.md";

/// Allowlisted context filenames that may be read/written at the workspace root.
pub const STANDARD_CONTEXT_FILES: &[&str] = &[
    README_FILE,
    AGENTS_FILE,
    ROADMAP_FILE,
    CONTRIBUTING_FILE,
];

/// Files/folders that indicate a workspace root (stronger first within a directory).
pub const WORKSPACE_MARKERS: &[&str] = &[
    ".git",
    AGENTS_FILE,
    ROADMAP_FILE,
    README_FILE,
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    NANNA_FOLDER, // weak legacy / local-state signal
    "nanna.toml", // legacy
];

/// Project context assembled from standard repo files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceContext {
    /// README.md — what the project is
    pub readme: Option<String>,
    /// Root AGENTS.md — agent instructions for this repo
    pub agents: Option<String>,
    /// CONTRIBUTING.md — conventions / how to work here
    pub contributing: Option<String>,
    /// ROADMAP.md — plan / checklist
    pub roadmap: Option<String>,
}

impl WorkspaceContext {
    /// Check if context has any content
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.readme.is_none()
            && self.agents.is_none()
            && self.contributing.is_none()
            && self.roadmap.is_none()
    }

    /// Build a system prompt injection from the standard project files
    #[must_use]
    pub fn build_system_prompt_injection(&self) -> String {
        let mut parts = Vec::new();

        if let Some(readme) = &self.readme {
            parts.push(format!("## README.md\n{readme}"));
        }
        if let Some(agents) = &self.agents {
            parts.push(format!("## AGENTS.md\n{agents}"));
        }
        if let Some(contributing) = &self.contributing {
            parts.push(format!("## CONTRIBUTING.md\n{contributing}"));
        }
        if let Some(roadmap) = &self.roadmap {
            parts.push(format!("## ROADMAP.md\n{roadmap}"));
        }

        if parts.is_empty() {
            String::new()
        } else {
            format!(
                "# Project Context\n\nThe following project files have been loaded:\n\n{}",
                parts.join("\n\n")
            )
        }
    }

    /// Get total character count of all context
    #[must_use]
    pub fn total_chars(&self) -> usize {
        self.readme.as_ref().map_or(0, String::len)
            + self.agents.as_ref().map_or(0, String::len)
            + self.contributing.as_ref().map_or(0, String::len)
            + self.roadmap.as_ref().map_or(0, String::len)
    }
}

/// Global persona + user profile injected into every session, independent of workspace.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GlobalPersona {
    /// Who the agent is (formerly SOUL.md / IDENTITY.md)
    pub persona: Option<String>,
    /// Who the user is (formerly USER.md)
    pub user_profile: Option<String>,
}

impl GlobalPersona {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.persona.as_ref().map_or(true, |s| s.trim().is_empty())
            && self.user_profile.as_ref().map_or(true, |s| s.trim().is_empty())
    }

    /// Build the system-prompt section for global persona/user.
    #[must_use]
    pub fn build_system_prompt_injection(&self) -> String {
        let mut parts = Vec::new();
        if let Some(p) = &self.persona {
            let t = p.trim();
            if !t.is_empty() {
                parts.push(format!("## Persona\n{t}"));
            }
        }
        if let Some(u) = &self.user_profile {
            let t = u.trim();
            if !t.is_empty() {
                parts.push(format!("## User Profile\n{t}"));
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            format!("# Agent Identity\n\n{}", parts.join("\n\n"))
        }
    }
}

/// A workspace represents a project directory context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    /// Unique identifier for this workspace
    pub id: String,
    /// Human-readable name (usually folder name)
    pub name: String,
    /// Absolute path to workspace root
    pub path: PathBuf,
    /// Loaded context files
    #[serde(skip)]
    pub context: WorkspaceContext,
    /// Whether workspace is currently active
    pub active: bool,
    /// Last accessed timestamp
    pub last_accessed: i64,
    /// Custom metadata
    pub metadata: HashMap<String, String>,
}

impl Workspace {
    /// Create a new workspace from a path
    pub fn new(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "workspace".to_string());
        let id = uuid::Uuid::new_v4().to_string();

        Self {
            id,
            name,
            path,
            context: WorkspaceContext::default(),
            active: false,
            last_accessed: chrono_timestamp(),
            metadata: HashMap::new(),
        }
    }

    /// Create with explicit name
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Path to the optional local-state directory (`.nanna/`).
    ///
    /// Holds non-md state only (config.toml / workspace id). Not a markdown sidecar.
    #[must_use]
    pub fn nanna_folder(&self) -> PathBuf {
        self.path.join(NANNA_FOLDER)
    }

    /// Ensure `.nanna/` exists for local non-md state.
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` if folder cannot be created.
    pub async fn ensure_nanna_folder(&self) -> Result<PathBuf, WorkspaceError> {
        let folder = self.nanna_folder();
        if !folder.exists() {
            fs::create_dir_all(&folder).await?;
            info!("Created .nanna local-state folder: {:?}", folder);
        }
        Ok(folder)
    }

    /// Load standard project context files from the workspace root
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` if files cannot be read.
    pub async fn load_context(&mut self) -> Result<(), WorkspaceError> {
        let root = &self.path;

        self.context = WorkspaceContext {
            readme: read_optional_file(&root.join(README_FILE)).await?,
            agents: read_optional_file(&root.join(AGENTS_FILE)).await?,
            contributing: read_optional_file(&root.join(CONTRIBUTING_FILE)).await?,
            roadmap: read_optional_file(&root.join(ROADMAP_FILE)).await?,
        };

        // One-shot legacy import: if root AGENTS.md is missing but `.nanna/AGENTS.md`
        // exists, surface it (do not delete — migration is best-effort).
        if self.context.agents.is_none() {
            let legacy = self.nanna_folder().join(AGENTS_FILE);
            if let Some(content) = read_optional_file(&legacy).await? {
                debug!("Loaded legacy .nanna/AGENTS.md for {}", self.name);
                self.context.agents = Some(content);
            }
        }

        self.last_accessed = chrono_timestamp();

        debug!(
            "Loaded workspace context: {} ({} chars)",
            self.name,
            self.context.total_chars()
        );

        Ok(())
    }

    /// Save a standard context file at the workspace root
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` / `Invalid` if file cannot be written or name rejected.
    pub async fn save_context_file(&self, filename: &str, content: &str) -> Result<(), WorkspaceError> {
        validate_context_filename(filename)?;
        let path = self.path.join(filename);
        // Postcondition: a validated single-component name stays inside root.
        debug_assert!(
            path.parent() == Some(self.path.as_path()),
            "validated filename escaped workspace root: {}",
            path.display()
        );
        fs::write(&path, content).await?;
        info!("Saved workspace file: {:?}", path);
        Ok(())
    }

    /// Initialize a minimal workspace: root `AGENTS.md` (+ optional `ROADMAP.md`).
    ///
    /// Does **not** scaffold SOUL/USER/TOOLS/IDENTITY/HEARTBEAT/MEMORY or a memory folder.
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` on write failure.
    pub async fn initialize_minimal(&self, with_roadmap: bool) -> Result<(), WorkspaceError> {
        fs::create_dir_all(&self.path).await?;

        let agents_path = self.path.join(AGENTS_FILE);
        if !agents_path.exists() {
            fs::write(&agents_path, DEFAULT_AGENTS_MD).await?;
            info!("Created {}", agents_path.display());
        }

        if with_roadmap {
            let roadmap_path = self.path.join(ROADMAP_FILE);
            if !roadmap_path.exists() {
                let name = &self.name;
                let body = format!(
                    "# {name} — Roadmap\n\n\
                     > Project plan. Phases, checklists, dated notes.\n\n\
                     **Last updated:** (add date)\n\n\
                     ---\n\n\
                     ## Immediate next actions\n\n\
                     1. \n"
                );
                fs::write(&roadmap_path, body).await?;
                info!("Created {}", roadmap_path.display());
            }
        }

        // Optional local-state dir for non-md config (not a markdown sidecar).
        let _ = self.ensure_nanna_folder().await?;
        Ok(())
    }
}

/// Default root AGENTS.md template (minimal).
pub const DEFAULT_AGENTS_MD: &str = r#"# AGENTS.md

This is the project workspace. Treat it that way.

## Every Session

1. Read `README.md` — what this project is
2. Read this file — agent instructions for the repo
3. Check `ROADMAP.md` if present — current plan / next actions

## Safety

- Don't exfiltrate private data. Ever.
- Don't run destructive commands without asking.
- When in doubt, ask.

## Make It Yours

Capture build commands, architecture notes, and common pitfalls here so future
sessions start with that knowledge. Keep it concise — this file is injected into
every prompt.
"#;

/// Maximum length of a workspace context filename, in bytes.
const CONTEXT_FILENAME_LEN_MAX: usize = 128;

const _: () = assert!(CONTEXT_FILENAME_LEN_MAX >= 16);

/// Validate that `filename` is a safe, single-component, allowlisted context filename.
///
/// # Errors
/// Returns [`WorkspaceError::Invalid`] if the name is empty, too long, not a single
/// safe component, or not in [`STANDARD_CONTEXT_FILES`].
pub fn validate_context_filename(filename: &str) -> Result<(), WorkspaceError> {
    if filename.is_empty() || filename.len() > CONTEXT_FILENAME_LEN_MAX {
        return Err(WorkspaceError::Invalid(format!(
            "context filename must be 1..={CONTEXT_FILENAME_LEN_MAX} bytes: {filename:?}"
        )));
    }
    if filename.contains('/') || filename.contains('\\') {
        return Err(WorkspaceError::Invalid(format!(
            "context filename must not contain path separators: {filename:?}"
        )));
    }
    let mut components = Path::new(filename).components();
    let single_normal = matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none();
    if !single_normal {
        return Err(WorkspaceError::Invalid(format!(
            "context filename must be a single path component: {filename:?}"
        )));
    }
    if !STANDARD_CONTEXT_FILES.contains(&filename) {
        return Err(WorkspaceError::Invalid(format!(
            "context filename not allowlisted (expected one of {STANDARD_CONTEXT_FILES:?}): {filename}"
        )));
    }
    Ok(())
}

/// Read a file if it exists, return None if it doesn't
async fn read_optional_file(path: &Path) -> Result<Option<String>, WorkspaceError> {
    match fs::read_to_string(path).await {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(WorkspaceError::Io(e)),
    }
}

/// Find workspace root by walking up from a path.
///
/// Looks for standard project signals (`.git`, `README.md`, `AGENTS.md`,
/// `ROADMAP.md`, `Cargo.toml` / `package.json` / `pyproject.toml`, etc.).
pub async fn find_workspace_root(start_path: impl AsRef<Path>) -> Option<PathBuf> {
    let mut current = start_path.as_ref().to_path_buf();

    if current.is_relative() {
        if let Ok(abs) = current.canonicalize() {
            current = abs;
        }
    }

    loop {
        for marker in WORKSPACE_MARKERS {
            if current.join(marker).exists() {
                debug!("Found workspace root at {:?} (marker: {})", current, marker);
                return Some(current);
            }
        }

        if !current.pop() {
            break;
        }
    }

    None
}

/// Discover workspaces under a search path (non-recursive beyond immediate children
/// that themselves look like workspace roots, plus the search path itself).
pub async fn discover_workspaces(search_path: impl AsRef<Path>) -> Vec<PathBuf> {
    let search = search_path.as_ref();
    let mut found = Vec::new();

    if is_workspace_root(search).await {
        found.push(search.to_path_buf());
    }

    if let Ok(mut entries) = fs::read_dir(search).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() && is_workspace_root(&path).await {
                found.push(path);
            }
        }
    }

    found
}

/// Check if a path looks like a workspace root
pub async fn is_workspace_root(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    if !path.is_dir() {
        return false;
    }
    WORKSPACE_MARKERS.iter().any(|m| path.join(m).exists())
}

/// In-memory registry of open workspaces
#[derive(Debug, Default)]
pub struct WorkspaceRegistry {
    workspaces: HashMap<String, Workspace>,
    active_id: Option<String>,
}

impl WorkspaceRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, workspace: Workspace) -> String {
        let id = workspace.id.clone();
        self.workspaces.insert(id.clone(), workspace);
        id
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Workspace> {
        self.workspaces.get_mut(id)
    }

    #[must_use]
    pub fn get_by_path(&self, path: &Path) -> Option<&Workspace> {
        self.workspaces.values().find(|ws| ws.path == path)
    }

    pub fn set_active(&mut self, id: &str) -> bool {
        if !self.workspaces.contains_key(id) {
            return false;
        }
        if let Some(prev) = self.active_id.take() {
            if let Some(ws) = self.workspaces.get_mut(&prev) {
                ws.active = false;
            }
        }
        if let Some(ws) = self.workspaces.get_mut(id) {
            ws.active = true;
            ws.last_accessed = chrono_timestamp();
        }
        self.active_id = Some(id.to_string());
        true
    }

    pub fn clear_active(&mut self) {
        if let Some(prev) = self.active_id.take() {
            if let Some(ws) = self.workspaces.get_mut(&prev) {
                ws.active = false;
            }
        }
    }

    #[must_use]
    pub fn active(&self) -> Option<&Workspace> {
        self.active_id
            .as_ref()
            .and_then(|id| self.workspaces.get(id))
    }

    pub fn active_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.active_id.clone()?;
        self.workspaces.get_mut(&id)
    }

    #[must_use]
    pub fn list(&self) -> Vec<&Workspace> {
        self.workspaces.values().collect()
    }

    pub fn remove(&mut self, id: &str) -> Option<Workspace> {
        if self.active_id.as_deref() == Some(id) {
            self.active_id = None;
        }
        self.workspaces.remove(id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.workspaces.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_workspace_creation() {
        let dir = tempdir().unwrap();
        let ws = Workspace::new(dir.path());
        assert_eq!(ws.path, dir.path());
        assert!(!ws.id.is_empty());
    }

    #[tokio::test]
    async fn test_workspace_context_injection() {
        let ctx = WorkspaceContext {
            readme: Some("A project".into()),
            agents: Some("Be careful".into()),
            contributing: None,
            roadmap: Some("Ship it".into()),
        };
        let injection = ctx.build_system_prompt_injection();
        assert!(injection.contains("README.md"));
        assert!(injection.contains("A project"));
        assert!(injection.contains("AGENTS.md"));
        assert!(injection.contains("ROADMAP.md"));
        assert!(!injection.contains("SOUL.md"));
        assert!(!injection.contains("MEMORY.md"));
    }

    #[tokio::test]
    async fn test_workspace_load_context() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(AGENTS_FILE), "instructions").unwrap();
        std::fs::write(dir.path().join(README_FILE), "hello").unwrap();

        let mut ws = Workspace::new(dir.path());
        ws.load_context().await.unwrap();
        assert_eq!(ws.context.agents.as_deref(), Some("instructions"));
        assert_eq!(ws.context.readme.as_deref(), Some("hello"));
        assert!(ws.context.roadmap.is_none());
    }

    #[tokio::test]
    async fn test_find_workspace_root() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let nested = dir.path().join("src").join("deep");
        std::fs::create_dir_all(&nested).unwrap();

        let found = find_workspace_root(&nested).await.unwrap();
        assert_eq!(found, dir.path());
    }

    #[tokio::test]
    async fn test_workspace_registry() {
        let dir = tempdir().unwrap();
        let mut reg = WorkspaceRegistry::new();
        let ws = Workspace::new(dir.path());
        let id = reg.register(ws);
        assert!(reg.set_active(&id));
        assert!(reg.active().is_some());
        reg.clear_active();
        assert!(reg.active().is_none());
    }

    #[test]
    fn test_validate_context_filename_accepts_standard_files() {
        for name in STANDARD_CONTEXT_FILES {
            assert!(validate_context_filename(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn test_validate_context_filename_rejects_traversal() {
        assert!(validate_context_filename("../etc/passwd").is_err());
        assert!(validate_context_filename("..\\secret").is_err());
        assert!(validate_context_filename("SOUL.md").is_err());
        assert!(validate_context_filename("MEMORY.md").is_err());
        assert!(validate_context_filename("").is_err());
    }

    #[tokio::test]
    async fn test_save_context_file_rejects_traversal() {
        let dir = tempdir().unwrap();
        let ws = Workspace::new(dir.path());
        let err = ws
            .save_context_file("../escape.md", "nope")
            .await
            .unwrap_err();
        assert!(matches!(err, WorkspaceError::Invalid(_)));
    }

    #[tokio::test]
    async fn test_initialize_minimal() {
        let dir = tempdir().unwrap();
        let ws = Workspace::new(dir.path().join("proj"));
        ws.initialize_minimal(true).await.unwrap();
        assert!(ws.path.join(AGENTS_FILE).exists());
        assert!(ws.path.join(ROADMAP_FILE).exists());
        assert!(ws.path.join(NANNA_FOLDER).exists());
        assert!(!ws.path.join("SOUL.md").exists());
        assert!(!ws.path.join("MEMORY.md").exists());
        assert!(!ws.path.join("memory").exists());
    }

    #[test]
    fn test_global_persona_injection() {
        let g = GlobalPersona {
            persona: Some("Calm and competent".into()),
            user_profile: Some("Works nights".into()),
        };
        let s = g.build_system_prompt_injection();
        assert!(s.contains("Persona"));
        assert!(s.contains("Calm and competent"));
        assert!(s.contains("User Profile"));
        assert!(s.contains("Works nights"));
    }

    #[tokio::test]
    async fn test_legacy_agents_fallback() {
        let dir = tempdir().unwrap();
        let nanna = dir.path().join(NANNA_FOLDER);
        std::fs::create_dir_all(&nanna).unwrap();
        std::fs::write(nanna.join(AGENTS_FILE), "legacy agents").unwrap();

        let mut ws = Workspace::new(dir.path());
        ws.load_context().await.unwrap();
        assert_eq!(ws.context.agents.as_deref(), Some("legacy agents"));
    }
}
