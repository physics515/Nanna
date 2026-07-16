//! Tool handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    // =========================================================================
    // Tool Handlers
    // =========================================================================
    
    pub(super) async fn handle_tool(&self, _client_id: &str, action: ToolAction) -> Value {
        let Some(ref tools) = self.tools else {
            return json!({ "error": "tools_unavailable", "message": "Tool registry not configured" });
        };
        
        match action {
            ToolAction::List => {
                let definitions = tools.definitions().await;
                let tool_list: Vec<_> = definitions.into_iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "enabled": true,
                    }))
                    .collect();
                json!({ "tools": tool_list })
            }
            ToolAction::Get { name } => {
                let definitions = tools.definitions().await;
                if let Some(tool) = definitions.into_iter().find(|t| t.name == name) {
                    json!({ "tool": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }})
                } else {
                    json!({ "error": "not_found", "name": name })
                }
            }
            ToolAction::Enable { name } => self.set_user_tool_enabled(&name, true).await,
            ToolAction::Disable { name } => self.set_user_tool_enabled(&name, false).await,
            ToolAction::Execute { name, input } => {
                use nanna_tools::ToolCall;
                
                let params: std::collections::HashMap<String, Value> = match input {
                    Value::Object(map) => map.into_iter().collect(),
                    _ => std::collections::HashMap::new(),
                };
                
                let call = ToolCall {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: name.clone(),
                    parameters: params,
                };
                
                let result = tools.execute(call).await;
                
                json!({
                    "name": name,
                    "success": result.result.success,
                    "output": result.result.content,
                })
            }
            ToolAction::Create { name, description, code, needs_shell } => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };
                
                // Build permissions
                let permissions = if needs_shell.unwrap_or(false) {
                    Some(crate::user_tools::UserToolPermissions {
                        run: true,
                        ..Default::default()
                    })
                } else {
                    None
                };
                
                match user_tools.create_tool(name.clone(), description, code, None, None, permissions).await {
                    Ok(meta) => {
                        // Register with tool registry immediately
                        if let Some(ref tools) = self.tools {
                            if let Ok(tool_impl) = user_tools.create_tool_impl(&meta) {
                                tools.register_boxed(tool_impl).await;
                            }
                        }
                        
                        info!("Created user tool: {}", name);
                        json!({
                            "status": "created",
                            "tool": {
                                "name": meta.name,
                                "description": meta.description,
                                "language": meta.language,
                                "enabled": meta.enabled,
                                "created_at": meta.created_at,
                            }
                        })
                    }
                    Err(e) => json!({ "error": "create_failed", "message": e })
                }
            }
            ToolAction::Update { name, description, code, needs_shell } => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };
                
                let permissions = needs_shell.map(|ns| {
                    if ns {
                        Some(crate::user_tools::UserToolPermissions {
                            run: true,
                            ..Default::default()
                        })
                    } else {
                        None
                    }
                }).flatten();
                
                match user_tools.update_tool(&name, description, code, None, permissions, None).await {
                    Ok(meta) => {
                        // Make the edit take effect live: drop the old registration
                        // and re-register the new source (if still enabled).
                        self.reconcile_tool_registration(&meta).await;
                        info!("Updated user tool: {}", name);
                        json!({
                            "status": "updated",
                            "tool": {
                                "name": meta.name,
                                "description": meta.description,
                                "language": meta.language,
                                "enabled": meta.enabled,
                                "updated_at": meta.updated_at,
                            }
                        })
                    }
                    Err(e) => json!({ "error": "update_failed", "message": e })
                }
            }
            ToolAction::Delete { name } => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };
                
                match user_tools.delete_tool(&name).await {
                    Ok(()) => {
                        // Make the deletion take effect live: a tool that's gone
                        // from disk must also stop being callable without a daemon
                        // restart (previously it lingered in the registry).
                        if let Some(ref tools) = self.tools {
                            tools.unregister(&name).await;
                        }
                        info!("Deleted user tool: {}", name);
                        json!({ "status": "deleted", "name": name })
                    }
                    Err(e) => json!({ "error": "delete_failed", "message": e })
                }
            }
            ToolAction::Test { code, input } => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };
                
                let input_map: std::collections::HashMap<String, Value> = match input {
                    Value::Object(map) => map.into_iter().collect(),
                    _ => std::collections::HashMap::new(),
                };
                
                match user_tools.test_tool(&code, input_map).await {
                    Ok(output) => json!({ "status": "success", "output": output }),
                    Err(e) => json!({ "status": "error", "error": e })
                }
            }
            ToolAction::GetSource { name } => {
                // Try tools directory first, then user tools
                if let Some(ref dir) = self.tools_dir {
                    let path = dir.join(&name).join("tool.ts");
                    if let Ok(source) = std::fs::read_to_string(&path) {
                        return json!({
                            "name": name,
                            "source": source,
                            "language": "typescript",
                            "path": path.to_string_lossy(),
                        });
                    }
                }
                // Fall back to user tools
                if let Some(ref user_tools) = self.user_tools {
                    if let Some(meta) = user_tools.get_tool(&name).await {
                        return json!({
                            "name": meta.name,
                            "source": meta.source,
                            "language": meta.language,
                        });
                    }
                }
                json!({ "error": "not_found", "name": name })
            }
            ToolAction::ListUser => {
                let Some(ref user_tools) = self.user_tools else {
                    return json!({ "error": "user_tools_unavailable", "message": "User tool manager not configured" });
                };

                let tools = user_tools.list_tools().await;
                let tool_list: Vec<_> = tools.into_iter()
                    .map(|t| json!({
                        "name": t.name,
                        "description": t.description,
                        "source": t.source,
                        "language": t.language,
                        "enabled": t.enabled,
                        "created_at": t.created_at,
                        "updated_at": t.updated_at,
                    }))
                    .collect();
                json!({ "tools": tool_list })
            }
        }
    }
}
