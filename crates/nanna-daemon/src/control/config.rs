//! Config handlers for the [`ControlPlane`].

use super::{json, warn, info, ControlPlane, Config, Event, ConfigAction, Value};

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

    /// Make `new_config` the running configuration and push it everywhere a
    /// live change must reach: the agent, then [`Self::propagate_committed`].
    ///
    /// This is the reload the GUI triggers after saving a credential — the step
    /// that makes a post-boot login actually reach the router. The Scheduler
    /// tab rides the same hop, and so does a hand edit of `config.toml` (see
    /// `config_watch`). What it does NOT re-apply is anything wired only at
    /// boot (channels, MCP servers, the webhook server, and the embedding
    /// server and models — see `embedding_reload`).
    pub(super) async fn apply_loaded_config(&self, new_config: Config) {
        let mut config = self.config.write().await;
        *config = new_config;
        info!("Config reloaded from disk");
        if let Some(ref agent) = self.agent {
            agent.apply_llm_config(&config.llm).await;
        }
        let snapshot = config.clone();
        drop(config);
        self.propagate_committed(&snapshot).await;
    }

    /// Push a committed config to everything that holds live settings — the
    /// LLM router's provider set, the scheduler loop, the embedding clients'
    /// Ollama token — and then tell every client.
    ///
    /// The one tail every mutation path (set, reset, reload, import, the file
    /// watcher) shares, so a live setting cannot reach one path and miss
    /// another. The Ollama token did exactly that: every path pushed it to the
    /// chat router, none to the embedder, so a server that wanted the token
    /// refused every memory embed until a restart.
    ///
    /// Called with the config lock released: provider resolution can block on
    /// the keyring or refresh an expired Claude CLI token over the network.
    async fn propagate_committed(&self, config: &Config) {
        self.rebuild_llm_providers(config).await;
        self.apply_scheduler_settings(config).await;
        if let Some(ref live) = self.live_embedding {
            live.apply(config);
        }
        self.notify_config_changed();
    }

    /// Tell every connected client the config changed.
    ///
    /// Carries no payload — see [`Event::ConfigChanged`]. Fired once per
    /// COMMITTED mutation (the in-memory config was replaced and the router
    /// re-derived); a rejected write emits nothing. Fire-and-forget: a send
    /// error only means nobody is subscribed.
    pub(super) fn notify_config_changed(&self) {
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
                let serialized = serde_json::to_value(&*self.config.read().await);
                let config_value = match serialized {
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
            ConfigAction::Set { path, value } => self.config_set(path, value).await,
            ConfigAction::Reset { path } => {
                let mut config = self.config.write().await;

                if let Some(_path) = path {
                    // Reset specific path - would need more complex logic
                    json!({ "error": "partial_reset_not_supported", "hint": "Use Reset without path to reset all" })
                } else {
                    let previous_ollama_host = config.memory.ollama_host.clone();
                    let reset = Config::default().with_env_overrides();
                    if let Err(message) = self.file_brought_in_secrets(&config, &reset).await {
                        warn!("config.reset refused: {message}");
                        return json!({ "error": "secret_store_failed", "message": message });
                    }
                    *config = reset;

                    // Save to disk
                    if let Some(ref config_path) = self.config_path
                        && let Err(e) = config.save_to(config_path)
                    {
                        warn!("Failed to save config: {}", e);
                    }

                    // Propagate to agent service
                    if let Some(ref agent) = self.agent {
                        agent.apply_llm_config(&config.llm).await;
                    }

                    let snapshot = config.clone();
                    drop(config);
                    // The reset moves the Ollama address like a `config.set`
                    // of it: a token saved with no server recorded was the
                    // old address's.
                    let snapshot = self
                        .adopt_moved_ollama_token(&previous_ollama_host, snapshot)
                        .await;
                    self.propagate_committed(&snapshot).await;

                    json!({ "status": "reset" })
                }
            }
            ConfigAction::Reload => self.config_reload().await,
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
                        let previous_ollama_host = config.memory.ollama_host.clone();
                        let imported = cfg.with_env_overrides();
                        if let Err(message) = self.file_brought_in_secrets(&config, &imported).await
                        {
                            warn!("config.import refused: {message}");
                            return json!({ "error": "secret_store_failed", "message": message });
                        }
                        *config = imported;
                        
                        // Save to disk
                        if let Some(ref config_path) = self.config_path
                            && let Err(e) = config.save_to(config_path)
                        {
                            warn!("Failed to save config: {}", e);
                        }
                        
                        info!("Config imported");

                        // Import replaces the whole config, `[llm]` included —
                        // it propagates for the same reason set/reset/reload do.
                        if let Some(ref agent) = self.agent {
                            agent.apply_llm_config(&config.llm).await;
                        }

                        let snapshot = config.clone();
                        drop(config);
                        // As for a reset: the address may have moved.
                        let snapshot = self
                            .adopt_moved_ollama_token(&previous_ollama_host, snapshot)
                            .await;
                        self.propagate_committed(&snapshot).await;

                        json!({ "status": "imported" })
                    }
                    Err(e) => json!({ "error": "import_failed", "message": e })
                }
            }
        }
    }

    /// `ConfigAction::Reload`: load `config.toml` again and apply it live.
    ///
    /// Loaded as the replacement for what is running, so a token saved with
    /// no server recorded stays the running server's rather than following an
    /// address edited in the file. Off the async runtime: keyring reads can
    /// block on an unlock prompt.
    async fn config_reload(&self) -> Value {
        let running_host = self.config.read().await.memory.ollama_host.clone();
        let store = self.credential_store.clone();
        let loaded =
            tokio::task::spawn_blocking(move || Config::load_replacing(&running_host, &store))
                .await;
        match loaded {
            Ok(Ok(new_config)) => {
                self.apply_loaded_config(new_config.with_env_overrides())
                    .await;
                json!({ "status": "reloaded" })
            }
            Ok(Err(e)) => json!({ "error": "reload_failed", "message": e.to_string() }),
            Err(e) => json!({ "error": "reload_failed", "message": e.to_string() }),
        }
    }

    /// `ConfigAction::Set`: write one dotted path, persist, and propagate the change live.
    async fn config_set(&self, path: String, value: Value) -> Value {
        if let Some(instead) = retired_key_set(&path, &value) {
            return json!({ "error": "retired_key", "message": instead, "path": path });
        }
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

        if let Err(e) = set_nested(&mut config_value, &parts, value.clone()) {
            return json!({ "error": "set_failed", "message": e, "path": path });
        }

        // Deserialize back to config
        match serde_json::from_value::<Config>(config_value) {
            Ok(mut new_config) => {
                // The Ollama token is bound to the server it was saved for;
                // edited in place, the config would carry the old server's
                // token to a new `memory.ollama_host`. Dropped here, under the
                // lock, so nothing ever reads it paired with the new address;
                // which token the new address gets is read from the secure
                // store below, once the lock is released.
                let previous_ollama_host = config.memory.ollama_host.clone();
                let ollama_moved = nanna_config::ollama_server_changed(
                    &previous_ollama_host,
                    &new_config.memory.ollama_host,
                );
                if ollama_moved {
                    new_config.llm.ollama_api_key = None;
                }
                if let Err(message) = self.file_brought_in_secrets(&config, &new_config).await {
                    warn!("config.set {path} refused: {message}");
                    return json!({ "error": "secret_store_failed", "message": message, "path": path });
                }
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
                if path.starts_with("llm.")
                    && let Some(ref agent) = self.agent
                {
                    agent.apply_llm_config(&config.llm).await;
                }

                // Re-derive the router's provider set (registration is
                // not boot-only) and the rest of the live settings. Lock
                // released first: resolution can block on keyring/network.
                let snapshot = config.clone();
                drop(config);
                let snapshot = self
                    .adopt_moved_ollama_token(&previous_ollama_host, snapshot)
                    .await;
                self.propagate_committed(&snapshot).await;

                json!({ "status": "updated", "path": path })
            }
            Err(e) => json!({ "error": "invalid_config", "message": e.to_string() })
        }
    }

    /// After a `config.set`, reset or import that may have moved
    /// `[memory].ollama_host` away from `previous_host`, give the running
    /// config the token the new address is owed, if any, and return
    /// `changed` as the rest of the change should propagate it.
    ///
    /// Nothing to do when the address did not move, or when the new config
    /// names a token of its own (an imported one — the operator's say-so, as
    /// a token written into `config.toml` is). Read with no config lock held,
    /// because keyring reads can block on an unlock prompt. A stored token
    /// with no recorded server was `previous_host`'s, and is recorded as such
    /// (see `Config::rebind_ollama_token_if_moved`). The token only lands if
    /// the address is still the one it was resolved for: a change that moved
    /// it again meanwhile resolves its own.
    async fn adopt_moved_ollama_token(&self, previous_host: &str, changed: Config) -> Config {
        let names_its_own = changed
            .llm
            .ollama_api_key
            .as_deref()
            .is_some_and(|token| !token.trim().is_empty());
        if names_its_own
            || !nanna_config::ollama_server_changed(previous_host, &changed.memory.ollama_host)
        {
            return changed;
        }
        let store = self.credential_store.clone();
        let previous_host = previous_host.to_string();
        let mut resolved = changed.clone();
        let resolved = tokio::task::spawn_blocking(move || {
            resolved.rebind_ollama_token_if_moved(&previous_host, &store);
            resolved
        })
        .await;
        let token = match resolved {
            Ok(resolved) => resolved.llm.ollama_api_key,
            Err(e) => {
                warn!("Resolving the Ollama token for the new address failed: {e}");
                None
            }
        };
        let Some(token) = token else {
            return changed;
        };
        let mut config = self.config.write().await;
        if nanna_config::same_ollama_server(&config.memory.ollama_host, &changed.memory.ollama_host)
            && config.llm.ollama_api_key.is_none()
        {
            config.llm.ollama_api_key = Some(token);
        }
        config.clone()
    }

    /// File in the secure store the secrets `changed` brings in over
    /// `previous` (`Config::brings_in_secrets`), before the save that strips
    /// them from `config.toml`. Filed nowhere, a secret set, reset or imported
    /// here worked until the next load — a restart, or the config watcher
    /// reading the save back seconds later — and was gone.
    ///
    /// Only for a control plane that saves: with no config path nothing it
    /// changes outlives it, secrets included. Called with the config write
    /// lock held, so filing and commit are one critical section and two
    /// changes of one secret cannot leave the store holding the one that
    /// lost. The store is written off the async runtime (a keyring write can
    /// wait on an unlock prompt), and only for a change that brings a secret
    /// in — one someone is entering as it happens. A change of anything else
    /// never touches the store.
    ///
    /// An `Err` says the store refused a secret, and the change must be
    /// refused with it: applied, it would work until the next load and then
    /// be gone. Secrets filed before the one refused stay filed.
    async fn file_brought_in_secrets(
        &self,
        previous: &Config,
        changed: &Config,
    ) -> Result<(), String> {
        if self.config_path.is_none() || !changed.brings_in_secrets(previous) {
            return Ok(());
        }
        let filing = changed.clone();
        let store = self.credential_store.clone();
        match tokio::task::spawn_blocking(move || filing.file_secrets_in(&store)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(format!("the secure store refused a secret: {e}")),
            Err(e) => Err(format!("filing secrets in the secure store failed: {e}")),
        }
    }
}

/// Keys the daemon once read and no longer does, each with the sentence
/// saying what replaced it.
///
/// A `config.set` of one would answer `updated` and change nothing: serde
/// drops an unknown key on the round trip, and the next save removes it from
/// disk. Refusing it by name is how a script that used it learns it stopped
/// working.
const RETIRED_KEYS: [(&str, &str); 1] = [(
    "llm.ollama_url",
    "[llm].ollama_url is no longer read: summaries go through chat's router, so an \
     `ollama/` summarization model uses chat's one Ollama server and its token. Set \
     memory.ollama_host to move it.",
)];

/// What replaced the retired key a `set` of `value` at `path` would write, or
/// `None` when it writes none.
///
/// Either the key's own path, or a parent of it given an object that carries
/// the key (`llm` with `{"ollama_url": …}`): the round trip drops the key the
/// same way in both, so both are refused the same way.
fn retired_key_set(path: &str, value: &Value) -> Option<&'static str> {
    RETIRED_KEYS.iter().find_map(|(key, instead)| {
        let written = *key == path
            || key
                .strip_prefix(path)
                .and_then(|rest| rest.strip_prefix('.'))
                .is_some_and(|rest| {
                    rest.split('.')
                        .try_fold(value, |node, part| node.get(part))
                        .is_some()
                });
        written.then_some(*instead)
    })
}

/// Set `value` at the dotted `parts` path inside `obj`, creating intermediate
/// objects as needed (pointer-based access for nested updates).
fn set_nested(obj: &mut Value, parts: &[&str], value: Value) -> Result<(), String> {
    if parts.is_empty() {
        return Err("Empty path".to_string());
    }
    
    if parts.len() == 1 {
        // Final part - set the value
        obj.as_object_mut().map_or_else(
            || Err("Parent is not an object".to_string()),
            |map| {
                map.insert(parts[0].to_string(), value);
                Ok(())
            },
        )
    } else {
        // Navigate deeper
        obj.as_object_mut().map_or_else(
            || Err("Parent is not an object".to_string()),
            |map| {
                let next = map.entry(parts[0]).or_insert(json!({}));
                set_nested(next, &parts[1..], value)
            },
        )
    }
}
