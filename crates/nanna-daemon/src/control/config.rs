//! Config handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    // =========================================================================
    // Config Handlers
    // =========================================================================
    
    pub(super) async fn handle_config(&self, _client_id: &str, action: ConfigAction) -> Value {
        match action {
            ConfigAction::Get { path } => {
                let config = self.config.read().await;
                let config_value = match serde_json::to_value(&*config) {
                    Ok(v) => v,
                    Err(e) => return json!({ "error": "serialize_failed", "message": e.to_string() }),
                };
                
                if let Some(path) = path {
                    // Get nested value by path (e.g., "llm.model")
                    let parts: Vec<&str> = path.split('.').collect();
                    let mut current = &config_value;
                    for part in parts {
                        match current.get(part) {
                            Some(v) => current = v,
                            None => return json!({ "error": "path_not_found", "path": path })
                        }
                    }
                    json!({ "value": current, "path": path })
                } else {
                    json!({ "config": config_value })
                }
            }
            ConfigAction::Set { path, value } => {
                let mut config = self.config.write().await;
                let mut config_value = match serde_json::to_value(&*config) {
                    Ok(v) => v,
                    Err(e) => return json!({ "error": "serialize_failed", "message": e.to_string() }),
                };
                
                // Set nested value by path using a helper function
                let parts: Vec<&str> = path.split('.').collect();
                if parts.is_empty() {
                    return json!({ "error": "invalid_path", "path": path });
                }
                
                // Use pointer-based access for nested updates
                fn set_nested(obj: &mut Value, parts: &[&str], value: Value) -> Result<(), String> {
                    if parts.is_empty() {
                        return Err("Empty path".to_string());
                    }
                    
                    if parts.len() == 1 {
                        // Final part - set the value
                        if let Some(map) = obj.as_object_mut() {
                            map.insert(parts[0].to_string(), value);
                            Ok(())
                        } else {
                            Err("Parent is not an object".to_string())
                        }
                    } else {
                        // Navigate deeper
                        if let Some(map) = obj.as_object_mut() {
                            let next = map.entry(parts[0]).or_insert(json!({}));
                            set_nested(next, &parts[1..], value)
                        } else {
                            Err("Parent is not an object".to_string())
                        }
                    }
                }
                
                if let Err(e) = set_nested(&mut config_value, &parts, value.clone()) {
                    return json!({ "error": "set_failed", "message": e, "path": path });
                }
                
                // Deserialize back to config
                match serde_json::from_value::<Config>(config_value) {
                    Ok(new_config) => {
                        *config = new_config;

                        // Save to disk if we have a path
                        if let Some(ref config_path) = self.config_path {
                            if let Err(e) = config.save_to(config_path) {
                                warn!("Failed to save config: {}", e);
                            } else {
                                info!("Config saved to {:?}", config_path);
                            }
                        }

                        // Propagate LLM config changes to agent service
                        if path.starts_with("llm.") {
                            if let Some(ref agent) = self.agent {
                                let model = if config.llm.model_priority.is_empty() {
                                    Some(config.llm.model.clone())
                                } else {
                                    config.llm.model_priority.first().cloned()
                                };
                                agent.update_config(
                                    model,
                                    Some(config.llm.model_priority.clone()),
                                ).await;
                            }
                        }

                        json!({ "status": "updated", "path": path })
                    }
                    Err(e) => json!({ "error": "invalid_config", "message": e.to_string() })
                }
            }
            ConfigAction::Reset { path } => {
                let mut config = self.config.write().await;

                if let Some(_path) = path {
                    // Reset specific path - would need more complex logic
                    json!({ "error": "partial_reset_not_supported", "hint": "Use Reset without path to reset all" })
                } else {
                    *config = Config::default().with_env_overrides();

                    // Save to disk
                    if let Some(ref config_path) = self.config_path {
                        if let Err(e) = config.save_to(config_path) {
                            warn!("Failed to save config: {}", e);
                        }
                    }

                    // Propagate to agent service
                    if let Some(ref agent) = self.agent {
                        let model = if config.llm.model_priority.is_empty() {
                            Some(config.llm.model.clone())
                        } else {
                            config.llm.model_priority.first().cloned()
                        };
                        agent.update_config(
                            model,
                            Some(config.llm.model_priority.clone()),
                        ).await;
                    }

                    json!({ "status": "reset" })
                }
            }
            ConfigAction::Reload => {
                match Config::load() {
                    Ok(new_config) => {
                        let mut config = self.config.write().await;
                        *config = new_config.with_env_overrides();
                        info!("Config reloaded from disk");

                        // Propagate to agent service
                        if let Some(ref agent) = self.agent {
                            let model = if config.llm.model_priority.is_empty() {
                                Some(config.llm.model.clone())
                            } else {
                                config.llm.model_priority.first().cloned()
                            };
                            agent.update_config(
                                model,
                                Some(config.llm.model_priority.clone()),
                            ).await;
                        }

                        json!({ "status": "reloaded" })
                    }
                    Err(e) => json!({ "error": "reload_failed", "message": e.to_string() })
                }
            }
            ConfigAction::Export => {
                let config = self.config.read().await;
                // Export as JSON (TOML export would require additional dependencies)
                match serde_json::to_value(&*config) {
                    Ok(v) => json!({ "config": v, "format": "json" }),
                    Err(e) => json!({ "error": "export_failed", "message": e.to_string() })
                }
            }
            ConfigAction::Import { config: config_value } => {
                // Parse as JSON object (TOML parsing removed for simplicity)
                let new_config: Result<Config, String> = 
                    serde_json::from_value(config_value).map_err(|e| e.to_string());
                
                match new_config {
                    Ok(cfg) => {
                        let mut config = self.config.write().await;
                        *config = cfg.with_env_overrides();
                        
                        // Save to disk
                        if let Some(ref config_path) = self.config_path {
                            if let Err(e) = config.save_to(config_path) {
                                warn!("Failed to save config: {}", e);
                            }
                        }
                        
                        info!("Config imported");
                        json!({ "status": "imported" })
                    }
                    Err(e) => json!({ "error": "import_failed", "message": e })
                }
            }
        }
    }
}
