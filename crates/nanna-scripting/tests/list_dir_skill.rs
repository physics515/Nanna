//! Behavioral tests for the `list_dir` default skill, executed for real
//! through the Boa engine with a bridge scoped to a temp directory.
//!
//! Same hazard class as `explore` (PR #149) and `code_search`: the listing was
//! unbounded, so a directory with tens of thousands of entries — or a
//! `recursive: true` call over a real workspace — marshalled every entry into
//! Boa as its own JS object before the script ran a line. The contract here:
//! the listing is bounded by a budget derived from the memory-chunk cost of
//! the result, truncation announces itself, the recursive walk's silent
//! noise-dir pruning is stated rather than left to be discovered, and an
//! unreadable path returns a structured `success: false` instead of throwing.
//!
//! Tolerant by design: if the sibling `nanna-tools/default-skills` tree isn't
//! present, the tests no-op.

#![cfg(feature = "boa")]

use nanna_scripting::{ScriptEngine, ScriptedTool, ToolPermissions};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nanna-tools/default-skills/list_dir/tool.ts")
}

async fn run_list(input: Value, dir: &Path) -> Result<Value, String> {
    let tool = ScriptedTool::from_file(skill_path())
        .expect("read list_dir tool.ts")
        .with_permissions(ToolPermissions::none().with_read([dir]));
    ScriptEngine::new()
        .execute(&tool, input, None, None)
        .await
        .map(|r| r.value)
        .map_err(|e| e.to_string())
}

async fn run_list_ok(input: Value, dir: &Path) -> String {
    let result = run_list(input, dir).await.expect("list should succeed");
    assert_eq!(
        result["success"],
        Value::Bool(true),
        "expected success:true, got: {result}"
    );
    result["content"]
        .as_str()
        .expect("content string")
        .to_string()
}

fn skill_missing() -> bool {
    if skill_path().is_file() {
        false
    } else {
        eprintln!("skipping: {} not present", skill_path().display());
        true
    }
}

fn seed(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, content).expect("seed file");
}

#[tokio::test]
async fn lists_entries_with_types_and_sizes() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "a.rs", "fn main() {}");
    seed(dir.path(), "sub/b.rs", "mod b;");

    let content = run_list_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(content.contains("[dir]  sub"), "got: {content}");
    assert!(content.contains("a.rs (12B)"), "got: {content}");
    // Non-recursive by default: the child is not listed.
    assert!(!content.contains("b.rs"), "got: {content}");
    // Nothing was bounded, so nothing announces itself.
    assert!(!content.contains("TRUNCATED"), "got: {content}");
}

#[tokio::test]
async fn empty_directory_is_an_observation_not_an_error() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();

    let content = run_list_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(content.contains("Empty directory"), "got: {content}");
}

#[tokio::test]
async fn recursive_listing_announces_the_pruned_dirs() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "src/main.rs", "fn main() {}");
    seed(dir.path(), "node_modules/pkg/vendored.js", "x");

    let content = run_list_ok(
        json!({ "path": dir.path().to_string_lossy(), "recursive": true }),
        dir.path(),
    )
    .await;

    assert!(content.contains("main.rs"), "got: {content}");
    // The bridge prunes noise dirs from the recursive walk. Silent omission
    // reads as "the tree is smaller than it is" — so it is stated.
    assert!(!content.contains("vendored.js"), "got: {content}");
    assert!(
        content.contains("never descended into"),
        "the pruning must announce itself: {content}"
    );
    assert!(content.contains("10 levels deep"), "got: {content}");
}

#[tokio::test]
async fn huge_directory_truncates_and_announces_itself() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // MAX_ENTRIES is floor(32000 / 9) = 3555 — the most entries that could
    // ever fit the 32,000-char (10 memory chunk) output budget. 3600 files
    // pass it, and the char budget then bites well before that.
    for i in 0..3_600 {
        std::fs::write(dir.path().join(format!("f{i}.log")), "x").expect("seed");
    }

    let content = run_list_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(
        content.contains("LISTING TRUNCATED"),
        "got tail: {}",
        tail(&content)
    );
    assert!(
        content.contains("more than 3555 entries"),
        "got tail: {}",
        tail(&content)
    );
    assert!(
        content.contains("SUCCEEDED"),
        "a truncated listing must not read as a failure: {}",
        tail(&content)
    );
    assert!(
        content.contains("List a subdirectory directly"),
        "must offer narrowing guidance: {}",
        tail(&content)
    );
    // The bound is real: the result stays inside the derived char budget.
    assert!(
        content.len() < 40_000,
        "result should be bounded near 32,000 chars, got {}",
        content.len()
    );
}

#[tokio::test]
async fn missing_path_returns_structured_failure_not_a_throw() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let ghost = dir.path().join("no_such_dir");

    let result = run_list(json!({ "path": ghost.to_string_lossy() }), dir.path())
        .await
        .expect("an inaccessible path must be returned, not thrown");

    assert_eq!(
        result["success"],
        Value::Bool(false),
        "expected success:false, got: {result}"
    );
    let content = result["content"].as_str().expect("content string");
    assert!(content.contains("no_such_dir"), "got: {content}");
    assert!(content.contains("exists"), "got: {content}");
}

fn tail(s: &str) -> &str {
    &s[s.len().saturating_sub(700)..]
}
