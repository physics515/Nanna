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
    pub has_readme: bool,
    pub has_agents: bool,
    pub has_contributing: bool,
    pub has_roadmap: bool,
    pub context_chars: usize,
}

impl From<&Workspace> for WorkspaceInfo {
    fn from(ws: &Workspace) -> Self {
        Self {
            id: ws.id.clone(),
            name: ws.name.clone(),
            path: ws.path.to_string_lossy().to_string(),
            active: ws.active,
            has_readme: ws.context.readme.is_some(),
            has_agents: ws.context.agents.is_some(),
            has_contributing: ws.context.contributing.is_some(),
            has_roadmap: ws.context.roadmap.is_some(),
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

    // Write to disk (standard project files at workspace root), then notify the daemon
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


/// Initialize a minimal workspace at path (root AGENTS.md + optional ROADMAP.md).
///
/// Persona/user/memory are NOT scaffolded — they live in global config + the DB store.
#[tauri::command]
pub async fn init_workspace(
    state: State<'_, Arc<RwLock<AppState>>>,
    path: String,
    files: Vec<String>,
) -> Result<WorkspaceInfo, String> {
    let path = std::path::PathBuf::from(&path);
    if !path.exists() {
        tokio::fs::create_dir_all(&path).await
            .map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    let mut workspace = Workspace::new(&path);
    let with_roadmap = files.iter().any(|f| f == "ROADMAP.md" || f == "roadmap");
    // Always create AGENTS.md; ROADMAP only if requested
    workspace
        .initialize_minimal(with_roadmap)
        .await
        .map_err(|e| format!("Failed to initialize workspace: {e}"))?;

    // Honour any requested standard context files beyond the defaults
    for file in &files {
        if file == "AGENTS.md" || file == "ROADMAP.md" {
            continue; // handled by initialize_minimal
        }
        if nanna_core::STANDARD_CONTEXT_FILES.contains(&file.as_str()) {
            let fp = path.join(file);
            if !fp.exists() {
                let content = match file.as_str() {
                    "README.md" => format!("# {}\n", workspace.name),
                    "CONTRIBUTING.md" => "# Contributing\n\n(How to work in this repo.)\n".to_string(),
                    _ => continue,
                };
                tokio::fs::write(&fp, content).await
                    .map_err(|e| format!("Failed to write {file}: {e}"))?;
            }
        }
    }

    workspace
        .load_context()
        .await
        .map_err(|e| format!("Failed to load workspace: {e}"))?;

    // Open on the daemon so persistence agrees
    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .workspace_open(&path.to_string_lossy())
        .await?;
    if let Some(id) = result.get("id").and_then(|v| v.as_str()) {
        workspace.id = id.to_string();
    }

    let info = WorkspaceInfo::from(&workspace);
    let mut registry = state_guard.workspaces.write().await;
    registry.register(workspace);
    Ok(info)
}

/// Read a standard context file from the workspace root
#[tauri::command]
pub async fn read_workspace_file(
    state: State<'_, Arc<RwLock<AppState>>>,
    workspace_id: String,
    filename: String,
) -> Result<Option<String>, String> {
    nanna_core::validate_context_filename(&filename)
        .map_err(|e| e.to_string())?;

    let state_guard = state.read().await;
    let registry = state_guard.workspaces.read().await;

    let ws = registry
        .get(&workspace_id)
        .ok_or_else(|| format!("Workspace not found: {workspace_id}"))?;

    let file_path = ws.path.join(&filename);
    match tokio::fs::read_to_string(&file_path).await {
        Ok(content) => Ok(Some(content)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Failed to read {filename}: {e}")),
    }
}

/// Check if a path looks like a valid workspace (standard project signals)
#[tauri::command]
pub async fn check_workspace_validity(path: String) -> Result<WorkspaceValidityCheck, String> {
    use nanna_core::{
        AGENTS_FILE, CONTRIBUTING_FILE, README_FILE, ROADMAP_FILE, WORKSPACE_MARKERS,
    };

    let path = std::path::PathBuf::from(&path);

    if !path.exists() {
        return Ok(WorkspaceValidityCheck {
            exists: false,
            is_valid: false,
            has_readme: false,
            has_agents: false,
            has_contributing: false,
            has_roadmap: false,
            has_git: false,
            has_manifest: false,
        });
    }

    let has_readme = path.join(README_FILE).exists();
    let has_agents = path.join(AGENTS_FILE).exists();
    let has_contributing = path.join(CONTRIBUTING_FILE).exists();
    let has_roadmap = path.join(ROADMAP_FILE).exists();
    let has_git = path.join(".git").exists();
    let has_manifest = ["Cargo.toml", "package.json", "pyproject.toml", "go.mod"]
        .iter()
        .any(|m| path.join(m).exists());

    // Valid if any standard project signal is present
    let is_valid = WORKSPACE_MARKERS.iter().any(|m| path.join(m).exists());

    Ok(WorkspaceValidityCheck {
        exists: true,
        is_valid,
        has_readme,
        has_agents,
        has_contributing,
        has_roadmap,
        has_git,
        has_manifest,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceValidityCheck {
    exists: bool,
    is_valid: bool,
    has_readme: bool,
    has_agents: bool,
    has_contributing: bool,
    has_roadmap: bool,
    has_git: bool,
    has_manifest: bool,
}
