//! Config handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    /// Push `[scheduler]` settings onto the **running** scheduler loop.
    ///
    /// The scheduler captures its config when it starts, so a config write
    /// alone would only take effect at the next daemon restart. That is the
    /// wrong latency for the heartbeat switch in particular: the heartbeat runs
    /// a full agent turn on the same model chat uses, so on a single-slot local
    /// backend it cancels an in-flight generation, and "turn it off" has to mean
    /// *now* — before the benchmark run, not after a restart.
    ///
    /// Takes a snapshot rather than reading `self.config`, so the caller can
    /// release its config guard first: the scheduler lock is only ever taken
    /// with no config lock held.
    async fn apply_scheduler_settings(&self, config: &Config) {
        let Some(scheduler) = self.scheduler.as_ref() else {
            return;
        };
        scheduler.write().await.apply_settings(
            config.scheduler.enabled,
            config.scheduler.heartbeat_enabled,
            std::time::Duration::from_secs(nanna_core::clamp_heartbeat_secs(
                config.scheduler.heartbeat_interval_secs,
            )),
        );
    }

    /// Tell every connected client the config changed.
    ///
    /// Carries no payload — see [`Event::ConfigChanged`]. Fired once per
    /// COMMITTED mutation (the in-memory config was replaced and the router
    /// re-derived); a rejected write emits nothing. Fire-and-forget: a send
    /// error only means nobody is subscribed.
    fn notify_config_changed(&self) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(Event::ConfigChanged);
        }
    }

    // =========================================================================
    // Config Handlers
    // =========================================================================

    /// Rebuild the LLM router's provider set from a config snapshot.
    ///
    /// Provider registration used to happen only at boot; the GUI would save a
    /// credential and call config-reload "so the daemon rebuilds its LLM
    /// client", but nothing did — every call to the newly-authenticated
    /// provider failed with "No provider available" until a daemon restart.
    /// Runs after every config mutation (set/reset/reload/import), with the
    /// config write lock already released — credential resolution may hit the
    /// OS keyring and refresh an expired Claude CLI token over the network.
    async fn rebuild_llm_providers(&self, config: &Config) {
        let Some(router) = self.router.as_ref() else {
            return;
        };
        let llm = crate::server::LlmConfig::from_nanna(config);
        let creds = crate::llm_router::ProviderCredentials::resolve(&llm).await;
        router.rebuild(&creds);
    }

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

                        // Propagate LLM config changes to agent service.
                        // Whole-`[llm]` push, not just the model fields: a
                        // `set` of e.g. `llm.summarization_priority` used to
                        // land on disk and in `self.config` while the running
                        // agent kept summarizing on the boot-time model.
                        if path.starts_with("llm.") {
                            if let Some(ref agent) = self.agent {
                                agent.apply_llm_config(&config.llm).await;
                            }
                        }

                        // Re-derive the router's provider set (registration is
                        // not boot-only). Lock released first: resolution can
                        // block on keyring/network.
                        let snapshot = config.clone();
                        drop(config);
                        self.rebuild_llm_providers(&snapshot).await;
                        self.apply_scheduler_settings(&snapshot).await;
                        self.notify_config_changed();

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
                        agent.apply_llm_config(&config.llm).await;
                    }

                    let snapshot = config.clone();
                    drop(config);
                    self.rebuild_llm_providers(&snapshot).await;
                    self.apply_scheduler_settings(&snapshot).await;
                    self.notify_config_changed();

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
                            agent.apply_llm_config(&config.llm).await;
                        }

                        // This is the reload the GUI triggers after saving a
                        // credential — the step that makes a post-boot login
                        // actually reach the router. The Scheduler tab rides the
                        // same hop: it saves to the shared config file and then
                        // asks for a reload, which is what gets those toggles to
                        // the running scheduler loop.
                        let snapshot = config.clone();
                        drop(config);
                        self.rebuild_llm_providers(&snapshot).await;
                        self.apply_scheduler_settings(&snapshot).await;
                        self.notify_config_changed();

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

                        // Import replaces the whole config, `[llm]` included —
                        // it propagates for the same reason set/reset/reload do.
                        if let Some(ref agent) = self.agent {
                            agent.apply_llm_config(&config.llm).await;
                        }

                        let snapshot = config.clone();
                        drop(config);
                        self.rebuild_llm_providers(&snapshot).await;
                        self.apply_scheduler_settings(&snapshot).await;
                        self.notify_config_changed();

                        json!({ "status": "imported" })
                    }
                    Err(e) => json!({ "error": "import_failed", "message": e })
                }
            }
        }
    }
}
