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
