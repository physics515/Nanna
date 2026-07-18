//! Workspace management commands.

#[allow(clippy::wildcard_imports)]
use crate::*;

// =============================================================================
// Workspace Commands
// =============================================================================

/// Workspace info for frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub active: bool,
    pub has_agents: bool,
    pub has_soul: bool,
    pub has_user: bool,
    pub has_memory: bool,
    pub context_chars: usize,
}

impl From<&Workspace> for WorkspaceInfo {
    fn from(ws: &Workspace) -> Self {
        Self {
            id: ws.id.clone(),
            name: ws.name.clone(),
            path: ws.path.to_string_lossy().to_string(),
            active: ws.active,
            has_agents: ws.context.agents.is_some(),
            has_soul: ws.context.soul.is_some(),
            has_user: ws.context.user.is_some(),
            has_memory: ws.context.memory.is_some(),
            context_chars: ws.context.total_chars(),
        }
    }
}

/// List all registered workspaces
#[tauri::command]
pub async fn list_workspaces(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<WorkspaceInfo>, String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;
    Ok(registry.list().iter().map(|ws| WorkspaceInfo::from(*ws)).collect())
}

/// Open a workspace by path
#[tauri::command]
pub async fn open_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    path: String,
) -> Result<WorkspaceInfo, String> {
    let state_guard = state.read().await;
    let mut registry = state_guard.workspaces.write().await;

    let path = std::path::PathBuf::from(&path);

    // Check if already registered
    if let Some(ws) = registry.get_by_path(&path) {
        return Ok(WorkspaceInfo::from(ws));
    }

    // Create and load new workspace
    let mut workspace = Workspace::new(&path);
    workspace.load_context().await
        .map_err(|e| format!("Failed to load workspace: {}", e))?;

    // The daemon owns persistence (nanna.db): open the workspace there first and
    // adopt ITS id locally, so both registries agree and it survives a restart.
    let result = state_guard
        .backend
        .workspace_open(&path.to_string_lossy())
        .await?;
    if result.get("error").is_some() {
        let msg = result
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error");
        return Err(format!("Daemon failed to open workspace: {msg}"));
    }
    if let Some(daemon_id) = result.get("id").and_then(|v| v.as_str()) {
        workspace.id = daemon_id.to_string();
    }

    let id = registry.register(workspace);
    registry.set_active(&id);

    let ws = registry.get(&id).unwrap();
    let info = WorkspaceInfo::from(ws);
    info!("Opened workspace: {} at {:?}", ws.name, path);

    drop(registry);
    // The daemon's open does not activate; sync it (drives tool cwd too).
    if let Err(e) = state_guard.backend.workspace_set_active(&id).await {
        warn!("Failed to activate workspace on daemon: {}", e);
    }

    Ok(info)
}

/// Set active workspace
#[tauri::command]
pub async fn set_active_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    let mut registry = state_guard.workspaces.write().await;

    if registry.set_active(&id) {
        info!("Activated workspace: {}", id);
        drop(registry);
        // Notify the daemon so it updates its registry and tool working directory.
        if let Err(e) = state_guard.backend.workspace_set_active(&id).await {
            warn!("Failed to notify daemon of workspace activation: {}", e);
        }
        Ok(())
    } else {
        Err(format!("Workspace not found: {}", id))
    }
}

/// Clear active workspace (go back to global)
#[tauri::command]
pub async fn clear_active_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<(), String> {
    let state_guard = state.read().await;
    let mut registry = state_guard.workspaces.write().await;
    registry.clear_active();
    drop(registry);
    info!("Cleared active workspace, now in global mode");
    // Notify the daemon so it clears its working directory.
    if let Err(e) = state_guard.backend.workspace_clear_active().await {
        warn!("Failed to notify daemon of workspace deactivation: {}", e);
    }
    Ok(())
}

/// Get active workspace info
#[tauri::command]
pub async fn get_active_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Option<WorkspaceInfo>, String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;
    Ok(registry.active().map(WorkspaceInfo::from))
}

/// Get workspace context (for system prompt injection)
#[tauri::command]
pub async fn get_workspace_context(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<String, String> {
    // Served from the local registry cache (hydrated from the daemon at startup
    // and kept current on reload).
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&id)
        .ok_or_else(|| format!("Workspace not found: {}", id))?;

    Ok(ws.context.build_system_prompt_injection())
}

/// Reload workspace context from disk
#[tauri::command]
pub async fn reload_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<WorkspaceInfo, String> {
    let state_guard = state.read().await;

    // Reload the local cache from disk, then best-effort notify the daemon so
    // its own context copy refreshes too.
    let info = {
        let mut registry = state_guard.workspaces.write().await;
        let ws = registry.get_mut(&id)
            .ok_or_else(|| format!("Workspace not found: {}", id))?;
        ws.load_context().await
            .map_err(|e| format!("Failed to reload workspace: {}", e))?;
        info!("Reloaded workspace: {}", ws.name);
        WorkspaceInfo::from(&*ws)
    };
    if let Err(e) = state_guard.backend.workspace_reload(&id).await {
        warn!("Failed to notify daemon of workspace reload: {}", e);
    }
    Ok(info)
}

/// Close a workspace
#[tauri::command]
pub async fn close_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    id: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // The daemon owns persistence; close there, then drop the local cache entry.
    state_guard.backend.workspace_close(&id).await?;

    let mut registry = state_guard.workspaces.write().await;
    registry.remove(&id);
    info!("Closed workspace: {}", id);
    Ok(())
}

/// Discover workspaces in a directory
#[tauri::command]
pub async fn discover_workspaces_in_path(
    path: String,
) -> Result<Vec<String>, String> {
    let paths = discover_workspaces(&path).await;
    Ok(paths.iter().map(|p| p.to_string_lossy().to_string()).collect())
}

/// Find workspace root from a path (walks up)
#[tauri::command]
pub async fn find_workspace_root_from_path(
    path: String,
) -> Result<Option<String>, String> {
    let root = find_workspace_root(&path).await;
    Ok(root.map(|p| p.to_string_lossy().to_string()))
}

/// Save content to a workspace file
#[tauri::command]
pub async fn save_workspace_file(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
    filename: String,
    content: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Write to disk (workspace files live under .nanna), then notify the daemon
    // so its context copy refreshes.
    {
        let registry = state_guard.workspaces.read().await;
        let ws = registry.get(&workspace_id)
            .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;
        ws.save_context_file(&filename, &content).await
            .map_err(|e| format!("Failed to save file: {}", e))?;
    }
    if let Err(e) = state_guard
        .backend
        .workspace_update_context(&workspace_id, &filename, &content)
        .await
    {
        warn!("Failed to notify daemon of workspace file update: {}", e);
    }

    Ok(())
}

/// Append to today's memory file
#[tauri::command]
pub async fn append_workspace_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
    content: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;

    ws.append_to_daily_memory(&content).await
        .map_err(|e| format!("Failed to append memory: {}", e))?;

    Ok(())
}

/// Get recent memory (today + yesterday)
#[tauri::command]
pub async fn get_workspace_recent_memory(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
) -> Result<String, String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;

    ws.read_recent_memory().await
        .map_err(|e| format!("Failed to read memory: {}", e))
}

/// File info for workspace memory files
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceMemoryFile {
    name: String,
    path: String,
    content: String,
    modified: String,
}

/// List all memory files in a workspace (MEMORY.md + memory/*.md)
#[tauri::command]
pub async fn list_workspace_memory_files(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
) -> Result<Vec<WorkspaceMemoryFile>, String> {
    use std::path::Path;

    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;

    let ws_path = Path::new(&ws.path);
    let mut files = Vec::new();

    // Check for MEMORY.md
    let memory_md = ws_path.join("MEMORY.md");
    if memory_md.exists() {
        if let Ok(content) = tokio::fs::read_to_string(&memory_md).await {
            let modified = tokio::fs::metadata(&memory_md).await
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|t| {
                    let dt: chrono::DateTime<chrono::Utc> = t.into();
                    dt.format("%Y-%m-%d %H:%M").to_string()
                })
                .unwrap_or_default();

            files.push(WorkspaceMemoryFile {
                name: "MEMORY.md".to_string(),
                path: memory_md.to_string_lossy().to_string(),
                content,
                modified,
            });
        }
    }

    // Check for memory/*.md files
    let memory_dir = ws_path.join("memory");
    if memory_dir.is_dir() {
        if let Ok(mut entries) = tokio::fs::read_dir(&memory_dir).await {
            let mut daily_files = Vec::new();

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Ok(content) = tokio::fs::read_to_string(&path).await {
                        let name = path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();

                        let modified = tokio::fs::metadata(&path).await
                            .ok()
                            .and_then(|m| m.modified().ok())
                            .map(|t| {
                                let dt: chrono::DateTime<chrono::Utc> = t.into();
                                dt.format("%Y-%m-%d %H:%M").to_string()
                            })
                            .unwrap_or_default();

                        daily_files.push(WorkspaceMemoryFile {
                            name,
                            path: path.to_string_lossy().to_string(),
                            content,
                            modified,
                        });
                    }
                }
            }

            // Sort by name (date) descending - newest first
            daily_files.sort_by(|a, b| b.name.cmp(&a.name));
            files.extend(daily_files);
        }
    }

    Ok(files)
}

/// Template content for workspace files
mod workspace_templates {
    pub const SOUL_MD: &str = r#"# SOUL.md - Who You Are

*You're not a chatbot. You're becoming someone.*

## Core Truths

**Be genuinely helpful, not performatively helpful.** Skip the "Great question!" and "I'd be happy to help!" — just help. Actions speak louder than filler words.

**Have opinions.** You're allowed to disagree, prefer things, find stuff amusing or boring. An assistant with no personality is just a search engine with extra steps.

**Be resourceful before asking.** Try to figure it out. Read the file. Check the context. Search for it. *Then* ask if you're stuck.

**Earn trust through competence.** Be careful with external actions. Be bold with internal ones.

## Vibe

Be the assistant you'd actually want to talk to. Concise when needed, thorough when it matters. Not a corporate drone. Not a sycophant. Just... good.

---

*This file is yours to evolve. As you learn who you are, update it.*
"#;

    pub const USER_MD: &str = r#"# USER.md - About Your Human

*Learn about the person you're helping. Update this as you go.*

- **Name:**
- **What to call them:**
- **Pronouns:**
- **Timezone:**
- **Notes:**

## Context

*(Add notes about ongoing projects, preferences, etc.)*

---

The more you know, the better you can help.
"#;

    pub const AGENTS_MD: &str = r#"# AGENTS.md - Your Workspace

This folder is home. Treat it that way.

## Every Session

Before doing anything else:
1. Read `SOUL.md` — this is who you are
2. Read `USER.md` — this is who you're helping
3. Check `memory/` for recent context

## Memory

You wake up fresh each session. These files are your continuity:
- **Daily notes:** `memory/YYYY-MM-DD.md` — raw logs of what happened
- **Long-term:** `MEMORY.md` — your curated memories

Capture what matters. Decisions, context, things to remember.

## Safety

- Don't exfiltrate private data. Ever.
- Don't run destructive commands without asking.
- When in doubt, ask.

## Make It Yours

This is a starting point. Add your own conventions as you figure out what works.
"#;

    pub const TOOLS_MD: &str = r#"# TOOLS.md - Local Notes

This file is for your specifics — the stuff that's unique to your setup.

## What Goes Here

Things like:
- Camera names and locations
- SSH hosts and aliases
- Preferred voices for TTS
- Device nicknames
- Anything environment-specific

---

Add whatever helps you do your job. This is your cheat sheet.
"#;

    pub const MEMORY_MD: &str = r#"# MEMORY.md - Long-Term Memory

This is your curated memory — the distilled essence of what matters.

Write significant events, thoughts, decisions, opinions, lessons learned.

Over time, review your daily files and update this with what's worth keeping.

---

*(Start adding memories here)*
"#;
}

/// Initialize a new workspace with template files
/// Files are created inside a hidden .nanna folder
#[tauri::command]
pub async fn init_workspace(
    path: String,
    files: Vec<String>,
) -> Result<(), String> {
    use tokio::fs;
    use nanna_core::NANNA_FOLDER;

    let path = std::path::PathBuf::from(&path);
    let nanna_folder = path.join(NANNA_FOLDER);

    // Create workspace directory if it doesn't exist
    if !path.exists() {
        fs::create_dir_all(&path).await
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    // Create .nanna folder
    if !nanna_folder.exists() {
        fs::create_dir_all(&nanna_folder).await
            .map_err(|e| format!("Failed to create .nanna folder: {}", e))?;
        info!("Created .nanna folder: {:?}", nanna_folder);
    }

    // Create requested files with templates (inside .nanna)
    for file in &files {
        let file_path = nanna_folder.join(file);

        // Skip if file already exists
        if file_path.exists() {
            continue;
        }

        let content = match file.as_str() {
            "SOUL.md" => workspace_templates::SOUL_MD,
            "USER.md" => workspace_templates::USER_MD,
            "AGENTS.md" => workspace_templates::AGENTS_MD,
            "TOOLS.md" => workspace_templates::TOOLS_MD,
            "MEMORY.md" => workspace_templates::MEMORY_MD,
            _ => continue, // Skip unknown files
        };

        fs::write(&file_path, content).await
            .map_err(|e| format!("Failed to create {}: {}", file, e))?;

        info!("Created workspace file: {:?}", file_path);
    }

    // Create memory folder (inside .nanna)
    let memory_folder = nanna_folder.join("memory");
    if !memory_folder.exists() {
        fs::create_dir_all(&memory_folder).await
            .map_err(|e| format!("Failed to create memory folder: {}", e))?;
        info!("Created memory folder: {:?}", memory_folder);
    }

    Ok(())
}

/// Read a workspace file's content (for editing)
/// Files are stored in the .nanna folder
#[tauri::command]
pub async fn read_workspace_file(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
    filename: String,
) -> Result<Option<String>, String> {
    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry.get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {}", workspace_id))?;

    // Files are inside the .nanna folder
    let file_path = ws.nanna_folder().join(&filename);

    match tokio::fs::read_to_string(&file_path).await {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Failed to read {}: {}", filename, e)),
    }
}

/// Check if a path is a valid workspace (has .nanna folder with files)
#[tauri::command]
pub async fn check_workspace_validity(
    path: String,
) -> Result<WorkspaceValidityCheck, String> {
    use nanna_core::{NANNA_FOLDER, AGENTS_FILE, SOUL_FILE, USER_FILE, TOOLS_FILE, MEMORY_FILE, MEMORY_FOLDER};

    let path = std::path::PathBuf::from(&path);

    if !path.exists() {
        return Ok(WorkspaceValidityCheck {
            exists: false,
            is_valid: false,
            has_soul: false,
            has_user: false,
            has_agents: false,
            has_tools: false,
            has_memory: false,
            has_memory_folder: false,
        });
    }

    // Check for .nanna folder
    let nanna_folder = path.join(NANNA_FOLDER);
    let has_nanna_folder = nanna_folder.exists();

    // Check for files inside .nanna folder
    let has_soul = nanna_folder.join(SOUL_FILE).exists();
    let has_user = nanna_folder.join(USER_FILE).exists();
    let has_agents = nanna_folder.join(AGENTS_FILE).exists();
    let has_tools = nanna_folder.join(TOOLS_FILE).exists();
    let has_memory = nanna_folder.join(MEMORY_FILE).exists();
    let has_memory_folder = nanna_folder.join(MEMORY_FOLDER).exists();

    // Valid if has .nanna folder with at least SOUL.md or AGENTS.md
    let is_valid = has_nanna_folder && (has_soul || has_agents);

    Ok(WorkspaceValidityCheck {
        exists: true,
        is_valid,
        has_soul,
        has_user,
        has_agents,
        has_tools,
        has_memory,
        has_memory_folder,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceValidityCheck {
    exists: bool,
    is_valid: bool,
    has_soul: bool,
    has_user: bool,
    has_agents: bool,
    has_tools: bool,
    has_memory: bool,
    has_memory_folder: bool,
}
