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
