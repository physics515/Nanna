//! Behavioral tests for the `explore` default skill, executed for real
//! through the Boa engine with a bridge scoped to a temp directory.
//!
//! The contract under test (regression for the 2026-08-02 live failure where
//! every explore call blew the 30s Boa deadline on a real workspace and the
//! model retry-looped it): the walk is level-by-level and bounded — noise
//! dirs (node_modules, .git, target, ...) are counted but never descended,
//! at most 1000 entries are visited (derived from the ~2KB context output
//! budget), the cap announces itself, and an inaccessible root returns a
//! structured `success: false` result instead of a thrown error.
//!
//! Tolerant by design (same as `default_skills_params.rs`): if the sibling
//! `nanna-tools/default-skills` tree isn't present, the tests no-op.

#![cfg(feature = "boa")]

mod common;

use nanna_scripting::{ScriptEngine, ScriptedTool, ToolPermissions};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../nanna-tools/default-skills/explore/tool.ts")
}

/// Execute the real explore tool.ts against `input`, sandboxed to `dir`.
async fn run_explore(input: Value, dir: &Path) -> Result<Value, String> {
    let tool = ScriptedTool::from_file(skill_path())
        .expect("read explore tool.ts")
        .with_permissions(ToolPermissions::none().with_read([dir]))
        // Scaffolding, not an assertion — see `common::FIXTURE_TIMEOUT_MS`.
        .with_timeout(common::FIXTURE_TIMEOUT_MS);
    ScriptEngine::new()
        .execute(&tool, input, None, None)
        .await
        .map(|r| r.value)
        .map_err(|e| e.to_string())
}

/// Run and expect success; returns the summary content string.
async fn run_explore_ok(input: Value, dir: &Path) -> String {
    let result = run_explore(input, dir)
        .await
        .expect("explore should succeed");
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

/// Guard: skip (returning true) when the skills tree isn't present.
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
async fn summarizes_tree_to_default_depth() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "a.rs", "fn main() {}");
    seed(dir.path(), "b.md", "# readme");
    seed(dir.path(), "sub/c.rs", "mod c;");
    seed(dir.path(), "sub/deep/d.rs", "mod d;"); // depth 3: not visited

    let content = run_explore_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    // dirs: sub + deep; files: a.rs, b.md, c.rs (d.rs is beyond depth 2).
    assert!(content.contains("2 directories, 3 files"), "got: {content}");
    assert!(content.contains(".rs: 2 files"), "got: {content}");
    assert!(content.contains("Top-level:"), "got: {content}");
    assert!(content.contains("sub/"), "got: {content}");
}

#[tokio::test]
async fn depth_1_lists_only_direct_children() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "a.rs", "fn main() {}");
    seed(dir.path(), "sub/c.rs", "mod c;");

    let content = run_explore_ok(
        json!({ "path": dir.path().to_string_lossy(), "depth": 1 }),
        dir.path(),
    )
    .await;

    assert!(content.contains("1 directories, 1 files"), "got: {content}");
}

#[tokio::test]
async fn noise_dirs_are_counted_but_never_descended() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "src/main.rs", "fn main() {}");
    for i in 0..20 {
        seed(dir.path(), &format!("node_modules/pkg/f{i}.js"), "x");
    }

    let content = run_explore_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    // node_modules itself counts as a dir; its 20 .js files must not.
    assert!(content.contains("2 directories, 1 files"), "got: {content}");
    assert!(
        !content.contains(".js"),
        "must not tally skipped content: {content}"
    );
    // The skip must announce itself.
    assert!(content.contains("node_modules"), "got: {content}");
    assert!(content.contains("never descended"), "got: {content}");
}

#[tokio::test]
async fn entry_cap_trips_and_announces_itself() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // 1100 entries in one flat directory: 100 over the 1000-entry budget.
    for i in 0..1100 {
        std::fs::write(dir.path().join(format!("f{i}.log")), "x").expect("seed");
    }

    let content = run_explore_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(
        content.contains("stopped at 1000 entries"),
        "got: {content}"
    );
    assert!(content.contains("partial"), "got: {content}");
    assert!(
        content.contains("path/depth"),
        "must offer narrowing guidance: {content}"
    );
}

#[tokio::test]
async fn missing_path_returns_structured_failure_not_a_throw() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let ghost = dir.path().join("no_such_dir");

    let result = run_explore(json!({ "path": ghost.to_string_lossy() }), dir.path())
        .await
        .expect("inaccessible path must be returned, not thrown");

    assert_eq!(
        result["success"],
        Value::Bool(false),
        "expected success:false, got: {result}"
    );
    let content = result["content"].as_str().expect("content string");
    assert!(content.contains("no_such_dir"), "got: {content}");
    assert!(content.contains("exists"), "got: {content}");
}

#[tokio::test]
async fn empty_directory_is_an_observation_not_an_error() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();

    let content = run_explore_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(content.contains("Empty directory"), "got: {content}");
}

/// Manual timing check against a REAL large tree (the nanna repo root,
/// including node_modules/.git/target). The old recursive-walk version blew
/// the 30s Boa deadline on 100% of calls here; the bounded walk must finish
/// in single-digit seconds. Ignored by default: wall-time on a shared CI box
/// is not a stable assertion.
/// Run: cargo test -p nanna-scripting --test explore_skill -- --ignored --nocapture
#[tokio::test]
#[ignore = "manual timing check against the real repo tree"]
async fn timing_against_real_workspace() {
    if skill_missing() {
        return;
    }
    // No canonicalize(): on Windows it yields a `\\?\` extended-length path,
    // and the tool joins child segments with `/`, which `\\?\` paths reject.
    // EXPLORE_TIMING_PATH overrides the default (this checkout's root) so the
    // check can be pointed at a tree with fully populated node_modules/target.
    let repo_root = std::env::var("EXPLORE_TIMING_PATH").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        PathBuf::from,
    );

    let start = std::time::Instant::now();
    let content = run_explore_ok(json!({ "path": repo_root.to_string_lossy() }), &repo_root).await;
    let elapsed = start.elapsed();

    println!("explore({}) took {elapsed:?}", repo_root.display());
    println!("--- output ({} bytes) ---\n{content}", content.len());
    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "bounded explore must beat the 30s script deadline with margin, took {elapsed:?}"
    );
}
