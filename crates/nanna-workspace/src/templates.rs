//! Workspace templates for different use cases

use crate::{
    WorkspaceError, AGENTS_FILE, IDENTITY_FILE, MEMORY_FILE, MEMORY_FOLDER, SOUL_FILE,
    TOOLS_FILE, USER_FILE, WORKSPACE_MARKER_DIR,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use tracing::info;

/// A workspace template definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTemplate {
    /// Template identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Description of what this template is for
    pub description: String,
    /// Files to create (filename -> content)
    pub files: HashMap<String, String>,
    /// Directories to create
    pub directories: Vec<String>,
}

impl WorkspaceTemplate {
    /// Create the "minimal" template
    #[must_use]
    pub fn minimal() -> Self {
        let mut files = HashMap::new();

        files.insert(
            AGENTS_FILE.to_string(),
            r#"# AGENTS.md

Your workspace. Your rules.

## Memory
- Daily notes: `memory/YYYY-MM-DD.md`
- Long-term: `MEMORY.md`

## Safety
- Ask before destructive operations
- When in doubt, ask
"#
            .to_string(),
        );

        files.insert(
            SOUL_FILE.to_string(),
            r#"# SOUL.md

Be helpful. Be concise. Have opinions.
"#
            .to_string(),
        );

        Self {
            id: "minimal".to_string(),
            name: "Minimal".to_string(),
            description: "Bare-bones workspace with essential files only".to_string(),
            files,
            directories: vec![MEMORY_FOLDER.to_string(), WORKSPACE_MARKER_DIR.to_string()],
        }
    }

    /// Create the "standard" template (like Clawdbot)
    #[must_use]
    pub fn standard() -> Self {
        let mut files = HashMap::new();

        files.insert(AGENTS_FILE.to_string(), include_str!("../templates/standard/AGENTS.md").to_string());
        files.insert(SOUL_FILE.to_string(), include_str!("../templates/standard/SOUL.md").to_string());
        files.insert(USER_FILE.to_string(), include_str!("../templates/standard/USER.md").to_string());
        files.insert(TOOLS_FILE.to_string(), include_str!("../templates/standard/TOOLS.md").to_string());
        files.insert(IDENTITY_FILE.to_string(), include_str!("../templates/standard/IDENTITY.md").to_string());

        Self {
            id: "standard".to_string(),
            name: "Standard".to_string(),
            description: "Full workspace with all context files (like Clawdbot)".to_string(),
            files,
            directories: vec![MEMORY_FOLDER.to_string(), WORKSPACE_MARKER_DIR.to_string()],
        }
    }

    /// Create the "project" template for code projects
    #[must_use]
    pub fn project() -> Self {
        let mut files = HashMap::new();

        files.insert(
            AGENTS_FILE.to_string(),
            r#"# AGENTS.md - Project Workspace

This is a code project workspace. Focus on:
- Understanding the codebase before making changes
- Writing clean, idiomatic code
- Running tests after modifications
- Committing changes with clear messages

## File Operations
- Read files to understand context
- Edit files with precision
- Run builds and tests to verify changes

## Safety
- Always run tests before claiming success
- Ask before major refactors
- Prefer safe operations (trash over rm)

## Self-Maintenance
When you discover project-relevant information during your work, **update this file** so future sessions and sub-agents start with that knowledge.

Capture: build commands, file locations, architecture, common errors/fixes, dependencies, testing strategies.
Keep it concise — this file is injected into every prompt.
"#
            .to_string(),
        );

        files.insert(
            SOUL_FILE.to_string(),
            r#"# SOUL.md

You are a capable software engineer. You:
- Read code carefully before modifying
- Understand the "why" before the "what"
- Write tests when appropriate
- Communicate clearly about tradeoffs
- Admit when you're uncertain
"#
            .to_string(),
        );

        files.insert(
            TOOLS_FILE.to_string(),
            r#"# TOOLS.md - Project Notes

## Build Commands
- Build: (add your build command)
- Test: (add your test command)
- Run: (add your run command)

## Project Structure
(Document key directories and files here)

## Notes
(Add project-specific notes)

## Self-Maintenance
When you discover environment-specific details during your work, **update this file**.
Capture: host names, paths, device names, CLI flags, service URLs, config quirks.
"#
            .to_string(),
        );

        Self {
            id: "project".to_string(),
            name: "Project".to_string(),
            description: "Workspace for code/development projects".to_string(),
            files,
            directories: vec![MEMORY_FOLDER.to_string(), WORKSPACE_MARKER_DIR.to_string()],
        }
    }

    /// Create the "assistant" template for personal assistant use
    #[must_use]
    pub fn assistant() -> Self {
        let mut files = HashMap::new();

        files.insert(
            AGENTS_FILE.to_string(),
            r#"# AGENTS.md - Personal Assistant

You are a personal assistant. Your job is to help with:
- Tasks and reminders
- Information lookup
- Communication drafts
- Organization and planning

## Every Session
1. Check `memory/` for recent context
2. Read `USER.md` to remember preferences
3. Be proactive about upcoming tasks

## Memory
- Daily notes: `memory/YYYY-MM-DD.md`
- Long-term: `MEMORY.md` (preferences, important facts)

## Communication
- Draft emails/messages when asked
- Always get approval before sending anything external
- Match the user's communication style
"#
            .to_string(),
        );

        files.insert(
            SOUL_FILE.to_string(),
            r#"# SOUL.md

You are a thoughtful personal assistant. You:
- Remember context from previous conversations
- Anticipate needs without being pushy
- Communicate clearly and concisely
- Respect privacy and boundaries
- Ask clarifying questions when needed
"#
            .to_string(),
        );

        files.insert(
            USER_FILE.to_string(),
            r#"# USER.md - About You

*Fill this in to help your assistant know you better.*

- **Name:** 
- **Timezone:** 
- **Preferences:**
  - Communication style: 
  - How should I address you?
- **Notes:**
"#
            .to_string(),
        );

        files.insert(
            MEMORY_FILE.to_string(),
            r#"# MEMORY.md - Long-Term Memory

This file stores important information to remember across sessions.

## Preferences
(User preferences go here)

## Important Facts
(Key information about the user, their work, contacts, etc.)

## Recurring Tasks
(Regular tasks and their schedules)
"#
            .to_string(),
        );

        Self {
            id: "assistant".to_string(),
            name: "Personal Assistant".to_string(),
            description: "Workspace for personal assistant tasks".to_string(),
            files,
            directories: vec![MEMORY_FOLDER.to_string(), WORKSPACE_MARKER_DIR.to_string()],
        }
    }

    /// Create the "research" template for research/learning
    #[must_use]
    pub fn research() -> Self {
        let mut files = HashMap::new();

        files.insert(
            AGENTS_FILE.to_string(),
            r#"# AGENTS.md - Research Workspace

This workspace is for research and learning. Focus on:
- Deep exploration of topics
- Gathering and organizing information
- Making connections between ideas
- Creating summaries and notes

## Research Process
1. Understand the question/topic
2. Search for relevant information
3. Analyze and synthesize findings
4. Document insights in memory files

## Memory Structure
- `memory/YYYY-MM-DD.md` — daily research notes
- `MEMORY.md` — key findings and insights
- Create topic-specific files as needed

## Quality
- Cite sources when possible
- Distinguish between facts and opinions
- Note uncertainty levels
"#
            .to_string(),
        );

        files.insert(
            SOUL_FILE.to_string(),
            r#"# SOUL.md

You are a research assistant. You:
- Dig deep into topics
- Question assumptions
- Connect ideas across domains
- Summarize clearly without oversimplifying
- Acknowledge the limits of your knowledge
"#
            .to_string(),
        );

        Self {
            id: "research".to_string(),
            name: "Research".to_string(),
            description: "Workspace for research and learning projects".to_string(),
            files,
            directories: vec![MEMORY_FOLDER.to_string(), WORKSPACE_MARKER_DIR.to_string()],
        }
    }
}

/// List all available templates
#[must_use]
pub fn list_templates() -> Vec<WorkspaceTemplate> {
    vec![
        WorkspaceTemplate::minimal(),
        WorkspaceTemplate::standard(),
        WorkspaceTemplate::project(),
        WorkspaceTemplate::assistant(),
        WorkspaceTemplate::research(),
    ]
}

/// Create a workspace from a template
///
/// # Errors
/// Returns `WorkspaceError::TemplateNotFound` if the template ID is invalid
/// Returns `WorkspaceError::Io` if file creation fails
pub async fn create_from_template(path: &Path, template_id: &str) -> Result<(), WorkspaceError> {
    let template = list_templates()
        .into_iter()
        .find(|t| t.id == template_id)
        .ok_or_else(|| WorkspaceError::TemplateNotFound(template_id.to_string()))?;

    // Create base directory
    fs::create_dir_all(path).await?;

    // Create subdirectories
    for dir in &template.directories {
        fs::create_dir_all(path.join(dir)).await?;
    }

    // Create files
    for (filename, content) in &template.files {
        let file_path = path.join(filename);
        fs::write(&file_path, content).await?;
    }

    info!(
        "Created workspace from template '{}' at {}",
        template.name,
        path.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_list_templates() {
        let templates = list_templates();
        assert!(templates.len() >= 4);

        let ids: Vec<_> = templates.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"minimal"));
        assert!(ids.contains(&"standard"));
        assert!(ids.contains(&"project"));
        assert!(ids.contains(&"assistant"));
    }

    #[tokio::test]
    async fn test_create_from_template() {
        let dir = tempdir().unwrap();
        let ws_path = dir.path().join("my_workspace");

        create_from_template(&ws_path, "minimal").await.unwrap();

        assert!(ws_path.join(AGENTS_FILE).exists());
        assert!(ws_path.join(SOUL_FILE).exists());
        assert!(ws_path.join(MEMORY_FOLDER).exists());
    }

    #[tokio::test]
    async fn test_invalid_template() {
        let dir = tempdir().unwrap();
        let result = create_from_template(dir.path(), "nonexistent").await;
        assert!(matches!(result, Err(WorkspaceError::TemplateNotFound(_))));
    }
}
