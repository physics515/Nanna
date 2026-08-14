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
    assert!(all["channels"].as_array().unwrap().len() >= 1);
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
    let cp = ControlPlane::new(Arc::new(SessionManager::new()));
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

/// Negative space: with no memory configured at all, consolidation reports the
/// missing store rather than reaching the dreaming gate.
#[tokio::test]
async fn consolidate_without_memory_reports_unavailable() {
    let cp = ControlPlane::new(Arc::new(SessionManager::new()));
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
