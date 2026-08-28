//! Behavioral tests for the `file_buffer` default skill, executed for real
//! through the Boa engine with a bridge scoped to a temp directory.
//!
//! The buffer is the answer to a 9B model's hardest constraint: one huge,
//! perfect tool argument per file. Chunks are appended across calls (each
//! response echoes the tail so the model continues in the right place) and
//! the real file changes only on commit. In this harness `Nanna.exec` is
//! unavailable, so the commit-time Python gate fails OPEN and cleanup falls
//! back to truncating the buffer — both by design.
//!
//! Tolerant by design (same as `default_skills_params.rs`): if the sibling
//! `nanna-tools/default-skills` tree isn't present, the tests no-op.

#![cfg(feature = "boa")]

mod common;

use nanna_scripting::{ScriptEngine, ScriptedTool, ToolPermissions};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../nanna-tools/default-skills/file_buffer/tool.ts")
}

/// The bridge workdir is pinned to the temp dir so the skill's own state
/// files (`.nanna/write_hiwater.json`, `.nanna/read_marks.json`) resolve
/// inside the fixture instead of the developer's home directory — reachable,
/// writable, and thrown away with the tempdir. Without it every run starts
/// with no read mark and the ratchet never persists, which is neither what
/// production does nor what these tests mean to exercise.
async fn run_buffer(input: Value, dir: &Path) -> Result<Value, String> {
    let tool = ScriptedTool::from_file(skill_path())
        .expect("read file_buffer tool.ts")
        .with_permissions(ToolPermissions::none().with_read([dir]).with_write([dir]))
        // Scaffolding, not an assertion — see `common::FIXTURE_TIMEOUT_MS`.
        .with_timeout(common::FIXTURE_TIMEOUT_MS);
    ScriptEngine::new()
        .execute_with_workdir(&tool, input, None, None, Some(dir.to_path_buf()))
        .await
        .map(|r| r.value)
        .map_err(|e| e.to_string())
}

/// Milliseconds since the epoch of a file's mtime, the unit read marks use.
fn mtime_ms(path: &Path) -> u64 {
    let modified = std::fs::metadata(path).expect("stat").modified().expect("mtime");
    u64::try_from(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .expect("mtime after epoch")
            .as_millis(),
    )
    .expect("mtime fits u64")
}

/// Seed `.nanna/read_marks.json` directly. With the workdir pinned above, the
/// skill's canonical key for a file inside the fixture is just its name.
fn seed_read_mark(dir: &Path, key: &str, at_ms: u64) {
    let state = dir.join(".nanna");
    std::fs::create_dir_all(&state).expect("mkdir .nanna");
    std::fs::write(
        state.join("read_marks.json"),
        json!({ key: { "at": at_ms } }).to_string(),
    )
    .expect("write read_marks.json");
}

async fn run_ok(input: Value, dir: &Path) -> String {
    let result = run_buffer(input, dir).await.expect("tool must not throw");
    assert_eq!(
        result["success"],
        Value::Bool(true),
        "expected success, got: {result}"
    );
    result["content"].as_str().expect("content").to_string()
}

async fn run_fail(input: Value, dir: &Path) -> String {
    let result = run_buffer(input, dir).await.expect("failures are returned, not thrown");
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
async fn append_append_commit_builds_the_file() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("big.txt").to_string_lossy().into_owned();

    let first = run_ok(
        json!({ "action": "append", "file_path": target, "content": "line one\nline two" }),
        dir.path(),
    )
    .await;
    // The response echoes the buffer tail so the model continues in place.
    assert!(first.contains("ends with"), "got: {first}");
    assert!(first.contains("line two"), "got: {first}");

    run_ok(
        json!({ "action": "append", "file_path": target, "content": "line three\n" }),
        dir.path(),
    )
    .await;

    let committed = run_ok(
        json!({ "action": "commit", "file_path": target }),
        dir.path(),
    )
    .await;
    assert!(committed.contains("Committed"), "got: {committed}");

    // Auto-newline joins chunks; the real file is the concatenation.
    let content = std::fs::read_to_string(dir.path().join("big.txt")).unwrap();
    assert_eq!(content, "line one\nline two\nline three\n");

    // Buffer cleared (exec unavailable here → truncated fallback).
    let buf = dir.path().join("big.txt.__buffer__");
    assert!(
        !buf.exists() || std::fs::read_to_string(&buf).unwrap().is_empty(),
        "buffer must be cleared after commit"
    );
}

#[tokio::test]
async fn content_without_action_is_treated_as_append() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("b.txt").to_string_lossy().into_owned();

    let out = run_ok(
        json!({ "file_path": target, "content": "hello" }),
        dir.path(),
    )
    .await;
    assert!(out.contains("Buffered"), "got: {out}");
}

#[tokio::test]
async fn commit_with_empty_buffer_teaches_the_sequence() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("c.txt").to_string_lossy().into_owned();

    let err = run_fail(
        json!({ "action": "commit", "file_path": target }),
        dir.path(),
    )
    .await;
    assert!(err.contains("nothing is buffered"), "got: {err}");
    assert!(err.contains("append"), "got: {err}");
    assert!(!dir.path().join("c.txt").exists(), "real file must not appear");
}

#[tokio::test]
async fn clear_discards_without_touching_the_real_file() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("d.txt");
    std::fs::write(&real, "precious").unwrap();
    let target = real.to_string_lossy().into_owned();

    run_ok(
        json!({ "action": "append", "file_path": target, "content": "draft" }),
        dir.path(),
    )
    .await;
    let cleared = run_ok(
        json!({ "action": "clear", "file_path": target }),
        dir.path(),
    )
    .await;
    assert!(cleared.contains("discarded"), "got: {cleared}");
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "precious");
}

#[tokio::test]
async fn second_clear_requires_force() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("h.txt").to_string_lossy().into_owned();

    // First discard is free (and warns about the next one) — but must NOT
    // advertise the force escape hatch: round 12 showed the model reads
    // "pass force=true" in refusals and makes forced bypass its habit.
    run_ok(
        json!({ "action": "append", "file_path": target, "content": "draft one" }),
        dir.path(),
    )
    .await;
    let first = run_ok(json!({ "action": "clear", "file_path": target }), dir.path()).await;
    assert!(first.contains("repair"), "got: {first}");
    assert!(!first.contains("force"), "must not advertise force: {first}");

    // Second discard is the regeneration loop — refused, no hatch named.
    run_ok(
        json!({ "action": "append", "file_path": target, "content": "draft two" }),
        dir.path(),
    )
    .await;
    let err = run_fail(json!({ "action": "clear", "file_path": target }), dir.path()).await;
    assert!(err.contains("CLEAR REFUSED"), "got: {err}");
    assert!(err.contains("edit_file"), "got: {err}");
    assert!(!err.contains("force"), "must not advertise force: {err}");

    // force=true still WORKS (an operator escape), it is just unnamed.
    let forced = run_ok(
        json!({ "action": "clear", "file_path": target, "force": true }),
        dir.path(),
    )
    .await;
    assert!(forced.contains("discarded"), "got: {forced}");
}

#[tokio::test]
async fn shrink_guard_keeps_buffer_and_file() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("e.txt");
    let original = "x".repeat(1_000);
    std::fs::write(&real, &original).unwrap();
    let target = real.to_string_lossy().into_owned();
    // The session has read the file as it stands now, so the stale-shrink
    // hold has nothing to say and the 30% floor is what answers.
    seed_read_mark(dir.path(), "e.txt", mtime_ms(&real));

    run_ok(
        json!({ "action": "append", "file_path": target, "content": "tiny" }),
        dir.path(),
    )
    .await;
    let err = run_fail(
        json!({ "action": "commit", "file_path": target }),
        dir.path(),
    )
    .await;
    assert!(err.contains("COMMIT REFUSED"), "got: {err}");
    assert!(err.contains("KEPT"), "got: {err}");
    assert!(err.contains("appending"), "got: {err}");
    assert!(!err.contains("force"), "must not advertise force: {err}");
    // Real file untouched, buffer still there for continued appending.
    assert_eq!(std::fs::read_to_string(&real).unwrap(), original);
    let buf = std::fs::read_to_string(dir.path().join("e.txt.__buffer__")).unwrap();
    assert_eq!(buf, "tiny");
}

/// P22 Tier 3 stale-shrink hold, NEVER-READ branch: no read mark exists at
/// all. The hold is right — you cannot shrink what you have not seen — but
/// the reason it states has to be the true one. These replies are read
/// literally by 9B-class local models, and "the file has CHANGED since you
/// last read it" sends one hunting for a change that never happened.
#[tokio::test]
async fn stale_shrink_hold_echoes_the_current_file() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("notes.txt");
    let original = "alpha\nbeta\ngamma\n".repeat(50);
    std::fs::write(&real, &original).unwrap();
    let target = real.to_string_lossy().into_owned();

    // Well clear of the 30% floor, so the hold is the only thing in play.
    let merged = "alpha\nbeta\n".repeat(50);
    run_ok(
        json!({ "action": "append", "file_path": target.clone(), "content": merged.clone() }),
        dir.path(),
    )
    .await;
    let err = run_fail(
        json!({ "action": "commit", "file_path": target.clone() }),
        dir.path(),
    )
    .await;

    assert!(err.contains("COMMIT HELD"), "got: {err}");
    assert!(
        err.contains("NEVER read this file in this session"),
        "must name the real reason: {err}"
    );
    assert!(
        !err.contains("CHANGED since you last read it"),
        "must not claim a change that never happened: {err}"
    );
    assert!(err.contains("gamma"), "the echo must carry the current content: {err}");
    assert!(
        err.contains("counts as your read"),
        "must say the bounce is the read: {err}"
    );
    // Real file untouched, buffer kept for the retry.
    assert_eq!(std::fs::read_to_string(&real).unwrap(), original);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notes.txt.__buffer__")).unwrap(),
        merged
    );

    // The echo counted as the read, so the same commit lands on the retry.
    run_ok(json!({ "action": "commit", "file_path": target }), dir.path()).await;
    assert_eq!(std::fs::read_to_string(&real).unwrap(), merged);
}

/// The other branch of the same hold: a read mark EXISTS but predates the
/// file's mtime, so the file genuinely moved under the model. Here the
/// "CHANGED since you last read it" sentence is the true one.
#[tokio::test]
async fn stale_shrink_hold_names_the_change_when_the_file_moved_under_us() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("notes.txt");
    let original = "alpha\nbeta\ngamma\n".repeat(50);
    std::fs::write(&real, &original).unwrap();
    let target = real.to_string_lossy().into_owned();
    // Read ten minutes before the file was last written: seen, then changed.
    seed_read_mark(dir.path(), "notes.txt", mtime_ms(&real) - 600_000);

    run_ok(
        json!({ "action": "append", "file_path": target.clone(), "content": "alpha\nbeta\n".repeat(50) }),
        dir.path(),
    )
    .await;
    let err = run_fail(
        json!({ "action": "commit", "file_path": target }),
        dir.path(),
    )
    .await;

    assert!(err.contains("COMMIT HELD"), "got: {err}");
    assert!(
        err.contains("CHANGED since you last read it"),
        "must name the real reason: {err}"
    );
    assert!(
        !err.contains("NEVER read this file"),
        "the session did read it: {err}"
    );
    assert!(err.contains("gamma"), "the echo must carry the current content: {err}");
    assert_eq!(std::fs::read_to_string(&real).unwrap(), original);
}

#[tokio::test]
async fn show_previews_the_pending_tail() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("f.txt").to_string_lossy().into_owned();

    run_ok(
        json!({ "action": "append", "file_path": target, "content": "alpha\nbeta\ngamma" }),
        dir.path(),
    )
    .await;
    let shown = run_ok(json!({ "action": "show", "file_path": target }), dir.path()).await;
    assert!(shown.contains("3 lines"), "got: {shown}");
    assert!(shown.contains("gamma"), "got: {shown}");
}

#[tokio::test]
async fn unknown_action_and_missing_params_fail_instructively() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("g.txt").to_string_lossy().into_owned();

    let err = run_fail(
        json!({ "action": "explode", "file_path": target }),
        dir.path(),
    )
    .await;
    assert!(err.contains("unknown action"), "got: {err}");
    assert!(err.contains("append"), "got: {err}");

    let err = run_fail(json!({ "action": "append", "file_path": target }), dir.path()).await;
    assert!(err.contains("content"), "got: {err}");

    let err = run_fail(json!({ "action": "append", "content": "x" }), dir.path()).await;
    assert!(err.contains("file_path"), "got: {err}");
}
