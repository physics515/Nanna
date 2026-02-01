//! Workspace manager - main interface for workspace operations

use crate::{
    discover_workspace, find_workspace_root, WorkspaceError, WorkspaceFiles, WorkspaceMarker,
    AGENTS_FILE, MEMORY_FOLDER, SOUL_FILE, WORKSPACE_MARKER_DIR,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
#[allow(unused_imports)]
use tracing::{debug, info, warn};

/// Configuration for a workspace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    /// Workspace name (defaults to directory name)
    pub name: Option<String>,
    /// Whether this workspace should include MEMORY.md in context
    /// (false for group chats, true for main sessions)
    pub include_memory: bool,
    /// Maximum context tokens from workspace files
    pub max_context_tokens: Option<usize>,
    /// Custom file loading order/selection
    pub file_priority: Option<Vec<String>>,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            name: None,
            include_memory: true,
            max_context_tokens: None,
            file_priority: None,
        }
    }
}

/// A loaded workspace with its files and configuration
#[derive(Debug, Clone)]
pub struct Workspace {
    /// Root directory of the workspace
    pub root: PathBuf,
    /// Marker that was used to identify this workspace
    pub marker: WorkspaceMarker,
    /// Loaded workspace files
    pub files: WorkspaceFiles,
    /// Workspace configuration
    pub config: WorkspaceConfig,
}

impl Workspace {
    /// Load a workspace from a directory
    pub async fn load(root: PathBuf) -> Result<Self, WorkspaceError> {
        if !root.exists() {
            return Err(WorkspaceError::NotFound(root));
        }

        // Determine marker
        let marker = find_workspace_root(&root)
            .map(|(_, m)| m)
            .unwrap_or(WorkspaceMarker::AgentsFile);

        // Load workspace files
        let files = WorkspaceFiles::load(&root).await;

        // Load workspace config from .nanna/config.toml if it exists
        let config = Self::load_config(&root).await.unwrap_or_default();

        info!("Loaded workspace from {} ({} files)", root.display(), files.existing_files().len());

        Ok(Self {
            root,
            marker,
            files,
            config,
        })
    }

    /// Load workspace config from .nanna/config.toml
    async fn load_config(root: &Path) -> Option<WorkspaceConfig> {
        let config_path = root.join(WORKSPACE_MARKER_DIR).join("config.toml");
        if config_path.exists() {
            let content = fs::read_to_string(&config_path).await.ok()?;
            toml::from_str(&content).ok()
        } else {
            None
        }
    }

    /// Save workspace config to .nanna/config.toml
    pub async fn save_config(&self) -> Result<(), WorkspaceError> {
        let config_dir = self.root.join(WORKSPACE_MARKER_DIR);
        fs::create_dir_all(&config_dir).await?;

        let config_path = config_dir.join("config.toml");
        let content = toml::to_string_pretty(&self.config)
            .map_err(|e| WorkspaceError::Parse(e.to_string()))?;
        fs::write(&config_path, content).await?;
        Ok(())
    }

    /// Get the workspace name (config name, or directory name)
    #[must_use]
    pub fn name(&self) -> String {
        self.config.name.clone().unwrap_or_else(|| {
            self.root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "workspace".to_string())
        })
    }

    /// Reload workspace files from disk
    pub async fn reload(&mut self) -> Result<(), WorkspaceError> {
        self.files = WorkspaceFiles::load(&self.root).await;
        debug!("Reloaded workspace files from {}", self.root.display());
        Ok(())
    }

    /// Generate system prompt context from workspace files
    #[must_use]
    pub fn system_context(&self) -> String {
        self.files.to_system_context(self.config.include_memory)
    }

    /// Get path to the memory folder
    #[must_use]
    pub fn memory_folder(&self) -> PathBuf {
        self.root.join(MEMORY_FOLDER)
    }

    /// Ensure the memory folder exists
    pub async fn ensure_memory_folder(&self) -> Result<PathBuf, WorkspaceError> {
        let path = self.memory_folder();
        fs::create_dir_all(&path).await?;
        Ok(path)
    }

    /// Check if this is a fresh workspace (has BOOTSTRAP.md)
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        self.files.is_fresh()
    }

    /// Append content to today's daily memory file
    pub async fn append_to_daily_memory(&self, content: &str) -> Result<(), WorkspaceError> {
        let mut file = WorkspaceFiles::load_or_create_today(&self.root).await?;
        file.content.push_str(content);
        file.content.push('\n');
        file.save().await
    }

    /// Initialize workspace with basic files if they don't exist
    pub async fn initialize(&self) -> Result<(), WorkspaceError> {
        // Create .nanna directory
        let marker_dir = self.root.join(WORKSPACE_MARKER_DIR);
        fs::create_dir_all(&marker_dir).await?;

        // Create memory folder
        fs::create_dir_all(self.memory_folder()).await?;

        // Create minimal AGENTS.md if it doesn't exist
        let agents_path = self.root.join(AGENTS_FILE);
        if !agents_path.exists() {
            let default_agents = r#"# AGENTS.md - Your Workspace

This folder is your workspace. Treat it as home.

## Memory

- **Daily notes:** `memory/YYYY-MM-DD.md` — raw logs of what happened
- **Long-term:** `MEMORY.md` — your curated memories

## Safety

- Don't exfiltrate private data
- Ask before destructive operations
- When in doubt, ask
"#;
            fs::write(&agents_path, default_agents).await?;
        }

        // Create minimal SOUL.md if it doesn't exist
        let soul_path = self.root.join(SOUL_FILE);
        if !soul_path.exists() {
            let default_soul = r#"# SOUL.md - Who You Are

Be genuinely helpful, not performatively helpful.
Have opinions. Be resourceful before asking.
Earn trust through competence.
"#;
            fs::write(&soul_path, default_soul).await?;
        }

        info!("Initialized workspace at {}", self.root.display());
        Ok(())
    }
}

/// Manager for multiple workspaces with switching support
pub struct WorkspaceManager {
    /// Currently active workspace
    active: RwLock<Option<Workspace>>,
    /// Cache of loaded workspaces by path
    cache: RwLock<HashMap<PathBuf, Workspace>>,
    /// Default workspace path (if configured)
    default_path: Option<PathBuf>,
}

impl WorkspaceManager {
    /// Create a new workspace manager
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: RwLock::new(None),
            cache: RwLock::new(HashMap::new()),
            default_path: None,
        }
    }

    /// Create with a default workspace path
    #[must_use]
    pub fn with_default(default_path: PathBuf) -> Self {
        Self {
            active: RwLock::new(None),
            cache: RwLock::new(HashMap::new()),
            default_path: Some(default_path),
        }
    }

    /// Get the currently active workspace
    pub async fn active(&self) -> Option<Workspace> {
        self.active.read().await.clone()
    }

    /// Load and activate a workspace
    pub async fn activate(&self, path: &Path) -> Result<Workspace, WorkspaceError> {
        let canonical = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(ws) = cache.get(&canonical) {
                let mut active = self.active.write().await;
                *active = Some(ws.clone());
                info!("Activated cached workspace: {}", ws.name());
                return Ok(ws.clone());
            }
        }

        // Load workspace
        let workspace = Workspace::load(canonical.clone()).await?;

        // Cache it
        {
            let mut cache = self.cache.write().await;
            cache.insert(canonical, workspace.clone());
        }

        // Set as active
        {
            let mut active = self.active.write().await;
            *active = Some(workspace.clone());
        }

        info!("Activated workspace: {}", workspace.name());
        Ok(workspace)
    }

    /// Auto-discover and activate a workspace
    pub async fn auto_activate(&self, explicit_path: Option<&Path>) -> Result<Workspace, WorkspaceError> {
        let path = discover_workspace(explicit_path)?;
        self.activate(&path).await
    }

    /// Activate the default workspace (if configured)
    pub async fn activate_default(&self) -> Result<Workspace, WorkspaceError> {
        if let Some(ref path) = self.default_path {
            self.activate(path).await
        } else {
            self.auto_activate(None).await
        }
    }

    /// Switch to a different workspace
    pub async fn switch(&self, path: &Path) -> Result<Workspace, WorkspaceError> {
        self.activate(path).await
    }

    /// Reload the currently active workspace
    pub async fn reload_active(&self) -> Result<(), WorkspaceError> {
        let mut active = self.active.write().await;
        if let Some(ref mut ws) = *active {
            ws.reload().await?;
        }
        Ok(())
    }

    /// Get all cached workspaces
    pub async fn list_cached(&self) -> Vec<Workspace> {
        self.cache.read().await.values().cloned().collect()
    }

    /// Clear the workspace cache
    pub async fn clear_cache(&self) {
        self.cache.write().await.clear();
    }

    /// Create and initialize a new workspace
    pub async fn create(&self, path: &Path) -> Result<Workspace, WorkspaceError> {
        // Create directory if it doesn't exist
        fs::create_dir_all(path).await?;

        // Load (creates default config)
        let workspace = Workspace::load(path.to_path_buf()).await?;

        // Initialize with default files
        workspace.initialize().await?;

        // Reload to pick up new files
        let mut workspace = workspace;
        workspace.reload().await?;

        // Cache it
        {
            let mut cache = self.cache.write().await;
            cache.insert(path.to_path_buf(), workspace.clone());
        }

        Ok(workspace)
    }
}

impl Default for WorkspaceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_workspace_load() {
        let dir = tempdir().unwrap();
        write(dir.path().join(AGENTS_FILE), "# Test agents").unwrap();

        let workspace = Workspace::load(dir.path().to_path_buf()).await.unwrap();
        assert!(workspace.files.agents.is_some());
        assert_eq!(workspace.marker, WorkspaceMarker::AgentsFile);
    }

    #[tokio::test]
    async fn test_workspace_manager_activate() {
        let dir = tempdir().unwrap();
        write(dir.path().join(AGENTS_FILE), "# Test").unwrap();

        let manager = WorkspaceManager::new();
        let workspace = manager.activate(dir.path()).await.unwrap();

        assert!(manager.active().await.is_some());
        assert_eq!(workspace.root, dir.path());
    }

    #[tokio::test]
    async fn test_workspace_initialize() {
        let dir = tempdir().unwrap();

        let workspace = Workspace::load(dir.path().to_path_buf()).await.unwrap();
        workspace.initialize().await.unwrap();

        assert!(dir.path().join(AGENTS_FILE).exists());
        assert!(dir.path().join(SOUL_FILE).exists());
        assert!(dir.path().join(WORKSPACE_MARKER_DIR).exists());
        assert!(dir.path().join(MEMORY_FOLDER).exists());
    }
}
