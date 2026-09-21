//! Cross-domain unit tests for the [`ControlPlane`].

use super::*;
use nanna_channels::ConnectionState;

#[test]
fn uptime_starts_near_zero_and_is_monotonic() {
    let cp = ControlPlane::new(Arc::new(SessionManager::new()));
    let first = cp.uptime_secs();
    assert!(
        first < 5,
        "a freshly created control plane should report ~0 uptime"
    );
    let second = cp.uptime_secs();
    assert!(second >= first, "uptime must be monotonic non-decreasing");
}

#[tokio::test]
async fn channel_status_reports_registered_state() {
    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    let sm = Arc::new(StatusManager::new());
    sm.register("telegram", "Telegram", true, true).await;
    sm.set_state("telegram", ConnectionState::Connected, None).await;
    cp.set_status_manager(Arc::clone(&sm));
    let cp = Arc::new(cp);

    // Single-channel query
    let one = cp
        .handle(
            "test",
            Action::Channel(ChannelAction::Status {
                id: Some("telegram".into()),
            }),
        )
        .await;
    assert_eq!(one["channel"]["provider"], "telegram");
    assert_eq!(one["channel"]["state"], "connected");
    assert_eq!(one["channel"]["configured"], true);

    // All-channel query includes summary
    let all = cp
        .handle(
            "test",
            Action::Channel(ChannelAction::Status { id: None }),
        )
        .await;
    assert_ne!(all["channels"].as_array().unwrap(), &Vec::<Value>::new());
    assert_eq!(all["summary"]["connected"], 1);
    assert_eq!(all["summary"]["configured"], 1);

    // Missing id → not_found
    let missing = cp
        .handle(
            "test",
            Action::Channel(ChannelAction::Status {
                id: Some("nope".into()),
            }),
        )
        .await;
    assert_eq!(missing["error"], "not_found");
}

#[tokio::test]
async fn channel_status_unavailable_without_manager() {
    let cp = Arc::new(ControlPlane::new(Arc::new(SessionManager::new())));
    let resp = cp
        .handle(
            "test",
            Action::Channel(ChannelAction::Status { id: None }),
        )
        .await;
    assert_eq!(resp["status"], "unavailable");
}

/// P13 unification invariant: the control plane's dreaming orchestrator must be
/// the **same `Arc`** the scheduled dream cycle holds, wrapping the **same live
/// memory store** the agent writes to. If either identity breaks, manual (IPC)
/// and automatic consolidation drift apart — separate feedback tallies and
/// separate pending-FSRS queues, so one path's flush never covers the other's.
#[tokio::test]
async fn dreaming_orchestrator_shares_the_control_plane_memory_store() {
    let memory = Arc::new(nanna_memory::MemoryService::new(
        nanna_memory::MemoryServiceConfig::default(),
    ));
    let dreaming = Arc::new(nanna_memory::DreamingService::with_shared_memory(
        nanna_memory::DreamingConfig::default(),
        Arc::clone(&memory),
    ));

    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.memory = Some(Arc::clone(&memory));
    cp.set_dreaming(Arc::clone(&dreaming));

    let attached = cp.dreaming.clone().expect("dreaming must be attached");
    assert!(
        Arc::ptr_eq(&attached, &dreaming),
        "control plane must hold the scheduler's orchestrator, not a copy"
    );
    assert!(
        Arc::ptr_eq(&attached.memory_arc(), &memory),
        "the orchestrator must dream over the control plane's live store"
    );
}

/// The boot-only-registration regression (2026-07-31): a credential added
/// through the control plane's config path must register its provider on the
/// LIVE router — the GUI saves a key and calls config-reload expecting exactly
/// this, but the provider map used to be frozen at daemon startup, so every
/// call to the new provider died with "No provider available" until a restart.
///
/// Uses `ConfigAction::Set` rather than `Reload` so the test never touches the
/// real on-disk config; both actions run the same rebuild helper. The config
/// credential wins before any keyring/CLI fallback, so the assertions are
/// deterministic on any machine.
#[tokio::test]
async fn config_set_rebuilds_llm_router_providers() {
    let router = Arc::new(crate::llm_router::LlmRouter::new());
    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.router = Some(Arc::clone(&router));
    let cp = Arc::new(cp);

    assert!(
        !router.has_provider(crate::llm_router::ProviderId::OpenRouter),
        "fresh router must start with no providers"
    );

    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "llm.openrouter_api_key".into(),
                value: json!("sk-or-test"),
            }),
        )
        .await;
    assert_eq!(resp["status"], "updated");

    assert!(
        router.has_provider(crate::llm_router::ProviderId::OpenRouter),
        "config mutation must register the new provider on the live router"
    );
    assert!(
        router.has_provider(crate::llm_router::ProviderId::Ollama),
        "Ollama registers unconditionally on every rebuild"
    );

    // /status must report the daemon-side provider list the GUI picker gates on.
    let status = cp
        .handle("test", Action::System(SystemAction::Status))
        .await;
    let providers: Vec<&str> = status["llm_providers"]
        .as_array()
        .expect("status must carry llm_providers")
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(providers.contains(&"openrouter"));
    assert!(providers.contains(&"ollama"));
}

/// The embedding router is built once, at boot. A token saved in Settings used
/// to reach chat (whose router is rebuilt) and never the embedder: against a
/// server that wants the token, every memory embed kept getting 401 and was
/// stored without a vector until a restart — with nothing saying one was
/// needed. Through the daemon's own wiring, the saved token must go out on the
/// very next embed, even though the old token's 401 had benched the provider.
///
/// The same saves change the embedding model, which must NOT follow: the
/// store's vectors are bound to the model the router was built with, and a
/// silent swap is the dimension split-brain.
///
/// `ConfigAction::Set`, not `Reload`, so the test never touches the real
/// on-disk config; every mutation path shares the propagation step.
#[tokio::test]
async fn a_saved_ollama_token_reaches_the_running_embedder_but_the_model_waits() {
    let (base, seen) = crate::embedding_reload::test_ollama::spawn_token_gated("new-token").await;
    let data_dir = tempfile::tempdir().expect("tempdir");
    let server = crate::server::DaemonServer::new(
        crate::server::DaemonConfig {
            data_dir: data_dir.path().to_path_buf(),
            ..crate::server::DaemonConfig::default()
        },
        crate::server::EmbeddingConfig {
            provider: "ollama".into(),
            model: "boot-embed".into(),
            ollama_host: base.clone(),
            ollama_api_key: Some("old-token".into()),
            priority: Vec::new(),
        },
        None,
        None,
    );
    let (info, client) = server
        .embedding_provider_for("ollama/boot-embed")
        .expect("an Ollama spec always resolves");
    let router = crate::embedding_router::EmbeddingRouter::new(info, client);
    router
        .embed_one_now("before")
        .await
        .expect_err("the server refuses the token the daemon booted with");

    let cp = ControlPlane::new(Arc::new(SessionManager::new()))
        .with_live_embedding(server.live_embedding());
    {
        // The running config names the server and model the daemon booted with.
        let mut config = cp.config.write().await;
        config.memory.ollama_host.clone_from(&base);
        config.memory.embedding_provider = "ollama".into();
        config.memory.embedding_model = "boot-embed".into();
        config.memory.embedding_priority.clear();
    }
    let cp = Arc::new(cp);
    for (path, value) in [
        ("memory.embedding_model", json!("other-embed")),
        ("llm.ollama_api_key", json!("new-token")),
    ] {
        let resp = cp
            .handle(
                "test",
                Action::Config(ConfigAction::Set {
                    path: path.into(),
                    value,
                }),
            )
            .await;
        assert_eq!(resp["status"], "updated", "{path}: {resp}");
    }

    let (vector, switched) = router
        .embed_one_now("after")
        .await
        .expect("the saved token opens the server on the next embed");
    assert_eq!(vector.len(), 3);
    assert!(
        switched.is_none(),
        "the one provider stays the one provider"
    );
    let last = seen
        .lock()
        .expect("record lock")
        .last()
        .cloned()
        .expect("the embed reached the server");
    assert_eq!(last.authorization.as_deref(), Some("Bearer new-token"));
    assert_eq!(
        last.model.as_deref(),
        Some("boot-embed"),
        "the model the store is bound to, not the one just saved"
    );
}


/// The Ollama bearer token is bound to the server it was saved for — and a
/// token saved by an older build records no server. `config.set` of a new
/// address (the agent can send one) used to re-read the store for the new
/// address, find no record, and hand the old server's token to the new one:
/// chat, embeddings and the probe would all have sent it there.
#[tokio::test]
async fn config_set_does_not_hand_a_legacy_ollama_token_to_a_new_address() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = nanna_config::SecureStore::file_only_at(dir.path().to_path_buf());
    store
        .set(
            nanna_config::credentials::keys::OLLAMA_API_KEY,
            "legacy-token",
        )
        .expect("set");
    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.credential_store = store.clone();
    // Persisting, as the daemon does: the save files what a change brings in,
    // and must not file the old server's token for the new address either.
    cp.config_path = Some(dir.path().join("config.toml"));
    {
        // As the boot load left it: the legacy token, for the configured server.
        let mut config = cp.config.write().await;
        config.memory.ollama_host = "https://gpu.example/ollama".to_string();
        config.llm.ollama_api_key = Some("legacy-token".to_string());
    }
    let cp = Arc::new(cp);

    let set_host = |host: &str| {
        Action::Config(ConfigAction::Set {
            path: "memory.ollama_host".into(),
            value: json!(host),
        })
    };
    let resp = cp
        .handle("test", set_host("https://elsewhere.example"))
        .await;
    assert_eq!(resp["status"], "updated");
    {
        let config = cp.config.read().await;
        assert_eq!(config.memory.ollama_host, "https://elsewhere.example");
        assert_ne!(
            config.llm.ollama_api_key.as_deref(),
            Some("legacy-token"),
            "the new address must not get the token that was going to the old one"
        );
    }
    assert_eq!(
        store
            .ollama_token_host()
            .expect("a file store answers")
            .as_deref(),
        Some("https://gpu.example/ollama"),
        "the token is recorded as the old server's"
    );

    // Back to the server it was going to: it goes there again.
    let resp = cp
        .handle("test", set_host("https://gpu.example/ollama/"))
        .await;
    assert_eq!(resp["status"], "updated");
    assert_eq!(
        cp.config.read().await.llm.ollama_api_key.as_deref(),
        Some("legacy-token")
    );
}

/// `config.reset` and `config.import` replace the whole config, the address
/// with it. A token saved with no server recorded was the replaced address's;
/// unrecorded, the next reload would read it as the new address's.
#[tokio::test]
async fn reset_and_import_record_a_legacy_ollama_tokens_server_first() {
    use nanna_config::credentials::keys;
    for action in [
        ConfigAction::Reset { path: None },
        ConfigAction::Import {
            config: serde_json::to_value(Config::default()).expect("json"),
        },
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = nanna_config::SecureStore::file_only_at(dir.path().to_path_buf());
        store
            .set(keys::OLLAMA_API_KEY, "legacy-token")
            .expect("set");
        let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
        cp.credential_store = store.clone();
        cp.config_path = Some(dir.path().join("config.toml"));
        {
            let mut config = cp.config.write().await;
            config.memory.ollama_host = "https://gpu.example/ollama".to_string();
            config.llm.ollama_api_key = Some("legacy-token".to_string());
        }
        let cp = Arc::new(cp);
        let label = format!("{action:?}");
        cp.handle("test", Action::Config(action)).await;

        let config = cp.config.read().await;
        assert_eq!(
            config.memory.ollama_host, "http://localhost:11434",
            "{label}"
        );
        assert_ne!(
            config.llm.ollama_api_key.as_deref(),
            Some("legacy-token"),
            "{label}: the default address must not get the old server's token"
        );
        assert_eq!(
            store.get(keys::OLLAMA_API_KEY_HOST).ok().as_deref(),
            Some("https://gpu.example/ollama"),
            "{label}: recorded as the replaced server's"
        );
    }
}

/// A control plane that persists as the daemon's does — a `config.toml` and a
/// secure store of its own under `dir`, never the OS keyring.
fn persisting_control_plane(dir: &std::path::Path) -> (ControlPlane, nanna_config::SecureStore) {
    let store = nanna_config::SecureStore::file_only_at(dir.join("store"));
    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.config_path = Some(dir.join("config.toml"));
    cp.credential_store = store.clone();
    (cp, store)
}

/// The saved `config.toml`, or nothing when no save was made.
fn saved_config(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("config.toml")).unwrap_or_default()
}

/// `config.toml` never holds a secret: the save strips every one, and the
/// secure store is their only durable home. A secret written by `config.set`
/// was never filed there, so it worked until the next load — a restart, or
/// the config watcher re-reading the daemon's own save — and was gone.
#[tokio::test]
async fn a_secret_set_is_filed_before_the_save_strips_it() {
    use nanna_config::credentials::keys;
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    let cp = Arc::new(cp);

    for (path, key, secret) in [
        (
            "llm.api_key",
            keys::ANTHROPIC_API_KEY,
            "sk-ant-set-by-config-set",
        ),
        (
            "llm.openrouter_api_key",
            keys::OPENROUTER_API_KEY,
            "sk-or-set-by-config-set",
        ),
        (
            "tools.brave_api_key",
            keys::BRAVE_API_KEY,
            "brave-set-by-config-set",
        ),
        (
            "server.webhook_secret",
            keys::SERVER_WEBHOOK_SECRET,
            "webhook-set-by-config-set",
        ),
    ] {
        let resp = cp
            .handle(
                "test",
                Action::Config(ConfigAction::Set {
                    path: path.into(),
                    value: json!(secret),
                }),
            )
            .await;
        assert_eq!(resp["status"], "updated", "{path}: {resp}");
        assert_eq!(
            store.get(key).ok().as_deref(),
            Some(secret),
            "{path}: filed"
        );
        let running = serde_json::to_value(&*cp.config.read().await).expect("json");
        let held = path
            .split('.')
            .try_fold(&running, |node, part| node.get(part))
            .and_then(Value::as_str);
        assert_eq!(
            held,
            Some(secret),
            "{path}: still held by the running config"
        );
        let saved = saved_config(dir.path());
        assert!(!saved.is_empty(), "{path}: the change was saved");
        assert!(
            !saved.contains(secret),
            "{path}: not written to config.toml"
        );
    }

    // A channel's secrets arrive with its section.
    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "channels.telegram".into(),
                value: json!({
                    "bot_token": "telegram-bot-set-by-config-set",
                    "webhook_secret": "telegram-webhook-set-by-config-set",
                }),
            }),
        )
        .await;
    assert_eq!(resp["status"], "updated", "{resp}");
    for (key, secret) in [
        (keys::TELEGRAM_BOT_TOKEN, "telegram-bot-set-by-config-set"),
        (
            keys::TELEGRAM_WEBHOOK_SECRET,
            "telegram-webhook-set-by-config-set",
        ),
    ] {
        assert_eq!(store.get(key).ok().as_deref(), Some(secret), "{key}: filed");
        assert!(
            !saved_config(dir.path()).contains(secret),
            "{key}: not written to config.toml"
        );
    }
    let telegram = cp
        .config
        .read()
        .await
        .channels
        .telegram
        .clone()
        .expect("the section is set");
    assert_eq!(telegram.bot_token, "telegram-bot-set-by-config-set");
    assert_eq!(
        telegram.webhook_secret.as_deref(),
        Some("telegram-webhook-set-by-config-set")
    );
}

/// The Ollama token is filed for the server it is set for. The store holds
/// one token, bound to one server; the token set here is the same string the
/// store holds for another. Filed as "already stored", it stayed bound to the
/// other server, and the next load withheld it from this one.
#[tokio::test]
async fn an_ollama_token_set_is_filed_for_the_running_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    store
        .save_ollama_token("shared-token", "https://a.example/ollama")
        .expect("save");
    cp.config.write().await.memory.ollama_host = "https://b.example/ollama".to_string();
    let cp = Arc::new(cp);

    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "llm.ollama_api_key".into(),
                value: json!("shared-token"),
            }),
        )
        .await;

    assert_eq!(resp["status"], "updated", "{resp}");
    assert_eq!(store.ollama_token().as_deref(), Some("shared-token"));
    assert_eq!(
        store
            .ollama_token_host()
            .expect("a file store answers")
            .as_deref(),
        Some("https://b.example/ollama"),
        "filed for the server it was set for"
    );
    assert_eq!(
        cp.config.read().await.llm.ollama_api_key.as_deref(),
        Some("shared-token")
    );
    assert!(!saved_config(dir.path()).contains("shared-token"));
}

/// `config.import` replaces the whole config, secrets included: each one it
/// carries is filed, the Ollama token for the server the import names.
/// (Secrets `with_env_overrides` would replace are left out, so the
/// environment the test runs in cannot change what it sees.)
#[tokio::test]
async fn an_import_files_the_secrets_it_carries() {
    use nanna_config::credentials::keys;
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    store
        .save_ollama_token("old-token", "https://a.example/ollama")
        .expect("save");
    {
        let mut config = cp.config.write().await;
        config.memory.ollama_host = "https://a.example/ollama".to_string();
        config.llm.ollama_api_key = Some("old-token".to_string());
    }
    let cp = Arc::new(cp);

    let mut imported = Config::default();
    imported.memory.ollama_host = "https://b.example/ollama".to_string();
    imported.llm.ollama_api_key = Some("imported-token".to_string());
    imported.llm.openrouter_api_key = Some("sk-or-imported".to_string());
    imported.tools.brave_api_key = Some("brave-imported".to_string());
    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Import {
                config: serde_json::to_value(&imported).expect("json"),
            }),
        )
        .await;

    assert_eq!(resp["status"], "imported", "{resp}");
    assert_eq!(
        store.get(keys::OPENROUTER_API_KEY).ok().as_deref(),
        Some("sk-or-imported")
    );
    assert_eq!(
        store.get(keys::BRAVE_API_KEY).ok().as_deref(),
        Some("brave-imported")
    );
    assert_eq!(store.ollama_token().as_deref(), Some("imported-token"));
    assert_eq!(
        store
            .ollama_token_host()
            .expect("a file store answers")
            .as_deref(),
        Some("https://b.example/ollama")
    );
    let config = cp.config.read().await;
    assert_eq!(
        config.llm.openrouter_api_key.as_deref(),
        Some("sk-or-imported")
    );
    assert_eq!(
        config.tools.brave_api_key.as_deref(),
        Some("brave-imported")
    );
    assert_eq!(config.llm.ollama_api_key.as_deref(), Some("imported-token"));
    let saved = saved_config(dir.path());
    for secret in ["sk-or-imported", "brave-imported", "imported-token"] {
        assert!(
            !saved.contains(secret),
            "{secret} not written to config.toml"
        );
    }
}

/// Only a change that brings a secret in writes the store. A running config
/// holds secrets that are not the store's to keep — one from the environment
/// here — and a change of something else (a Settings toggle, a slider) must
/// not copy them into the keyring, or touch it at all.
#[tokio::test]
async fn a_change_that_brings_in_no_secret_leaves_the_store_alone() {
    use nanna_config::credentials::keys;
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    cp.config.write().await.llm.api_key = Some("sk-ant-from-the-environment".to_string());
    let cp = Arc::new(cp);

    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "llm.model".into(),
                value: json!("nanna-test-other-model"),
            }),
        )
        .await;

    assert_eq!(resp["status"], "updated", "{resp}");
    assert!(
        !store.exists(keys::ANTHROPIC_API_KEY),
        "a secret the change did not bring in is not filed"
    );
    assert_eq!(
        cp.config.read().await.llm.api_key.as_deref(),
        Some("sk-ant-from-the-environment")
    );
}

/// A secret the store cannot file would live until the next load and then be
/// gone. The change is refused instead, with why: nothing is applied or saved.
#[tokio::test]
async fn a_secret_the_store_cannot_file_is_refused_with_its_change() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The store's directory would sit under a plain file: every write fails.
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, "not a directory").expect("write");
    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.config_path = Some(dir.path().join("config.toml"));
    cp.credential_store = nanna_config::SecureStore::file_only_at(blocker.join("store"));
    let cp = Arc::new(cp);

    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "llm.openrouter_api_key".into(),
                value: json!("sk-or-unfilable"),
            }),
        )
        .await;
    assert_eq!(resp["error"], "secret_store_failed", "{resp}");
    assert_eq!(resp["path"], "llm.openrouter_api_key", "{resp}");
    assert_eq!(cp.config.read().await.llm.openrouter_api_key, None);

    let mut imported = Config::default();
    imported.tools.brave_api_key = Some("brave-unfilable".to_string());
    imported.llm.model = "nanna-test-imported-model".to_string();
    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Import {
                config: serde_json::to_value(&imported).expect("json"),
            }),
        )
        .await;
    assert_eq!(resp["error"], "secret_store_failed", "{resp}");
    let config = cp.config.read().await;
    assert_eq!(config.tools.brave_api_key, None);
    assert_ne!(
        config.llm.model, "nanna-test-imported-model",
        "nothing is applied"
    );
    drop(config);
    assert_eq!(saved_config(dir.path()), "", "nothing is saved");
}

/// Negative space: with no memory configured at all, consolidation reports the
/// missing store rather than reaching the dreaming gate.
#[tokio::test]
async fn consolidate_without_memory_reports_unavailable() {
    let cp = Arc::new(ControlPlane::new(Arc::new(SessionManager::new())));
    let resp = cp
        .handle("test", Action::Memory(MemoryAction::Consolidate))
        .await;
    assert_eq!(resp["error"], "memory_unavailable");
}

/// A memory store with **no** orchestrator attached must NOT be a hard fault:
/// the handler falls back to the low-level `MemoryService::consolidate`
/// (minimal constructions keep working), so the first missing precondition it
/// reports is the absent LLM — exactly as in the fully-wired case.
#[tokio::test]
async fn consolidate_without_dreaming_falls_back_and_stops_at_the_llm() {
    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.memory = Some(Arc::new(nanna_memory::MemoryService::new(
        nanna_memory::MemoryServiceConfig::default(),
    )));
    let cp = Arc::new(cp);
    // No router either — the fallback must carry consolidation past the
    // (absent) orchestrator to the LLM precondition, never report a fault.
    let resp = cp
        .handle("test", Action::Memory(MemoryAction::Consolidate))
        .await;
    assert_ne!(
        resp["error"], "dreaming_unavailable",
        "a missing orchestrator must fall back, not fail"
    );
    assert_eq!(resp["error"], "llm_unavailable");
}

/// With the orchestrator attached, consolidation gets **past** the dreaming
/// gate and fails on the (absent) LLM instead. This is the hermetic proof that
/// the IPC path now runs through `DreamingService` — no live model required.
#[tokio::test]
async fn consolidate_with_dreaming_passes_the_gate_and_stops_at_the_llm() {
    let memory = Arc::new(nanna_memory::MemoryService::new(
        nanna_memory::MemoryServiceConfig::default(),
    ));
    let dreaming = Arc::new(nanna_memory::DreamingService::with_shared_memory(
        nanna_memory::DreamingConfig::default(),
        Arc::clone(&memory),
    ));

    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.memory = Some(memory);
    cp.set_dreaming(dreaming);
    let cp = Arc::new(cp);

    let resp = cp
        .handle("test", Action::Memory(MemoryAction::Consolidate))
        .await;
    assert_ne!(
        resp["error"], "dreaming_unavailable",
        "an attached orchestrator must not report a wiring fault"
    );
    assert_eq!(
        resp["error"], "llm_unavailable",
        "with dreaming wired, the next precondition is the summarizer model"
    );
}

#[tokio::test]
async fn enable_disable_reconciles_live_registry() {
    use crate::user_tools::UserToolManager;
    use nanna_tools::ToolRegistry;

    let tmp = tempfile::tempdir().expect("tempdir");
    let registry = Arc::new(ToolRegistry::new());
    let user_tools = Arc::new(UserToolManager::new(tmp.path().to_path_buf()));

    let source =
        "export default { name: \"t_demo\", description: \"demo\", execute(p) { return \"ok\"; } }";
    user_tools
        .create_tool("t_demo".into(), "demo".into(), source.into(), None, None, None)
        .await
        .expect("create tool");
    user_tools.register_with_registry(&registry).await;
    assert!(registry.get("t_demo").await.is_some(), "tool should start registered");

    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.tools = Some(registry.clone());
    cp.user_tools = Some(user_tools.clone());

    // Disable → the tool is dropped from the live registry (stops executing).
    let resp = cp.set_user_tool_enabled("t_demo", false).await;
    assert_eq!(resp["status"], "disabled");
    assert!(
        registry.get("t_demo").await.is_none(),
        "disabled tool must not remain callable"
    );

    // Re-enable → the tool becomes callable again without a restart.
    let resp = cp.set_user_tool_enabled("t_demo", true).await;
    assert_eq!(resp["status"], "enabled");
    assert!(
        registry.get("t_demo").await.is_some(),
        "re-enabled tool must be registered again"
    );
}

// ---------------------------------------------------------------------------
// Universal per-tool toggle (bundled skills, not just user tools)
// ---------------------------------------------------------------------------

/// A stand-in for a bundled skill: registered in the registry, with no entry in
/// the user-tool store. That combination is the whole point — it is the class
/// the toggle used to be unable to touch.
struct DemoBundledTool;

#[async_trait::async_trait]
impl nanna_tools::Tool for DemoBundledTool {
    fn definition(&self) -> nanna_tools::ToolDefinition {
        nanna_tools::ToolDefinition {
            name: "demo_builtin".to_string(),
            description: "a bundled demo tool".to_string(),
            parameters: Vec::new(),
            output_schema: None,
        }
    }

    async fn execute(
        &self,
        _params: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<nanna_tools::ToolResult, nanna_tools::ToolError> {
        Ok(nanna_tools::ToolResult::success("ok"))
    }
}

/// The defect this closes: `Enable`/`Disable` routed to the user-tool store, so
/// for a bundled skill it answered `update_failed` and changed nothing. The 44
/// shipped skills could not be turned off from any client.
#[tokio::test]
async fn a_bundled_tool_can_be_toggled_and_the_gate_really_stops_it() {
    use nanna_tools::{ToolCall, ToolRegistry};

    let registry = Arc::new(ToolRegistry::new());
    registry.register(DemoBundledTool).await;

    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.tools = Some(registry.clone());
    // No user-tool manager: this tool has no per-tool store, which is exactly
    // the case the old path could not serve.

    let call = || ToolCall {
        id: "c".to_string(),
        name: "demo_builtin".to_string(),
        parameters: std::collections::HashMap::new(),
    };
    assert!(
        registry.execute(call()).await.result.success,
        "the tool runs before anything is toggled"
    );

    let resp = cp.set_tool_enabled("demo_builtin", false).await;
    assert_eq!(resp["status"], "disabled");
    assert_eq!(resp["name"], "demo_builtin");

    // The verdict has to be the gate refusing, not merely a flag somewhere.
    assert!(
        !registry.execute(call()).await.result.success,
        "a disabled tool must actually stop executing"
    );

    let resp = cp.set_tool_enabled("demo_builtin", true).await;
    assert_eq!(resp["status"], "enabled");
    assert!(
        registry.execute(call()).await.result.success,
        "re-enabling must restore it without a restart"
    );
}

/// Disabling must not hide the tool from the listing that turns it back on.
/// `definitions()` drops a denied tool on purpose, so a `List` built on it made
/// disabling a one-way door: the tool vanished and no client could reach it.
#[tokio::test]
async fn a_disabled_bundled_tool_is_still_listed_so_it_can_be_re_enabled() {
    use nanna_tools::ToolRegistry;

    let registry = Arc::new(ToolRegistry::new());
    registry.register(DemoBundledTool).await;

    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.tools = Some(registry.clone());
    let cp = Arc::new(cp);

    cp.set_tool_enabled("demo_builtin", false).await;

    let resp = cp.handle("test", Action::Tool(ToolAction::List)).await;
    let listed = resp["tools"]
        .as_array()
        .expect("list must return an array")
        .iter()
        .find(|t| t["name"] == "demo_builtin")
        .expect("a disabled tool must remain listed, or nothing can re-enable it");
    assert_eq!(
        listed["enabled"], false,
        "and it must be reported AS disabled, not silently claimed enabled"
    );

    // Its detail view must survive being switched off too.
    let resp = cp
        .handle(
            "test",
            Action::Tool(ToolAction::Get {
                name: "demo_builtin".to_string(),
            }),
        )
        .await;
    assert_eq!(
        resp["tool"]["name"], "demo_builtin",
        "a disabled tool must still be inspectable"
    );
    assert_eq!(resp["tool"]["enabled"], false);
}

/// The alias lesson the audit trail already paid for, now on the toggle path.
/// `resolve_tool` returns the registry KEY, which for an alias is the alias
/// itself — so writing that into the denylist would produce a toggle that
/// reports success while the gate, which speaks canonical names, keeps letting
/// the tool run.
#[tokio::test]
async fn toggling_an_alias_disables_the_tool_it_points_at() {
    use nanna_tools::{ToolCall, ToolRegistry};

    let registry = Arc::new(ToolRegistry::new());
    registry.register(DemoBundledTool).await;
    registry.register_alias("DemoAlias", "demo_builtin").await;

    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.tools = Some(registry.clone());

    let resp = cp.set_tool_enabled("DemoAlias", false).await;
    assert_eq!(
        resp["name"], "demo_builtin",
        "the toggle must report the canonical identity it actually acted on"
    );

    let blocked = registry
        .execute(ToolCall {
            id: "c".to_string(),
            name: "demo_builtin".to_string(),
            parameters: std::collections::HashMap::new(),
        })
        .await;
    assert!(
        !blocked.result.success,
        "disabling via an alias must stop the canonical tool, not a name nothing checks"
    );
}

/// A name no tool answers to must be refused, not silently written into the
/// denylist where it would sit forever gating nothing.
#[tokio::test]
async fn toggling_an_unknown_tool_is_refused() {
    use nanna_tools::ToolRegistry;

    let registry = Arc::new(ToolRegistry::new());
    registry.register(DemoBundledTool).await;

    let mut cp = ControlPlane::new(Arc::new(SessionManager::new()));
    cp.tools = Some(registry.clone());

    let resp = cp
        .set_tool_enabled("zzzz_no_such_tool_at_all", false)
        .await;
    assert_eq!(resp["error"], "not_found");
}

// ---------------------------------------------------------------------------
// ChatRunRegistry admission gate (P22 Tier 4)
// ---------------------------------------------------------------------------

/// With nothing live, background work is admitted immediately — the gate
/// must never add latency to an idle system.
#[tokio::test]
async fn gate_wait_idle_returns_immediately_when_nothing_runs() {
    let registry = chat_harness::ChatRunRegistry::new();
    tokio::time::timeout(std::time::Duration::from_secs(1), registry.wait_idle())
        .await
        .expect("an idle registry admits background work without waiting");
}

/// The admission edge: background work parks while a run is live and is
/// released the moment the LAST run releases — priority, not a quota.
#[tokio::test]
async fn gate_wait_idle_parks_until_the_last_run_releases() {
    let registry = Arc::new(chat_harness::ChatRunRegistry::new());
    assert!(registry.try_claim("turn-a").await);
    assert!(registry.try_claim("turn-b").await);

    let gate = registry.clone();
    let waiter = tokio::spawn(async move { gate.wait_idle().await });

    // Still parked while ANY run is live.
    registry.release("turn-a").await;
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!waiter.is_finished(), "one live run must keep the gate shut");

    registry.release("turn-b").await;
    tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .expect("the last release must wake the waiter")
        .expect("waiter must not panic");
}

/// The preemption edge: a parked watcher fires the moment a run claims.
#[tokio::test]
async fn gate_wait_active_fires_on_claim() {
    let registry = Arc::new(chat_harness::ChatRunRegistry::new());

    let gate = registry.clone();
    let watcher = tokio::spawn(async move { gate.wait_active().await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!watcher.is_finished(), "an idle registry must keep the preemption watcher parked");

    assert!(registry.try_claim("user-turn").await);
    tokio::time::timeout(std::time::Duration::from_secs(1), watcher)
        .await
        .expect("a claim must wake the preemption watcher")
        .expect("watcher must not panic");
}

/// Wakeup-loss stress: edges fired in a tight loop while waiters park and
/// re-park. Interest is registered before the condition check (`enable`), so
/// no ordering of claim/release against a parking waiter may strand it.
#[tokio::test(flavor = "multi_thread")]
async fn gate_edges_are_never_lost_under_racing_claims() {
    for _ in 0..100 {
        let registry = Arc::new(chat_harness::ChatRunRegistry::new());
        assert!(registry.try_claim("racer").await);
        let gate = registry.clone();
        let waiter = tokio::spawn(async move { gate.wait_idle().await });
        // Release immediately — sometimes before the waiter first parks,
        // sometimes after; both orderings must wake it.
        registry.release("racer").await;
        tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
            .await
            .expect("no interleaving of claim/release may strand a waiter")
            .expect("waiter must not panic");
    }
}

/// A job may name a conversation for its results — and only a real one: a
/// result posted into a missing session would be dropped on every run.
#[tokio::test]
async fn a_scheduled_job_can_post_into_an_existing_conversation_only() {
    let sessions = Arc::new(SessionManager::new());
    let session = sessions.create(None).await;
    let scheduler = Arc::new(RwLock::new(Scheduler::new(
        nanna_core::SchedulerConfig::default(),
    )));
    let cp = Arc::new(ControlPlane::new(sessions).with_scheduler(Arc::clone(&scheduler)));
    let add = |session_id: Option<String>| {
        Action::Scheduler(SchedulerAction::Add {
            schedule: "0 8 * * *".into(),
            task: "Summarize my inbox".into(),
            name: Some("inbox".into()),
            session_id,
        })
    };

    let refused = cp.handle("test", add(Some("no-such-chat".into()))).await;
    assert_eq!(refused["error"], "session_not_found", "{refused}");
    assert!(
        scheduler.read().await.list_tasks().await.is_empty(),
        "nothing was added"
    );

    let created = cp.handle("test", add(Some(session.id.clone()))).await;
    let id = created["id"].as_str().expect("created").to_string();
    let job = cp
        .handle("test", Action::Scheduler(SchedulerAction::Get { id }))
        .await;
    assert_eq!(job["job"]["target_session"], session.id.as_str(), "{job}");

    let unrouted = cp.handle("test", add(None)).await;
    assert_eq!(unrouted["status"], "created");
}

/// `[llm].ollama_url` is retired: summaries go through chat's router and its
/// one Ollama server. A `set` of it used to answer `updated` while serde
/// dropped the key on the round trip, so a script that moved its summarizer
/// that way would never learn it no longer does.
#[tokio::test]
async fn setting_a_retired_key_is_refused_with_what_replaced_it() {
    let cp = Arc::new(ControlPlane::new(Arc::new(SessionManager::new())));
    let before = cp.config.read().await.clone();

    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "llm.ollama_url".into(),
                value: json!("http://gpu.example:11434"),
            }),
        )
        .await;

    assert_eq!(resp["error"], "retired_key", "{resp}");
    assert_eq!(resp["path"], "llm.ollama_url", "{resp}");
    let message = resp["message"].as_str().unwrap_or_default();
    assert!(message.contains("memory.ollama_host"), "{message}");
    assert_eq!(
        cp.config.read().await.memory.ollama_host,
        before.memory.ollama_host,
        "nothing else moves"
    );
}

/// The same key inside an object set at its parent is refused the same way:
/// a `set` of `llm` carrying `ollama_url` used to answer `updated` and drop
/// the key, and the rest of the object with it went through unnoticed.
#[tokio::test]
async fn a_retired_key_inside_a_parent_object_is_refused_too() {
    let cp = Arc::new(ControlPlane::new(Arc::new(SessionManager::new())));
    let before = cp.config.read().await.llm.model.clone();

    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "llm".into(),
                value: json!({
                    "model": "nanna-test-other-model",
                    "ollama_url": "http://gpu.example:11434",
                }),
            }),
        )
        .await;

    assert_eq!(resp["error"], "retired_key", "{resp}");
    assert_eq!(resp["path"], "llm", "{resp}");
    let message = resp["message"].as_str().unwrap_or_default();
    assert!(
        message.contains("ollama_url") && message.contains("memory.ollama_host"),
        "{message}"
    );
    assert_eq!(cp.config.read().await.llm.model, before, "nothing is applied");

    // The same object without the retired key is an ordinary set.
    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "llm".into(),
                value: json!({ "model": "nanna-test-other-model" }),
            }),
        )
        .await;
    assert_eq!(resp["status"], "updated", "{resp}");
    assert_eq!(cp.config.read().await.llm.model, "nanna-test-other-model");
}

// -----------------------------------------------------------------------------
// Clearing a secret, and a change that merely lacks one
// -----------------------------------------------------------------------------

/// Boot `cp` as the daemon boots: `config` is its saved `config.toml`, and
/// the running config is what loading that file gives, with `store`'s
/// secrets and the environment's.
async fn boot_from(cp: &ControlPlane, dir: &std::path::Path, config: &Config) {
    let file = dir.join("config.toml");
    config.save_to(&file).expect("save");
    let loaded =
        Config::load_from_replacing(&file, &config.memory.ollama_host, &cp.credential_store)
            .expect("loads")
            .with_env_overrides();
    *cp.config.write().await = loaded;
}

/// Assert that the running config is what the next load of the save gives:
/// a restart, or the config watcher reading the daemon's own save back
/// within one poll (loaded here as the watcher loads it). Where the two
/// differ, the watcher applies the load, and what the change did is undone
/// seconds later.
async fn assert_running_is_the_next_load(cp: &ControlPlane, dir: &std::path::Path, label: &str) {
    let running = cp.config.read().await.clone();
    let loaded = Config::load_from_replacing(
        &dir.join("config.toml"),
        &running.memory.ollama_host,
        &cp.credential_store,
    )
    .expect("the save loads")
    .with_env_overrides();
    assert_eq!(
        serde_json::to_value(&running).expect("json"),
        serde_json::to_value(&loaded).expect("json"),
        "{label}: the running config is what the next load gives"
    );
}

/// What the environment supplies for `var`: a blank variable supplies nothing.
fn env_supplies(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

/// The string at dotted `path` in `config`, when there is one.
fn string_at(config: &Config, path: &str) -> Option<String> {
    let value = serde_json::to_value(config).expect("json");
    path.split('.')
        .try_fold(&value, |node, part| node.get(part))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

/// `config.set` of a secret's path to null or blank is someone clearing it.
/// Cleared in the running config only, the store kept it, and the next load
/// (a restart, or the watcher reading the save back) brought it back. It is
/// deleted from the store, and the running config holds what the next load
/// gives: only what the environment still supplies.
#[tokio::test]
async fn clearing_a_secret_deletes_it_from_the_store() {
    use nanna_config::credentials::keys;
    for (path, key, env_var, clear) in [
        (
            "llm.api_key",
            keys::ANTHROPIC_API_KEY,
            "ANTHROPIC_API_KEY",
            json!(null),
        ),
        (
            "llm.openrouter_api_key",
            keys::OPENROUTER_API_KEY,
            "OPENROUTER_API_KEY",
            json!(""),
        ),
        (
            "tools.brave_api_key",
            keys::BRAVE_API_KEY,
            "BRAVE_API_KEY",
            json!(null),
        ),
        (
            "server.webhook_secret",
            keys::SERVER_WEBHOOK_SECRET,
            "NANNA_WEBHOOK_SECRET",
            json!("  "),
        ),
        (
            "channels.telegram.bot_token",
            keys::TELEGRAM_BOT_TOKEN,
            "TELEGRAM_BOT_TOKEN",
            json!(""),
        ),
    ] {
        let dir = tempfile::tempdir().expect("tempdir");
        let (cp, store) = persisting_control_plane(dir.path());
        store.set(key, "stored-secret").expect("set");
        let mut config = Config::default();
        config.channels.telegram = Some(nanna_config::TelegramConfig {
            bot_token: String::new(),
            webhook_url: None,
            allowed_users: Some(vec![42]),
            webhook_secret: None,
        });
        boot_from(&cp, dir.path(), &config).await;
        let cp = Arc::new(cp);

        let resp = cp
            .handle(
                "test",
                Action::Config(ConfigAction::Set {
                    path: path.into(),
                    value: clear,
                }),
            )
            .await;

        assert_eq!(resp["status"], "updated", "{path}: {resp}");
        assert!(!store.exists(key), "{path}: deleted from the store");
        assert_eq!(
            string_at(&*cp.config.read().await, path).filter(|held| !held.is_empty()),
            env_supplies(env_var),
            "{path}: the running config holds only what the environment supplies"
        );
        assert_running_is_the_next_load(&cp, dir.path(), path).await;
    }
}

/// The store holds one Ollama token, bound to the server it was saved for.
/// Clearing the token clears the running server's: the stored one goes when
/// it is that server's, or recorded for none (an older build's, loaded for
/// the running server). Another server's token is not this one's to clear.
#[tokio::test]
async fn clearing_the_ollama_token_deletes_only_the_running_servers() {
    use nanna_config::credentials::keys;
    const RUNNING: &str = "https://gpu.example/ollama";
    let clear = || {
        Action::Config(ConfigAction::Set {
            path: "llm.ollama_api_key".into(),
            value: Value::Null,
        })
    };
    let mut running_config = Config::default();
    running_config.memory.ollama_host = RUNNING.to_string();

    // Filed for the running server.
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    store.save_ollama_token("gpu-token", RUNNING).expect("save");
    boot_from(&cp, dir.path(), &running_config).await;
    let cp = Arc::new(cp);
    let resp = cp.handle("test", clear()).await;
    assert_eq!(resp["status"], "updated", "{resp}");
    assert_eq!(
        store.ollama_token(),
        None,
        "the running server's token is deleted"
    );
    assert!(
        !store.exists(keys::OLLAMA_API_KEY_HOST),
        "and its server's record"
    );
    assert_eq!(
        cp.config.read().await.llm.ollama_api_key,
        env_supplies("OLLAMA_API_KEY")
    );
    assert_running_is_the_next_load(&cp, dir.path(), "the running server's").await;

    // Filed for another server: kept, still bound to it.
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    store
        .save_ollama_token("other-token", "https://other.example/ollama")
        .expect("save");
    boot_from(&cp, dir.path(), &running_config).await;
    let cp = Arc::new(cp);
    let resp = cp.handle("test", clear()).await;
    assert_eq!(resp["status"], "updated", "{resp}");
    assert_eq!(store.ollama_token().as_deref(), Some("other-token"));
    assert_eq!(
        store
            .ollama_token_host()
            .expect("a file store answers")
            .as_deref(),
        Some("https://other.example/ollama"),
        "another server's token is kept for it"
    );
    assert_running_is_the_next_load(&cp, dir.path(), "another server's").await;

    // Filed by an older build, with no server recorded: the running server's.
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    store
        .set(keys::OLLAMA_API_KEY, "legacy-token")
        .expect("set");
    running_config
        .save_to(&dir.path().join("config.toml"))
        .expect("save");
    {
        let mut config = cp.config.write().await;
        *config = running_config.clone();
        config.llm.ollama_api_key = Some("legacy-token".to_string());
    }
    let cp = Arc::new(cp);
    let resp = cp.handle("test", clear()).await;
    assert_eq!(resp["status"], "updated", "{resp}");
    assert_eq!(
        store.ollama_token(),
        None,
        "an unbound token was the running server's"
    );
    assert_running_is_the_next_load(&cp, dir.path(), "an unbound token").await;
}

/// The secrets a reset keeps: every one the store holds. Reset put the
/// defaults in the running config, credentials included, so chat went
/// without the stored key until the watcher read the save back and the
/// store's secrets came back with it. The running config after a reset is
/// what a restart loads, and the store loses nothing.
#[tokio::test]
async fn a_reset_keeps_the_stored_credentials() {
    use nanna_config::credentials::keys;
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    let stored = [
        (
            keys::OPENROUTER_API_KEY,
            "OPENROUTER_API_KEY",
            "llm.openrouter_api_key",
            "sk-or-stored",
        ),
        (
            keys::BRAVE_API_KEY,
            "BRAVE_API_KEY",
            "tools.brave_api_key",
            "brave-stored",
        ),
        (
            keys::SERVER_WEBHOOK_SECRET,
            "NANNA_WEBHOOK_SECRET",
            "server.webhook_secret",
            "webhook-stored",
        ),
    ];
    for (key, _, _, secret) in stored {
        store.set(key, secret).expect("set");
    }
    let default_host = Config::default().memory.ollama_host;
    store
        .save_ollama_token("ollama-stored", &default_host)
        .expect("save");
    let mut config = Config::default();
    config.llm.model = "nanna-test-configured-model".to_string();
    boot_from(&cp, dir.path(), &config).await;
    let cp = Arc::new(cp);

    let resp = cp
        .handle("test", Action::Config(ConfigAction::Reset { path: None }))
        .await;

    assert_eq!(resp["status"], "reset", "{resp}");
    let running = cp.config.read().await.clone();
    assert_eq!(running.llm.model, Config::default().llm.model, "reset");
    for (key, env_var, path, secret) in stored {
        assert_eq!(store.get(key).ok().as_deref(), Some(secret), "{path}: kept");
        assert_eq!(
            string_at(&running, path),
            Some(env_supplies(env_var).unwrap_or_else(|| secret.to_string())),
            "{path}: still held by the running config"
        );
    }
    assert_eq!(store.ollama_token().as_deref(), Some("ollama-stored"));
    assert_eq!(
        running.llm.ollama_api_key,
        Some(env_supplies("OLLAMA_API_KEY").unwrap_or_else(|| "ollama-stored".to_string()))
    );
    assert_running_is_the_next_load(&cp, dir.path(), "reset").await;
}

/// An import replaces the whole config, but an export may carry no secrets
/// (a copy that never held them, one written before they left
/// `config.toml`): a secret the import lacks is not one it clears. The store
/// keeps it, and the running config holds it as the next load will.
#[tokio::test]
async fn an_import_that_lacks_a_secret_keeps_it() {
    use nanna_config::credentials::keys;
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    store
        .set(keys::OPENROUTER_API_KEY, "sk-or-stored")
        .expect("set");
    store.set(keys::BRAVE_API_KEY, "brave-stored").expect("set");
    boot_from(&cp, dir.path(), &Config::default()).await;
    let cp = Arc::new(cp);

    let mut imported = Config::default();
    imported.llm.model = "nanna-test-imported-model".to_string();
    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Import {
                config: serde_json::to_value(&imported).expect("json"),
            }),
        )
        .await;

    assert_eq!(resp["status"], "imported", "{resp}");
    let running = cp.config.read().await.clone();
    assert_eq!(running.llm.model, "nanna-test-imported-model");
    for (key, env_var, secret, held) in [
        (
            keys::OPENROUTER_API_KEY,
            "OPENROUTER_API_KEY",
            "sk-or-stored",
            running.llm.openrouter_api_key.clone(),
        ),
        (
            keys::BRAVE_API_KEY,
            "BRAVE_API_KEY",
            "brave-stored",
            running.tools.brave_api_key.clone(),
        ),
    ] {
        assert_eq!(store.get(key).ok().as_deref(), Some(secret), "{key}: kept");
        assert_eq!(
            held,
            Some(env_supplies(env_var).unwrap_or_else(|| secret.to_string())),
            "{key}: still held by the running config"
        );
    }
    assert_running_is_the_next_load(&cp, dir.path(), "import").await;
}

/// A `config.set` of a whole section that lacks a secret is not a clear of
/// it, any more than an import is: only a set of the secret's own path to
/// null or blank clears it.
#[tokio::test]
async fn a_set_of_a_section_that_lacks_a_secret_keeps_it() {
    use nanna_config::credentials::keys;
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    store
        .set(keys::OPENROUTER_API_KEY, "sk-or-stored")
        .expect("set");
    boot_from(&cp, dir.path(), &Config::default()).await;
    let cp = Arc::new(cp);

    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "llm".into(),
                value: json!({ "model": "nanna-test-other-model", "openrouter_api_key": null }),
            }),
        )
        .await;

    assert_eq!(resp["status"], "updated", "{resp}");
    assert_eq!(
        store.get(keys::OPENROUTER_API_KEY).ok().as_deref(),
        Some("sk-or-stored"),
        "kept in the store"
    );
    assert_eq!(
        cp.config.read().await.llm.openrouter_api_key,
        Some(env_supplies("OPENROUTER_API_KEY").unwrap_or_else(|| "sk-or-stored".to_string()))
    );
    assert_running_is_the_next_load(&cp, dir.path(), "section set").await;
}

/// A store that cannot delete the secret refuses the clear, with why:
/// applied, it would last until the next load and the secret would be back.
/// Nothing is applied or saved.
#[tokio::test]
async fn a_clear_the_store_cannot_make_is_refused() {
    use nanna_config::credentials::keys;
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, store) = persisting_control_plane(dir.path());
    store.set(keys::BRAVE_API_KEY, "brave-stored").expect("set");
    boot_from(&cp, dir.path(), &Config::default()).await;
    let held = cp.config.read().await.tools.brave_api_key.clone();
    let saved = saved_config(dir.path());
    // The store's file no longer decrypts: every read and write of it fails.
    std::fs::write(
        dir.path().join("store").join("credentials.enc"),
        b"not a store",
    )
    .expect("write");
    let cp = Arc::new(cp);

    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "tools.brave_api_key".into(),
                value: Value::Null,
            }),
        )
        .await;

    assert_eq!(resp["error"], "secret_store_failed", "{resp}");
    assert_eq!(resp["path"], "tools.brave_api_key", "{resp}");
    assert_eq!(
        cp.config.read().await.tools.brave_api_key,
        held,
        "nothing is applied"
    );
    assert_eq!(saved_config(dir.path()), saved, "nothing is saved");
}

/// Negative space: a set of null on a path that is no secret is no clear —
/// the GUI sends `llm.sub_agent_model: null` with every sub-agent list — and
/// never touches the store (this one cannot be read at all).
#[tokio::test]
async fn a_null_set_of_no_secret_leaves_the_store_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (cp, _store) = persisting_control_plane(dir.path());
    std::fs::create_dir_all(dir.path().join("store")).expect("mkdir");
    std::fs::write(
        dir.path().join("store").join("credentials.enc"),
        b"not a store",
    )
    .expect("write");
    cp.config.write().await.llm.sub_agent_model = Some("nanna-test-sub-agent".to_string());
    let cp = Arc::new(cp);

    let resp = cp
        .handle(
            "test",
            Action::Config(ConfigAction::Set {
                path: "llm.sub_agent_model".into(),
                value: Value::Null,
            }),
        )
        .await;

    assert_eq!(resp["status"], "updated", "{resp}");
    assert_eq!(cp.config.read().await.llm.sub_agent_model, None);
}
