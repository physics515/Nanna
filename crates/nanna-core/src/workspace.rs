//! Workspace management for Nanna
//!
//! A workspace is a directory context that provides:
//! - Project-specific configuration (AGENTS.md, SOUL.md, USER.md, TOOLS.md)
//! - Per-workspace memory (memory/ folder with daily notes)
//! - Isolation for multi-agent scenarios
//!
//! Workspaces are inspired by Clawdbot's directory-based project context.

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

/// Hidden folder containing all Nanna workspace files
pub const NANNA_FOLDER: &str = ".nanna";

/// Standard workspace files (relative to NANNA_FOLDER)
pub const AGENTS_FILE: &str = "AGENTS.md";
pub const SOUL_FILE: &str = "SOUL.md";
pub const USER_FILE: &str = "USER.md";
pub const TOOLS_FILE: &str = "TOOLS.md";
pub const MEMORY_FILE: &str = "MEMORY.md";
pub const IDENTITY_FILE: &str = "IDENTITY.md";
pub const HEARTBEAT_FILE: &str = "HEARTBEAT.md";

/// Memory folder for daily notes (inside NANNA_FOLDER)
pub const MEMORY_FOLDER: &str = "memory";

/// Files/folders that indicate a workspace root
pub const WORKSPACE_MARKERS: &[&str] = &[
    NANNA_FOLDER,    // .nanna folder (primary marker)
    "nanna.toml",    // Legacy: Config file at root
];

/// Workspace context files loaded from disk
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceContext {
    /// AGENTS.md content - how the agent should behave
    pub agents: Option<String>,
    /// SOUL.md content - agent personality/identity
    pub soul: Option<String>,
    /// USER.md content - info about the user
    pub user: Option<String>,
    /// TOOLS.md content - tool-specific notes
    pub tools: Option<String>,
    /// MEMORY.md content - long-term memory
    pub memory: Option<String>,
    /// IDENTITY.md content - agent identity details
    pub identity: Option<String>,
    /// HEARTBEAT.md content - periodic task checklist
    pub heartbeat: Option<String>,
}

impl WorkspaceContext {
    /// Check if context has any content
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.agents.is_none()
            && self.soul.is_none()
            && self.user.is_none()
            && self.tools.is_none()
            && self.memory.is_none()
            && self.identity.is_none()
            && self.heartbeat.is_none()
    }

    /// Build a system prompt injection from the context files
    #[must_use]
    pub fn build_system_prompt_injection(&self) -> String {
        let mut parts = Vec::new();

        if let Some(soul) = &self.soul {
            parts.push(format!("## SOUL.md\n{soul}"));
        }
        if let Some(user) = &self.user {
            parts.push(format!("## USER.md\n{user}"));
        }
        if let Some(agents) = &self.agents {
            parts.push(format!("## AGENTS.md\n{agents}"));
        }
        if let Some(tools) = &self.tools {
            parts.push(format!("## TOOLS.md\n{tools}"));
        }
        if let Some(identity) = &self.identity {
            parts.push(format!("## IDENTITY.md\n{identity}"));
        }
        // Note: MEMORY.md and HEARTBEAT.md are typically not injected directly
        // They're used for memory recall and periodic tasks

        if parts.is_empty() {
            String::new()
        } else {
            format!("# Project Context\n\n{}", parts.join("\n\n"))
        }
    }

    /// Get total character count of all context
    #[must_use]
    pub fn total_chars(&self) -> usize {
        self.agents.as_ref().map_or(0, String::len)
            + self.soul.as_ref().map_or(0, String::len)
            + self.user.as_ref().map_or(0, String::len)
            + self.tools.as_ref().map_or(0, String::len)
            + self.memory.as_ref().map_or(0, String::len)
            + self.identity.as_ref().map_or(0, String::len)
            + self.heartbeat.as_ref().map_or(0, String::len)
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

    /// Get path to the .nanna folder
    #[must_use]
    pub fn nanna_folder(&self) -> PathBuf {
        self.path.join(NANNA_FOLDER)
    }

    /// Ensure .nanna folder exists
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` if folder cannot be created.
    pub async fn ensure_nanna_folder(&self) -> Result<PathBuf, WorkspaceError> {
        let folder = self.nanna_folder();
        if !folder.exists() {
            fs::create_dir_all(&folder).await?;
            info!("Created .nanna folder: {:?}", folder);
        }
        Ok(folder)
    }

    /// Load context files from disk
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` if files cannot be read.
    pub async fn load_context(&mut self) -> Result<(), WorkspaceError> {
        let nanna = self.nanna_folder();
        
        self.context = WorkspaceContext {
            agents: read_optional_file(&nanna.join(AGENTS_FILE)).await?,
            soul: read_optional_file(&nanna.join(SOUL_FILE)).await?,
            user: read_optional_file(&nanna.join(USER_FILE)).await?,
            tools: read_optional_file(&nanna.join(TOOLS_FILE)).await?,
            memory: read_optional_file(&nanna.join(MEMORY_FILE)).await?,
            identity: read_optional_file(&nanna.join(IDENTITY_FILE)).await?,
            heartbeat: read_optional_file(&nanna.join(HEARTBEAT_FILE)).await?,
        };
        
        self.last_accessed = chrono_timestamp();
        
        debug!(
            "Loaded workspace context: {} ({} chars)",
            self.name,
            self.context.total_chars()
        );
        
        Ok(())
    }

    /// Save a context file back to disk
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` if file cannot be written.
    pub async fn save_context_file(&self, filename: &str, content: &str) -> Result<(), WorkspaceError> {
        self.ensure_nanna_folder().await?;
        let path = self.nanna_folder().join(filename);
        fs::write(&path, content).await?;
        info!("Saved workspace file: {:?}", path);
        Ok(())
    }

    /// Get path to memory folder (inside .nanna)
    #[must_use]
    pub fn memory_folder(&self) -> PathBuf {
        self.nanna_folder().join(MEMORY_FOLDER)
    }

    /// Get path to today's memory file
    #[must_use]
    pub fn today_memory_file(&self) -> PathBuf {
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        self.memory_folder().join(format!("{date}.md"))
    }

    /// Ensure memory folder exists
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` if folder cannot be created.
    pub async fn ensure_memory_folder(&self) -> Result<PathBuf, WorkspaceError> {
        let folder = self.memory_folder();
        if !folder.exists() {
            fs::create_dir_all(&folder).await?;
            info!("Created memory folder: {:?}", folder);
        }
        Ok(folder)
    }

    /// Append to today's memory file
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` if file cannot be written.
    pub async fn append_to_daily_memory(&self, content: &str) -> Result<(), WorkspaceError> {
        self.ensure_memory_folder().await?;
        let path = self.today_memory_file();
        
        let mut existing = fs::read_to_string(&path).await.unwrap_or_default();
        if !existing.is_empty() && !existing.ends_with('\n') {
            existing.push('\n');
        }
        existing.push_str(content);
        if !existing.ends_with('\n') {
            existing.push('\n');
        }
        
        fs::write(&path, existing).await?;
        debug!("Appended to daily memory: {:?}", path);
        Ok(())
    }

    /// Read recent memory files (today + yesterday)
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` if files cannot be read.
    pub async fn read_recent_memory(&self) -> Result<String, WorkspaceError> {
        let folder = self.memory_folder();
        let mut content = String::new();
        
        // Read yesterday's file
        let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let yesterday_path = folder.join(format!("{yesterday}.md"));
        if let Some(text) = read_optional_file(&yesterday_path).await? {
            content.push_str(&format!("# {yesterday}\n{text}\n\n"));
        }
        
        // Read today's file
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let today_path = folder.join(format!("{today}.md"));
        if let Some(text) = read_optional_file(&today_path).await? {
            content.push_str(&format!("# {today}\n{text}\n"));
        }
        
        Ok(content)
    }
}

/// Read a file if it exists, return None if it doesn't
async fn read_optional_file(path: &Path) -> Result<Option<String>, WorkspaceError> {
    match fs::read_to_string(path).await {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(WorkspaceError::Io(e)),
    }
}

/// Find workspace root by walking up from a path
///
/// Looks for workspace markers (AGENTS.md, SOUL.md, .nanna, nanna.toml)
pub async fn find_workspace_root(start_path: impl AsRef<Path>) -> Option<PathBuf> {
    let mut current = start_path.as_ref().to_path_buf();
    
    // Ensure we start with an absolute path
    if current.is_relative() {
        if let Ok(abs) = current.canonicalize() {
            current = abs;
        }
    }
    
    loop {
        // Check for any workspace markers
        for marker in WORKSPACE_MARKERS {
            if current.join(marker).exists() {
                debug!("Found workspace root at {:?} (marker: {})", current, marker);
                return Some(current);
            }
        }
        
        // Move up to parent
        if !current.pop() {
            break;
        }
    }
    
    None
}

/// Discover all workspaces in a directory (non-recursive for now)
pub async fn discover_workspaces(search_path: impl AsRef<Path>) -> Vec<PathBuf> {
    let search_path = search_path.as_ref();
    let mut workspaces = Vec::new();
    
    // Check if search_path itself is a workspace
    if is_workspace_root(search_path).await {
        workspaces.push(search_path.to_path_buf());
    }
    
    // Check immediate children
    if let Ok(mut entries) = fs::read_dir(search_path).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() && is_workspace_root(&path).await {
                workspaces.push(path);
            }
        }
    }
    
    workspaces
}

/// Check if a path is a workspace root
pub async fn is_workspace_root(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    for marker in WORKSPACE_MARKERS {
        if path.join(marker).exists() {
            return true;
        }
    }
    false
}

/// Global workspace registry
#[derive(Debug, Default)]
pub struct WorkspaceRegistry {
    workspaces: HashMap<String, Workspace>,
    active_workspace_id: Option<String>,
}

impl WorkspaceRegistry {
    /// Create a new empty registry
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a workspace
    pub fn register(&mut self, workspace: Workspace) -> String {
        let id = workspace.id.clone();
        self.workspaces.insert(id.clone(), workspace);
        id
    }

    /// Get a workspace by ID
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.get(id)
    }

    /// Get a mutable workspace by ID
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Workspace> {
        self.workspaces.get_mut(id)
    }

    /// Get workspace by path
    #[must_use]
    pub fn get_by_path(&self, path: &Path) -> Option<&Workspace> {
        self.workspaces.values().find(|w| w.path == path)
    }

    /// Set active workspace
    pub fn set_active(&mut self, id: &str) -> bool {
        if self.workspaces.contains_key(id) {
            // Deactivate previous
            if let Some(prev_id) = &self.active_workspace_id {
                if let Some(prev) = self.workspaces.get_mut(prev_id) {
                    prev.active = false;
                }
            }
            // Activate new
            if let Some(ws) = self.workspaces.get_mut(id) {
                ws.active = true;
                ws.last_accessed = chrono_timestamp();
            }
            self.active_workspace_id = Some(id.to_string());
            true
        } else {
            false
        }
    }

    /// Get active workspace
    #[must_use]
    pub fn active(&self) -> Option<&Workspace> {
        self.active_workspace_id
            .as_ref()
            .and_then(|id| self.workspaces.get(id))
    }

    /// Get active workspace mutably
    pub fn active_mut(&mut self) -> Option<&mut Workspace> {
        let id = self.active_workspace_id.clone()?;
        self.workspaces.get_mut(&id)
    }

    /// List all workspaces
    #[must_use]
    pub fn list(&self) -> Vec<&Workspace> {
        self.workspaces.values().collect()
    }

    /// Remove a workspace
    pub fn remove(&mut self, id: &str) -> Option<Workspace> {
        if self.active_workspace_id.as_deref() == Some(id) {
            self.active_workspace_id = None;
        }
        self.workspaces.remove(id)
    }

    /// Get count of workspaces
    #[must_use]
    pub fn len(&self) -> usize {
        self.workspaces.len()
    }

    /// Check if empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_workspace_creation() {
        let ws = Workspace::new("/tmp/test-project");
        assert_eq!(ws.name, "test-project");
        assert!(!ws.active);
    }

    #[tokio::test]
    async fn test_workspace_context_injection() {
        let mut ctx = WorkspaceContext::default();
        ctx.soul = Some("You are helpful.".to_string());
        ctx.user = Some("Name: Alice".to_string());
        
        let injection = ctx.build_system_prompt_injection();
        assert!(injection.contains("SOUL.md"));
        assert!(injection.contains("You are helpful"));
        assert!(injection.contains("USER.md"));
        assert!(injection.contains("Alice"));
    }

    #[tokio::test]
    async fn test_workspace_load_context() {
        let dir = tempdir().unwrap();
        // Create .nanna folder and files inside it
        let nanna_folder = dir.path().join(NANNA_FOLDER);
        std::fs::create_dir(&nanna_folder).unwrap();
        let soul_path = nanna_folder.join(SOUL_FILE);
        std::fs::write(&soul_path, "Test soul content").unwrap();
        
        let mut ws = Workspace::new(dir.path());
        ws.load_context().await.unwrap();
        
        assert_eq!(ws.context.soul, Some("Test soul content".to_string()));
        assert!(ws.context.agents.is_none());
    }

    #[tokio::test]
    async fn test_find_workspace_root() {
        let dir = tempdir().unwrap();
        // Create .nanna folder as workspace marker
        let nanna_folder = dir.path().join(NANNA_FOLDER);
        std::fs::create_dir(&nanna_folder).unwrap();
        
        // Create a subdirectory
        let subdir = dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        
        // Should find root from subdirectory
        let root = find_workspace_root(&subdir).await;
        assert_eq!(root, Some(dir.path().to_path_buf()));
    }

    #[tokio::test]
    async fn test_workspace_registry() {
        let mut registry = WorkspaceRegistry::new();
        
        let ws1 = Workspace::new("/tmp/project1");
        let ws2 = Workspace::new("/tmp/project2");
        
        let id1 = registry.register(ws1);
        let id2 = registry.register(ws2);
        
        assert_eq!(registry.len(), 2);
        assert!(registry.active().is_none());
        
        registry.set_active(&id1);
        assert!(registry.active().is_some());
        assert!(registry.get(&id1).unwrap().active);
        
        registry.set_active(&id2);
        assert!(!registry.get(&id1).unwrap().active);
        assert!(registry.get(&id2).unwrap().active);
    }

    #[tokio::test]
    async fn test_daily_memory() {
        let dir = tempdir().unwrap();
        let ws = Workspace::new(dir.path());
        
        ws.append_to_daily_memory("First note").await.unwrap();
        ws.append_to_daily_memory("Second note").await.unwrap();
        
        let content = std::fs::read_to_string(ws.today_memory_file()).unwrap();
        assert!(content.contains("First note"));
        assert!(content.contains("Second note"));
    }
}
