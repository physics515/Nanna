//! Workspace file loading and management

use crate::{
    WorkspaceError, AGENTS_FILE, CONTRIBUTING_FILE, README_FILE, ROADMAP_FILE,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

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

/// Collection of standard project context files
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceFiles {
    /// README.md — what the project is
    pub readme: Option<WorkspaceFile>,
    /// AGENTS.md — agent instructions for this repo
    pub agents: Option<WorkspaceFile>,
    /// CONTRIBUTING.md — conventions
    pub contributing: Option<WorkspaceFile>,
    /// ROADMAP.md — plan / checklist
    pub roadmap: Option<WorkspaceFile>,
}

impl WorkspaceFiles {
    /// Load standard project files from a root directory
    pub async fn load(root: &Path) -> Self {
        Self {
            readme: Self::load_if_exists(root, README_FILE).await,
            agents: Self::load_if_exists(root, AGENTS_FILE).await,
            contributing: Self::load_if_exists(root, CONTRIBUTING_FILE).await,
            roadmap: Self::load_if_exists(root, ROADMAP_FILE).await,
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

    /// Generate system prompt context from loaded standard project files
    #[must_use]
    pub fn to_system_context(&self) -> String {
        let mut sections = Vec::new();

        if let Some(ref readme) = self.readme {
            if readme.has_content() {
                sections.push(format!("## README.md\n{}", readme.content));
            }
        }
        if let Some(ref agents) = self.agents {
            if agents.has_content() {
                sections.push(format!("## AGENTS.md\n{}", agents.content));
            }
        }
        if let Some(ref contributing) = self.contributing {
            if contributing.has_content() {
                sections.push(format!("## CONTRIBUTING.md\n{}", contributing.content));
            }
        }
        if let Some(ref roadmap) = self.roadmap {
            if roadmap.has_content() {
                sections.push(format!("## ROADMAP.md\n{}", roadmap.content));
            }
        }

        if sections.is_empty() {
            String::new()
        } else {
            format!(
                "# Project Context\n\nThe following project files have been loaded:\n\n{}",
                sections.join("\n\n")
            )
        }
    }

    /// Get list of existing files
    #[must_use]
    pub fn existing_files(&self) -> Vec<&WorkspaceFile> {
        let mut files = Vec::new();
        if let Some(ref f) = self.readme {
            if f.exists {
                files.push(f);
            }
        }
        if let Some(ref f) = self.agents {
            if f.exists {
                files.push(f);
            }
        }
        if let Some(ref f) = self.contributing {
            if f.exists {
                files.push(f);
            }
        }
        if let Some(ref f) = self.roadmap {
            if f.exists {
                files.push(f);
            }
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
        write(dir.path().join(README_FILE), "# Readme").unwrap();

        let files = WorkspaceFiles::load(dir.path()).await;

        assert!(files.agents.is_some());
        assert!(files.readme.is_some());
        assert!(files.roadmap.is_none());
        assert!(files.agents.unwrap().content.contains("Agent instructions"));
    }

    #[tokio::test]
    async fn test_system_context_generation() {
        let dir = tempdir().unwrap();
        write(dir.path().join(AGENTS_FILE), "Be helpful").unwrap();
        write(dir.path().join(README_FILE), "Project X").unwrap();

        let files = WorkspaceFiles::load(dir.path()).await;
        let context = files.to_system_context();

        assert!(context.contains("Be helpful"));
        assert!(context.contains("Project X"));
        assert!(context.contains("AGENTS.md"));
        assert!(context.contains("README.md"));
        assert!(!context.contains("SOUL.md"));
        assert!(!context.contains("MEMORY.md"));
    }
}
