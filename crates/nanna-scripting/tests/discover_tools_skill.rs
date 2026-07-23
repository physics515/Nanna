//! Behavioral tests for the `discover_tools` default skill, executed for real
//! through the Boa engine.
//!
//! This harness attaches tool definitions but NO tool-search function, so
//! `Nanna.searchTools` returns an empty array — exactly the environment of an
//! older/embedded engine. The skill must detect that and fall back to its
//! local tokenizer path, keeping the result shape (`content` +
//! `data.activate_tools`) identical. The ranked path itself is covered by
//! Rust tests in `nanna-tools::search`.
//!
//! Tolerant by design (same as `edit_file_skill.rs`): if the sibling
//! `nanna-tools/default-skills` tree isn't present, the tests no-op.

#![cfg(feature = "boa")]

use nanna_scripting::{ScriptEngine, ScriptedTool, ToolSearchFn};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../nanna-tools/default-skills/discover_tools/tool.ts")
}

/// Guard: skip (returning true) when the skills tree isn't present.
fn skill_missing() -> bool {
    if skill_path().is_file() {
        false
    } else {
        eprintln!("skipping: {} not present", skill_path().display());
        true
    }
}

/// Tool definitions in the same shape `ScriptedToolWrapper::execute`
/// serializes for `Nanna.listTools()`.
fn defs() -> Value {
    json!([
        {
            "name": "write_file",
            "description": "Write content to a file. BOTH parameters are REQUIRED on every call.",
            "parameters": [
                {"name": "file_path", "type": "string", "required": true, "description": "Path"},
                {"name": "content", "type": "string", "required": true, "description": "Text"}
            ]
        },
        {
            "name": "edit_file",
            "description": "Replace one exact text snippet in a file with new text.",
            "parameters": []
        },
        {
            "name": "exec",
            "description": "Execute a shell command in a POSIX bash shell and return its output.",
            "parameters": []
        },
        {
            "name": "remember",
            "description": "Store a memory (core tool, always active).",
            "parameters": []
        }
    ])
}

/// Execute the real discover_tools tool.ts against `input`, with tool
/// definitions attached but no tool-search fn (the fallback environment).
async fn run_discover(input: Value) -> Result<Value, String> {
    let tool = ScriptedTool::from_file(skill_path()).expect("read discover_tools tool.ts");
    ScriptEngine::new()
        .execute(&tool, input, Some(defs()), None)
        .await
        .map(|r| r.value)
        .map_err(|e| e.to_string())
}

fn activate_tools(result: &Value) -> Vec<String> {
    result["data"]["activate_tools"]
        .as_array()
        .expect("activate_tools array")
        .iter()
        .map(|v| v.as_str().expect("tool name string").to_string())
        .collect()
}

#[tokio::test]
async fn ranked_search_path_orders_results_and_filters_core_tools() {
    if skill_missing() {
        return;
    }
    // Attach a stub ToolSearchFn (the shape `ScriptedToolWrapper` wires from
    // the registry): the skill must use its ranking verbatim — exec before
    // write_file despite listTools order — and still drop core tools that
    // the search surfaced.
    let search: ToolSearchFn = Arc::new(|query: &str, _limit: usize| {
        assert_eq!(query, "run command");
        json!([
            {"name": "exec", "description": "Execute a shell command.", "score": 2.5},
            {"name": "remember", "description": "Core tool.", "score": 1.9},
            {"name": "write_file", "description": "Write content to a file.", "score": 0.4}
        ])
    });

    let tool = ScriptedTool::from_file(skill_path()).expect("read discover_tools tool.ts");
    let result = ScriptEngine::new()
        .execute_full(
            &tool,
            json!({ "query": "run command" }),
            Some(defs()),
            None,
            None,
            None,
            Some(search),
        )
        .await
        .expect("discover_tools should run")
        .value;

    let names = activate_tools(&result);
    assert_eq!(
        names,
        vec!["exec".to_string(), "write_file".to_string()],
        "ranked order must be preserved and core tools dropped"
    );
}

#[tokio::test]
async fn query_falls_back_to_tokenizer_without_search_tools() {
    if skill_missing() {
        return;
    }
    // searchTools exists in the Boa bridge but no search fn is attached, so it
    // returns [] — the skill must fall back to per-term substring matching and
    // still find both file tools for "file write".
    let result = run_discover(json!({ "query": "file write" }))
        .await
        .expect("discover_tools should run");

    let names = activate_tools(&result);
    assert!(
        names.contains(&"write_file".to_string()),
        "fallback must find write_file, got: {names:?}"
    );
    assert!(
        names.contains(&"edit_file".to_string()),
        "fallback must find edit_file, got: {names:?}"
    );
    assert!(
        !names.contains(&"remember".to_string()),
        "core tools must stay filtered out, got: {names:?}"
    );
    assert!(
        result["content"]
            .as_str()
            .unwrap_or("")
            .contains("write_file"),
        "content must describe the found tools"
    );
}

#[tokio::test]
async fn no_query_lists_all_discoverable_tools() {
    if skill_missing() {
        return;
    }
    let result = run_discover(json!({}))
        .await
        .expect("discover_tools should run");

    let names = activate_tools(&result);
    assert_eq!(names.len(), 3, "all non-core tools listed, got: {names:?}");
    assert!(!names.contains(&"remember".to_string()), "got: {names:?}");
}

#[tokio::test]
async fn unmatched_query_returns_no_tools_message() {
    if skill_missing() {
        return;
    }
    let result = run_discover(json!({ "query": "zzqqx" }))
        .await
        .expect("discover_tools should run");

    assert_eq!(activate_tools(&result), Vec::<String>::new());
    assert!(
        result["content"]
            .as_str()
            .unwrap_or("")
            .contains("No tools found"),
        "got: {}",
        result["content"]
    );
}
