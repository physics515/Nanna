//! Behavioral tests for the `write_file` default skill (v0.1.4), executed
//! for real through the Boa engine. Covers the structured guidance
//! failures, the versioned-copy-name refusal, and the shrink guard. The
//! Python syntax gate needs `Nanna.exec`, which this harness does not
//! grant — that path fails OPEN here by design and is exercised live.
//!
//! Tolerant by design: if the sibling default-skills tree isn't present,
//! the tests no-op.

#![cfg(feature = "boa")]

use nanna_scripting::{ScriptEngine, ScriptedTool, ToolPermissions};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../nanna-tools/default-skills/write_file/tool.ts")
}

async fn run_write(input: Value, dir: &Path) -> Result<Value, String> {
    let tool = ScriptedTool::from_file(skill_path())
        .expect("read write_file tool.ts")
        .with_permissions(ToolPermissions::none().with_read([dir]).with_write([dir]));
    ScriptEngine::new()
        .execute(&tool, input, None, None)
        .await
        .map(|r| r.value)
        .map_err(|e| e.to_string())
}

async fn run_fail(input: Value, dir: &Path) -> String {
    let result = run_write(input, dir).await.expect("failures are returned, not thrown");
    assert_eq!(
        result["success"],
        Value::Bool(false),
        "expected failure, got: {result}"
    );
    result["content"].as_str().expect("content").to_string()
}

fn skill_missing() -> bool {
    if skill_path().is_file() {
        false
    } else {
        eprintln!("skipping: {} not present", skill_path().display());
        true
    }
}

#[tokio::test]
async fn writes_and_reports() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.txt").to_string_lossy().into_owned();

    let result = run_write(
        json!({ "file_path": target, "content": "hello" }),
        dir.path(),
    )
    .await
    .expect("write should succeed");
    let content = result["content"].as_str().expect("content");
    assert!(content.contains("Wrote 5 bytes"), "got: {content}");
    assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "hello");
}

#[tokio::test]
async fn missing_content_is_a_structured_failure() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("b.txt").to_string_lossy().into_owned();

    // Round-6 live log showed this reaching the model as a thrown error
    // under five "Execution failed:" prefixes — it must be structured now.
    let err = run_fail(json!({ "file_path": target }), dir.path()).await;
    assert!(err.contains("missing content"), "got: {err}");
    assert!(err.contains("Nothing was written"), "got: {err}");
}

#[tokio::test]
async fn versioned_copy_names_are_refused() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();

    for name in [
        "new_notekeeper.py",
        "notekeeper_v2.py",
        "script_fixed.txt",
        "runner.py.new",
        "notes_backup.py",
    ] {
        let target = dir.path().join(name).to_string_lossy().into_owned();
        let err = run_fail(
            json!({ "file_path": target, "content": "print('hi')\n" }),
            dir.path(),
        )
        .await;
        assert!(err.contains("WRITE REFUSED"), "{name}: {err}");
        assert!(err.contains("versioned copy"), "{name}: {err}");
        assert!(!err.contains("force"), "must not advertise force: {err}");
        assert!(!dir.path().join(name).exists(), "{name} must not be created");
    }

    // force=true is the escape hatch for genuinely new standalone files.
    let target = dir.path().join("new_module.py").to_string_lossy().into_owned();
    let result = run_write(
        json!({ "file_path": target, "content": "print('hi')\n", "force": true }),
        dir.path(),
    )
    .await
    .expect("forced write succeeds");
    assert_eq!(result["success"], Value::Null, "write_file success result has no success:false");
    assert!(dir.path().join("new_module.py").exists());
}

#[tokio::test]
async fn valid_rewrite_over_a_parked_draft_is_accepted() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("a.py").to_string_lossy().into_owned();
    std::fs::write(dir.path().join("a.py.__buffer__"), "draft = (").unwrap();

    // Round-13 lesson: the old rail bounced even VALID regenerations while
    // a draft was parked — throwing away the model's one reliable move.
    // Valid content always wins now. (In this harness the syntax checker
    // fails open, which lands on the same accept path.)
    let result = run_write(
        json!({ "file_path": target, "content": "print('regenerated')\n" }),
        dir.path(),
    )
    .await
    .expect("valid rewrite must be accepted");
    let content = result["content"].as_str().expect("content");
    assert!(content.contains("Wrote"), "got: {content}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.py")).unwrap(),
        "print('regenerated')\n"
    );
}

#[tokio::test]
async fn shrink_guard_still_refuses_fragments() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("big.txt");
    std::fs::write(&real, "y".repeat(1_000)).unwrap();
    let target = real.to_string_lossy().into_owned();

    let err = run_fail(
        json!({ "file_path": target, "content": "fragment" }),
        dir.path(),
    )
    .await;
    assert!(err.contains("WRITE REFUSED"), "got: {err}");
    assert!(err.contains("NOT modified"), "got: {err}");
    assert_eq!(std::fs::read_to_string(&real).unwrap().len(), 1_000);
}
