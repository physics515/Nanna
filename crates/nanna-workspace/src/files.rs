//! Workspace file loading and management

use crate::{
    WorkspaceError, AGENTS_FILE, BOOTSTRAP_FILE, HEARTBEAT_FILE, IDENTITY_FILE, MEMORY_FILE,
    MEMORY_FOLDER, SOUL_FILE, TOOLS_FILE, USER_FILE,
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;
#[allow(unused_imports)]
use tracing::{debug, warn};

/// A single workspace file with its content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    /// File name (e.g., "AGENTS.md")
    pub name: String,
    /// Full path to the file
    pub path: PathBuf,
    /// File content
    pub content: String,
    /// Whether the file exists
    pub exists: bool,
    /// Last modified timestamp (Unix seconds)
    pub modified: Option<i64>,
}

impl WorkspaceFile {
    /// Load a workspace file from disk
    pub async fn load(root: &Path, name: &str) -> Self {
        let path = root.join(name);
        match fs::read_to_string(&path).await {
            Ok(content) => {
                let modified = fs::metadata(&path)
                    .await
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64);

                Self {
                    name: name.to_string(),
                    path,
                    content,
                    exists: true,
                    modified,
                }
            }
            Err(_) => Self {
                name: name.to_string(),
                path,
                content: String::new(),
                exists: false,
                modified: None,
            },
        }
    }

    /// Save the file content to disk
    ///
    /// # Errors
    /// Returns `WorkspaceError::Io` if writing fails
    pub async fn save(&self) -> Result<(), WorkspaceError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&self.path, &self.content).await?;
        Ok(())
    }

    /// Check if the file has content
    #[must_use]
    pub fn has_content(&self) -> bool {
        self.exists && !self.content.trim().is_empty()
    }
}

/// Collection of all workspace context files
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceFiles {
    /// AGENTS.md - Agent instructions and behavior
    pub agents: Option<WorkspaceFile>,
    /// SOUL.md - Agent personality and identity
    pub soul: Option<WorkspaceFile>,
    /// USER.md - Information about the user
    pub user: Option<WorkspaceFile>,
    /// TOOLS.md - Local tool notes and configuration
    pub tools: Option<WorkspaceFile>,
    /// MEMORY.md - Long-term memory (main session only)
    pub memory: Option<WorkspaceFile>,
    /// IDENTITY.md - Agent identity metadata
    pub identity: Option<WorkspaceFile>,
    /// HEARTBEAT.md - Heartbeat tasks
    pub heartbeat: Option<WorkspaceFile>,
    /// BOOTSTRAP.md - First-run instructions (if exists)
    pub bootstrap: Option<WorkspaceFile>,
    /// Daily memory files from memory/ folder
    pub daily_memories: Vec<WorkspaceFile>,
}

impl WorkspaceFiles {
    /// Load all workspace files from a root directory
    pub async fn load(root: &Path) -> Self {
        let agents = Self::load_if_exists(root, AGENTS_FILE).await;
        let soul = Self::load_if_exists(root, SOUL_FILE).await;
        let user = Self::load_if_exists(root, USER_FILE).await;
        let tools = Self::load_if_exists(root, TOOLS_FILE).await;
        let memory = Self::load_if_exists(root, MEMORY_FILE).await;
        let identity = Self::load_if_exists(root, IDENTITY_FILE).await;
        let heartbeat = Self::load_if_exists(root, HEARTBEAT_FILE).await;
        let bootstrap = Self::load_if_exists(root, BOOTSTRAP_FILE).await;

        // Load recent daily memories (today + yesterday)
        let daily_memories = Self::load_daily_memories(root).await;

        Self {
            agents,
            soul,
            user,
            tools,
            memory,
            identity,
            heartbeat,
            bootstrap,
            daily_memories,
        }
    }

    async fn load_if_exists(root: &Path, name: &str) -> Option<WorkspaceFile> {
        let file = WorkspaceFile::load(root, name).await;
        if file.exists {
            Some(file)
        } else {
            None
        }
    }

    /// Load daily memory files (today and yesterday)
    async fn load_daily_memories(root: &Path) -> Vec<WorkspaceFile> {
        let memory_dir = root.join(MEMORY_FOLDER);
        if !memory_dir.exists() {
            return Vec::new();
        }

        let today = Local::now().date_naive();
        let yesterday = today.pred_opt().unwrap_or(today);

        let mut memories = Vec::new();

        for date in [today, yesterday] {
            let filename = format!("{}.md", date.format("%Y-%m-%d"));
            let file = WorkspaceFile::load(&memory_dir, &filename).await;
            if file.exists {
                memories.push(file);
            }
        }

        memories
    }

    /// Get today's daily memory file path
    #[must_use]
    pub fn today_memory_path(root: &Path) -> PathBuf {
        let today = Local::now().date_naive();
        root.join(MEMORY_FOLDER)
            .join(format!("{}.md", today.format("%Y-%m-%d")))
    }

    /// Load or create today's daily memory file
    pub async fn load_or_create_today(root: &Path) -> Result<WorkspaceFile, WorkspaceError> {
        let path = Self::today_memory_path(root);
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        // Ensure memory directory exists
        let memory_dir = root.join(MEMORY_FOLDER);
        fs::create_dir_all(&memory_dir).await?;

        let file = WorkspaceFile::load(&memory_dir, &name).await;
        if file.exists {
            Ok(file)
        } else {
            // Create with header
            let today = Local::now().date_naive();
            let content = format!(
                "# Daily Notes — {}\n\n",
                today.format("%B %d, %Y")
            );
            let new_file = WorkspaceFile {
                name,
                path,
                content,
                exists: true,
                modified: None,
            };
            new_file.save().await?;
            Ok(new_file)
        }
    }

    /// Check if this is a fresh workspace (has BOOTSTRAP.md)
    #[must_use]
    pub fn is_fresh(&self) -> bool {
        self.bootstrap.as_ref().map_or(false, |f| f.exists)
    }

    /// Generate system prompt context from loaded files
    ///
    /// # Arguments
    /// * `include_memory` - Whether to include MEMORY.md (only for main sessions)
    #[must_use]
    pub fn to_system_context(&self, include_memory: bool) -> String {
        let mut sections = Vec::new();

        // Bootstrap takes priority if present
        if let Some(ref bootstrap) = self.bootstrap {
            if bootstrap.has_content() {
                sections.push(format!("## BOOTSTRAP.md\n{}", bootstrap.content));
            }
        }

        // AGENTS.md - core instructions
        if let Some(ref agents) = self.agents {
            if agents.has_content() {
                sections.push(format!("## AGENTS.md\n{}", agents.content));
            }
        }

        // SOUL.md - personality
        if let Some(ref soul) = self.soul {
            if soul.has_content() {
                sections.push(format!("## SOUL.md\n{}", soul.content));
            }
        }

        // USER.md - user info
        if let Some(ref user) = self.user {
            if user.has_content() {
                sections.push(format!("## USER.md\n{}", user.content));
            }
        }

        // TOOLS.md - tool notes
        if let Some(ref tools) = self.tools {
            if tools.has_content() {
                sections.push(format!("## TOOLS.md\n{}", tools.content));
            }
        }

        // IDENTITY.md - agent identity
        if let Some(ref identity) = self.identity {
            if identity.has_content() {
                sections.push(format!("## IDENTITY.md\n{}", identity.content));
            }
        }

        // MEMORY.md - long-term memory (only for main sessions)
        if include_memory {
            if let Some(ref memory) = self.memory {
                if memory.has_content() {
                    sections.push(format!("## MEMORY.md\n{}", memory.content));
                }
            }
        }

        // Daily memories (always included if present)
        for daily in &self.daily_memories {
            if daily.has_content() {
                sections.push(format!("## {}\n{}", daily.name, daily.content));
            }
        }

        // HEARTBEAT.md - heartbeat tasks
        if let Some(ref heartbeat) = self.heartbeat {
            if heartbeat.has_content() {
                sections.push(format!("## HEARTBEAT.md\n{}", heartbeat.content));
            }
        }

        if sections.is_empty() {
            String::new()
        } else {
            format!("# Project Context\n\nThe following project context files have been loaded:\n\n{}", 
                    sections.join("\n\n"))
        }
    }

    /// Get list of existing files
    #[must_use]
    pub fn existing_files(&self) -> Vec<&WorkspaceFile> {
        let mut files = Vec::new();
        if let Some(ref f) = self.agents { if f.exists { files.push(f); } }
        if let Some(ref f) = self.soul { if f.exists { files.push(f); } }
        if let Some(ref f) = self.user { if f.exists { files.push(f); } }
        if let Some(ref f) = self.tools { if f.exists { files.push(f); } }
        if let Some(ref f) = self.memory { if f.exists { files.push(f); } }
        if let Some(ref f) = self.identity { if f.exists { files.push(f); } }
        if let Some(ref f) = self.heartbeat { if f.exists { files.push(f); } }
        if let Some(ref f) = self.bootstrap { if f.exists { files.push(f); } }
        for f in &self.daily_memories {
            if f.exists { files.push(f); }
        }
        files
    }

    /// Total size of all loaded content (in bytes)
    #[must_use]
    pub fn total_size(&self) -> usize {
        self.existing_files().iter().map(|f| f.content.len()).sum()
    }

    /// Estimated token count (rough: ~4 chars per token)
    #[must_use]
    pub fn estimated_tokens(&self) -> usize {
        self.total_size() / 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::write;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_load_workspace_files() {
        let dir = tempdir().unwrap();
        write(dir.path().join(AGENTS_FILE), "# Agent instructions").unwrap();
        write(dir.path().join(SOUL_FILE), "# Soul content").unwrap();

        let files = WorkspaceFiles::load(dir.path()).await;

        assert!(files.agents.is_some());
        assert!(files.soul.is_some());
        assert!(files.user.is_none());
        assert!(files.agents.unwrap().content.contains("Agent instructions"));
    }

    #[tokio::test]
    async fn test_system_context_generation() {
        let dir = tempdir().unwrap();
        write(dir.path().join(AGENTS_FILE), "Be helpful").unwrap();
        write(dir.path().join(SOUL_FILE), "Be kind").unwrap();

        let files = WorkspaceFiles::load(dir.path()).await;
        let context = files.to_system_context(false);

        assert!(context.contains("Be helpful"));
        assert!(context.contains("Be kind"));
        assert!(context.contains("AGENTS.md"));
        assert!(context.contains("SOUL.md"));
    }

    #[tokio::test]
    async fn test_fresh_workspace_detection() {
        let dir = tempdir().unwrap();
        write(dir.path().join(BOOTSTRAP_FILE), "First run!").unwrap();

        let files = WorkspaceFiles::load(dir.path()).await;
        assert!(files.is_fresh());
    }
}
