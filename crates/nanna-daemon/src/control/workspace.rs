//! Workspace handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    // =========================================================================
    // Workspace Handlers
    // =========================================================================
    
    /// Persist current workspace registry to the database
    async fn save_workspaces(&self) {
        let Some(ref storage) = self.storage else { return };
        let registry = self.workspaces.read().await;
        let repo = storage.workspaces();
        for ws in registry.list() {
            let record = nanna_storage::WorkspaceRecord {
                id: ws.id.clone(),
                name: ws.name.clone(),
                path: ws.path.display().to_string(),
                active: ws.active,
                created_at: String::new(), // DB handles default
                last_accessed: String::new(), // DB handles default
            };
            if let Err(e) = repo.upsert(&record).await {
                error!("Failed to save workspace {}: {}", ws.name, e);
            }
        }
    }

    pub(super) async fn handle_workspace(&self, _client_id: &str, action: WorkspaceAction) -> Value {
        match action {
            WorkspaceAction::List => {
                let registry = self.workspaces.read().await;
                let workspaces: Vec<_> = registry.list().iter()
                    .map(|ws| json!({
                        "id": ws.id,
                        "name": ws.name,
                        "path": ws.path,
                        "active": ws.active,
                        "last_accessed": ws.last_accessed,
                    }))
                    .collect();
                let active_id = registry.active().map(|ws| ws.id.clone());
                json!({ "workspaces": workspaces, "active_id": active_id })
            }
            WorkspaceAction::Get { id } => {
                let registry = self.workspaces.read().await;
                if let Some(ws) = registry.get(&id) {
                    json!({
                        "workspace": {
                            "id": ws.id,
                            "name": ws.name,
                            "path": ws.path,
                            "active": ws.active,
                            "last_accessed": ws.last_accessed,
                            "metadata": ws.metadata,
                            "context_loaded": !ws.context.is_empty(),
                        }
                    })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::Open { path } => {
                let path = PathBuf::from(&path);
                
                // Check if workspace already registered
                {
                    let registry = self.workspaces.read().await;
                    if let Some(existing) = registry.get_by_path(&path) {
                        return json!({ 
                            "status": "already_registered", 
                            "id": existing.id,
                            "name": existing.name,
                        });
                    }
                }
                
                // Check if path is a valid workspace
                if !nanna_core::is_workspace_root(&path).await {
                    // Create .nanna folder to make it a workspace
                    let nanna_folder = path.join(nanna_core::NANNA_FOLDER);
                    if let Err(e) = tokio::fs::create_dir_all(&nanna_folder).await {
                        return json!({ "error": "create_failed", "message": e.to_string() });
                    }
                    info!("Created workspace at {:?}", path);
                }
                
                // Create and register workspace
                let mut ws = Workspace::new(&path);
                if let Err(e) = ws.load_context().await {
                    warn!("Failed to load workspace context: {}", e);
                }
                
                let id = ws.id.clone();
                let name = ws.name.clone();
                
                let mut registry = self.workspaces.write().await;
                registry.register(ws);
                
                info!("Registered workspace: {} ({})", name, id);
                self.save_workspaces().await;
                json!({ "status": "opened", "id": id, "name": name })
            }
            WorkspaceAction::Close { id } => {
                let mut registry = self.workspaces.write().await;
                if let Some(ws) = registry.remove(&id) {
                    info!("Closed workspace: {} ({})", ws.name, id);
                    drop(registry);
                    // Remove from database
                    if let Some(ref storage) = self.storage {
                        let _ = storage.workspaces().delete(&id).await;
                    }
                    json!({ "status": "closed", "id": id })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::SetActive { id } => {
                let mut registry = self.workspaces.write().await;
                if registry.set_active(&id) {
                    let ws_path = registry.get(&id).map(|ws| ws.path.clone());
                    let name = registry.get(&id).map(|ws| ws.name.clone());
                    drop(registry);
                    // Update tool registry's default working directory to workspace path
                    if let (Some(tools), Some(path)) = (&self.tools, &ws_path) {
                        tools.set_default_workdir(Some(path.clone())).await;
                        info!("Set tool working directory to {:?}", path);
                    }
                    info!("Set active workspace: {:?}", name);
                    self.save_workspaces().await;
                    json!({ "status": "activated", "id": id, "name": name })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::ClearActive => {
                let mut registry = self.workspaces.write().await;
                registry.clear_active();
                drop(registry);
                // Clear tool registry's default working directory
                if let Some(ref tools) = self.tools {
                    tools.set_default_workdir(None).await;
                }
                info!("Cleared active workspace (global mode)");
                self.save_workspaces().await;
                json!({ "status": "cleared" })
            }
            WorkspaceAction::Reload { id } => {
                let mut registry = self.workspaces.write().await;
                if let Some(ws) = registry.get_mut(&id) {
                    match ws.load_context().await {
                        Ok(()) => {
                            info!("Reloaded workspace context: {}", ws.name);
                            json!({ 
                                "status": "reloaded", 
                                "id": id,
                                "context_chars": ws.context.total_chars(),
                            })
                        }
                        Err(e) => json!({ "error": "reload_failed", "message": e.to_string() })
                    }
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::GetContext { id } => {
                let registry = self.workspaces.read().await;
                if let Some(ws) = registry.get(&id) {
                    json!({
                        "context": {
                            "agents": ws.context.agents,
                            "soul": ws.context.soul,
                            "user": ws.context.user,
                            "tools": ws.context.tools,
                            "memory": ws.context.memory,
                            "identity": ws.context.identity,
                            "heartbeat": ws.context.heartbeat,
                        },
                        "total_chars": ws.context.total_chars(),
                        "system_prompt_injection": ws.context.build_system_prompt_injection(),
                    })
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
            WorkspaceAction::UpdateContext { id, file, content } => {
                // Validate file name
                let valid_files = [
                    nanna_core::AGENTS_FILE,
                    nanna_core::SOUL_FILE,
                    nanna_core::USER_FILE,
                    nanna_core::TOOLS_FILE,
                    nanna_core::MEMORY_FILE,
                    nanna_core::IDENTITY_FILE,
                    nanna_core::HEARTBEAT_FILE,
                ];
                
                if !valid_files.contains(&file.as_str()) {
                    return json!({ 
                        "error": "invalid_file", 
                        "file": file,
                        "valid_files": valid_files,
                    });
                }
                
                let registry = self.workspaces.read().await;
                if let Some(ws) = registry.get(&id) {
                    match ws.save_context_file(&file, &content).await {
                        Ok(()) => {
                            info!("Updated workspace file: {} in {}", file, ws.name);
                            json!({ "status": "updated", "id": id, "file": file })
                        }
                        Err(e) => json!({ "error": "save_failed", "message": e.to_string() })
                    }
                } else {
                    json!({ "error": "not_found", "id": id })
                }
            }
        }
    }
}
