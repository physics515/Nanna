//! An agent can author a tool and call it in the same run.
//!
//! The three authoring skills (`create_tool`, `edit_tool`, `list_user_tools`)
//! were withheld at every boot because `tools.create` / `tools.update` /
//! `tools.list` were registered nowhere. A compile cannot tell you whether a
//! service is wired — the skill loads either way and fails at call time — so
//! this drives the real services against a real registry and then *calls the
//! tool they produced*, which is the only thing that distinguishes "written to
//! disk" from "callable right now", the claim `create_tool` makes to the model.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use nanna_daemon::tool_authoring::build_tool_authoring_services;
use nanna_scripting::ServiceFn;
use nanna_tools::{ToolCall, ToolRegistry};
use serde_json::{Value, json};

/// A complete module, the shape `create_tool` assembles before calling the
/// service. Returns a fixed string so the round trip can assert on output
/// rather than on the absence of an error.
fn probe_source(reply: &str) -> String {
    format!(
        r#"export default {{
  name: "probe_tool",
  description: "a probe authored through tools.create",
  parameters: {{ "type": "object", "properties": {{}}, "required": [] }},
  execute: function(input) {{ return "{reply}"; }}
}};
"#
    )
}

struct Harness {
    registry: Arc<ToolRegistry>,
    services: HashMap<String, ServiceFn>,
    tools_dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        let tools_dir = tempfile::tempdir().expect("temp tools dir");
        let registry = Arc::new(ToolRegistry::new());
        let slot: Arc<OnceLock<HashMap<String, ServiceFn>>> = Arc::new(OnceLock::new());
        let services = build_tool_authoring_services(
            tools_dir.path().to_path_buf(),
            Arc::downgrade(&registry),
            Arc::clone(&slot),
        );
        // Same ordering the daemon uses: build, then fill the slot so an
        // authored tool is loaded with the finished map.
        slot.set(services.clone()).ok();
        Self {
            registry,
            services,
            tools_dir,
        }
    }

    async fn call(&self, service: &str, params: Value) -> Result<Value, String> {
        let f = self
            .services
            .get(service)
            .unwrap_or_else(|| panic!("{service} must be registered"));
        f(params).await
    }

    async fn run_tool(&self, name: &str) -> String {
        let response = self
            .registry
            .execute(ToolCall {
                id: "probe-call".to_string(),
                name: name.to_string(),
                parameters: HashMap::new(),
            })
            .await;
        assert!(
            response.result.success,
            "calling the authored tool failed: {:?}",
            response.result.error
        );
        response.result.content
    }
}

#[tokio::test]
async fn a_tool_authored_through_the_service_is_callable_without_a_restart() {
    let h = Harness::new();

    let created = h
        .call(
            "tools.create",
            json!({
                "name": "probe_tool",
                "description": "a probe authored through tools.create",
                "source": probe_source("first"),
            }),
        )
        .await
        .expect("tools.create succeeds");

    assert_eq!(
        created["registered"], true,
        "the tool was written but not registered, so `create_tool` would have \
         told the model to restart: {created}"
    );
    let path = created["path"]
        .as_str()
        .expect("the service reports a path");
    assert!(
        std::path::Path::new(path).is_file(),
        "reported path does not exist: {path}"
    );
    assert!(
        path.starts_with(h.tools_dir.path().to_string_lossy().as_ref()),
        "the tool was written outside the tools directory: {path}"
    );

    // The claim under test: callable right now.
    assert_eq!(h.run_tool("probe_tool").await, "first");

    // And it carries a scope somebody chose, rather than waiting for
    // `ensure_permissions` to fill one in at the next boot.
    let permissions = std::fs::read_to_string(
        h.tools_dir
            .path()
            .join("probe_tool")
            .join("permissions.json"),
    )
    .expect("an authored tool ships permissions.json");
    let parsed: Value = serde_json::from_str(&permissions).expect("permissions parse");
    assert!(
        parsed["read"]
            .as_array()
            .is_some_and(|scopes| { scopes.iter().all(|scope| scope.as_str() != Some("*")) }),
        "an agent-authored tool was granted the whole filesystem: {permissions}"
    );
}

#[tokio::test]
async fn creating_over_an_existing_tool_is_refused_rather_than_clobbering_it() {
    let h = Harness::new();
    h.call(
        "tools.create",
        json!({ "name": "probe_tool", "description": "d", "source": probe_source("first") }),
    )
    .await
    .expect("the first create succeeds");

    let err = h
        .call(
            "tools.create",
            json!({ "name": "probe_tool", "description": "d", "source": probe_source("second") }),
        )
        .await
        .expect_err("creating over an existing tool must be refused");
    assert!(err.contains("already exists"), "unhelpful refusal: {err}");

    // The original survived the refusal.
    assert_eq!(h.run_tool("probe_tool").await, "first");
}

#[tokio::test]
async fn an_edit_changes_the_tool_and_the_next_call_sees_it() {
    let h = Harness::new();
    h.call(
        "tools.create",
        json!({ "name": "probe_tool", "description": "d", "source": probe_source("first") }),
    )
    .await
    .expect("create succeeds");
    assert_eq!(h.run_tool("probe_tool").await, "first");

    let updated = h
        .call(
            "tools.update",
            json!({
                "name": "probe_tool",
                "old_string": "\"first\"",
                "new_string": "\"second\"",
            }),
        )
        .await
        .expect("tools.update succeeds");
    assert_eq!(updated["replacements"], 1);
    assert_eq!(updated["registered"], true);

    // Re-registration is the half a file write cannot prove.
    assert_eq!(
        h.run_tool("probe_tool").await,
        "second",
        "the edit reached disk but the live tool still runs the old source"
    );
}

#[tokio::test]
async fn an_edit_that_would_break_the_tool_is_refused_before_disk() {
    let h = Harness::new();
    h.call(
        "tools.create",
        json!({ "name": "probe_tool", "description": "d", "source": probe_source("first") }),
    )
    .await
    .expect("create succeeds");

    let err = h
        .call(
            "tools.update",
            json!({ "name": "probe_tool", "source": "const broken = 1;" }),
        )
        .await
        .expect_err("a source with no default export must be refused");
    assert!(err.contains("export default"), "unhelpful refusal: {err}");

    // Refused before disk means the tool still works.
    assert_eq!(h.run_tool("probe_tool").await, "first");
}

#[tokio::test]
async fn editing_a_tool_that_does_not_exist_is_refused_rather_than_creating_one() {
    let h = Harness::new();
    let err = h
        .call(
            "tools.update",
            json!({ "name": "absent_tool", "old_string": "a", "new_string": "b" }),
        )
        .await
        .expect_err("editing an absent tool must be refused");
    assert!(err.contains("no tool named"), "unhelpful refusal: {err}");
    assert!(
        !h.tools_dir.path().join("absent_tool").exists(),
        "a refused edit created the tool anyway"
    );
}

#[tokio::test]
async fn a_traversing_name_never_reaches_the_filesystem() {
    let h = Harness::new();
    for name in ["../escape", "a/b", "..", "Upper", "with space"] {
        let outcome = h
            .call(
                "tools.create",
                json!({ "name": name, "description": "d", "source": probe_source("x") }),
            )
            .await;
        let err = match outcome {
            Err(err) => err,
            Ok(created) => panic!("{name:?} was accepted as a tool name: {created}"),
        };
        assert!(
            err.contains("tool name"),
            "{name:?} was refused for the wrong reason: {err}"
        );
    }

    // Nothing was created, inside the tools directory or beside it.
    let entries: Vec<_> = std::fs::read_dir(h.tools_dir.path())
        .expect("the tools directory still exists")
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert!(
        entries.is_empty(),
        "a refused name still put something on disk: {entries:?}"
    );
    assert!(
        !h.tools_dir
            .path()
            .parent()
            .expect("the temp dir has a parent")
            .join("escape")
            .exists(),
        "a traversing name escaped the tools directory"
    );
}

#[tokio::test]
async fn listing_reports_what_was_authored() {
    let h = Harness::new();
    assert_eq!(
        h.call("tools.list", json!({}))
            .await
            .expect("list succeeds"),
        json!([]),
        "a fresh tools directory listed something"
    );

    h.call(
        "tools.create",
        json!({ "name": "probe_tool", "description": "d", "source": probe_source("first") }),
    )
    .await
    .expect("create succeeds");

    let listed = h
        .call("tools.list", json!({}))
        .await
        .expect("list succeeds");
    let entries = listed.as_array().expect("list returns an array");
    assert_eq!(
        entries.len(),
        1,
        "expected exactly the authored tool: {listed}"
    );
    assert_eq!(entries[0]["name"], "probe_tool");
    assert_eq!(
        entries[0]["description"], "a probe authored through tools.create",
        "the description comes from the manifest, not from the create call"
    );
}

/// An authored tool must still be there after a restart.
///
/// The daemon authors into `{data_dir}/tools`, which in a **debug** build is
/// *not* where skills are loaded from — `resolve_tools_dir` returns the source
/// tree there, so writing to it would drop new skill directories into the
/// checkout. Loading from source is deliberate; writing to it is not. The
/// daemon therefore loads the authoring directory as well, and without that a
/// tool would be callable for exactly one session and gone after a restart.
///
/// This drives the same two steps: create through the service, then load the
/// directory the way a fresh boot would.
#[tokio::test]
async fn an_authored_tool_is_loaded_again_by_a_fresh_registry() {
    let h = Harness::new();
    h.call(
        "tools.create",
        json!({ "name": "probe_tool", "description": "d", "source": probe_source("first") }),
    )
    .await
    .expect("create succeeds");

    // A fresh registry, as a restarted daemon would build, loading only the
    // authoring directory.
    let restarted = Arc::new(ToolRegistry::new());
    let loaded = restarted
        .load_skills_with_services(h.tools_dir.path(), &HashMap::new())
        .await;
    assert!(
        loaded >= 1,
        "a restarted daemon loaded nothing from the authoring directory, so an \
         authored tool would survive exactly one session",
    );
    assert!(
        restarted.has("probe_tool").await,
        "the authored tool is missing after a restart",
    );
}
