//! Behavioral tests for the `project_structure` default skill, executed for
//! real through the Boa engine with a bridge scoped to a temp directory.
//!
//! Same hazard class as `explore` (PR #149), `code_search` and `list_dir`: the
//! walk was unbounded. `buildTree` recursed with a per-directory
//! `Nanna.listDir(dirPath, false)` down to `max_depth` with no noise-dir skip
//! list and no entry budget, so a real workspace marshalled every entry under
//! node_modules/.git/target into Boa as its own JS object — and then READ every
//! text file under 1 MB and `split("\n")` it to report a line count, the single
//! most expensive operation in this engine. A longer declared deadline (120s
//! rather than the 30s default) is more rope, not a bound, and an overrun
//! returns NOTHING to the model.
//!
//! The contract under test:
//!
//! - discovery is level-by-level and non-recursive, honors `max_depth`, and
//!   never descends into the noise dirs — which are still shown, so the tree
//!   does not lie about what exists;
//! - no file contents are read at all, so no per-line cost can be incurred;
//! - entry, result-size and wall-clock budgets bound the walk, and every one
//!   announces itself — WHAT was cut, WHY, and that the call SUCCEEDED;
//! - every directory the walk did not fully cover is marked, so an empty child
//!   list never reads as "this directory is empty";
//! - an unreadable root returns a structured `success: false` instead of
//!   throwing.
//!
//! Tolerant by design (same as `default_skills_params.rs`): if the sibling
//! `nanna-tools/default-skills` tree isn't present, the tests no-op.

#![cfg(feature = "boa")]

use nanna_scripting::{ScriptEngine, ScriptedTool, ToolPermissions};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../nanna-tools/default-skills/project_structure/tool.ts")
}

/// Execute the real project_structure tool.ts against `input`, sandboxed to
/// `dir`.
///
/// The 120s timeout mirrors production: `ScriptedSkill::from_file` applies the
/// manifest's `timeout: 120`, while `ScriptedTool::from_file` alone would leave
/// the 30s default. The skill derives its work budget from 120s, so a harness
/// that gave it only 30s would be testing a deadline the daemon never uses.
async fn run_structure(input: Value, dir: &Path) -> Result<Value, String> {
    let tool = ScriptedTool::from_file(skill_path())
        .expect("read project_structure tool.ts")
        .with_permissions(ToolPermissions::none().with_read([dir]))
        .with_timeout(120_000);
    ScriptEngine::new()
        .execute(&tool, input, None, None)
        .await
        .map(|r| r.value)
        .map_err(|e| e.to_string())
}

/// Run and expect success; returns the result content string.
async fn run_structure_ok(input: Value, dir: &Path) -> String {
    let result = run_structure(input, dir)
        .await
        .expect("project_structure should succeed");
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

fn tail(s: &str) -> &str {
    &s[s.len().saturating_sub(900)..]
}

#[tokio::test]
async fn renders_a_tree_with_sizes_and_counts() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "a.rs", "fn main() {}");
    seed(dir.path(), "src/lib.rs", "mod b;");

    let content = run_structure_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    // Directories sort before files, each level drawn with tree connectors.
    assert!(content.contains("├── src/"), "got: {content}");
    assert!(content.contains("└── a.rs  12B"), "got: {content}");
    assert!(content.contains("lib.rs  6B"), "got: {content}");
    assert!(content.contains("1 directory, 2 files"), "got: {content}");
    // Nothing was bounded, so nothing announces itself.
    assert!(!content.contains("STOPPED"), "got: {content}");
    assert!(!content.contains("PARTIAL"), "got: {content}");
}

/// The old tool read every text file under 1 MB and `split("\n")` it purely to
/// print an `(NL)` column. That split measured ~99% of all scan cost and grows
/// super-linearly in line count, so it is gone: sizes come from the directory
/// metadata the listing already carries, and the tool reads zero file bytes.
#[tokio::test]
async fn reports_no_line_counts_so_no_file_is_ever_read() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "many_lines.rs", &"let x = 1;\n".repeat(500));

    let content = run_structure_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(content.contains("many_lines.rs"), "got: {content}");
    // The size column stays; the line-count column is gone.
    assert!(content.contains("5.4KB"), "got: {content}");
    assert!(
        !content.contains("500L") && !content.contains("L)"),
        "no line-count column may survive: {content}"
    );
}

#[tokio::test]
async fn noise_dirs_are_shown_but_never_descended_and_the_skip_announces_itself() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "src/main.rs", "fn main() {}");
    seed(dir.path(), "node_modules/pkg/vendored.js", "x");
    seed(dir.path(), "target/debug/generated.rs", "x");

    let content = run_structure_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(content.contains("main.rs"), "got: {content}");
    assert!(
        !content.contains("vendored.js") && !content.contains("generated.rs"),
        "must not descend into noise dirs: {content}"
    );
    // Shown, not omitted: a tree that hides node_modules lies about the layout.
    assert!(content.contains("node_modules/ (skipped)"), "got: {content}");
    assert!(content.contains("target/ (skipped)"), "got: {content}");
    // And the skip announces itself.
    assert!(
        content.contains("marks noise dirs that exist but were never descended into"),
        "got: {content}"
    );
}

#[tokio::test]
async fn depth_bound_limits_the_walk_and_marks_what_it_cut() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "shallow.rs", "x");
    seed(dir.path(), "a/b/deep.rs", "x");

    let shallow = run_structure_ok(
        json!({ "path": dir.path().to_string_lossy(), "max_depth": 1 }),
        dir.path(),
    )
    .await;
    assert!(shallow.contains("shallow.rs"), "got: {shallow}");
    assert!(!shallow.contains("deep.rs"), "got: {shallow}");
    // A directory that holds more must not render like an empty one.
    assert!(shallow.contains("a/ ..."), "got: {shallow}");
    assert!(
        shallow.contains("max_depth is 1"),
        "the depth cut must name its bound: {shallow}"
    );

    let deep = run_structure_ok(
        json!({ "path": dir.path().to_string_lossy(), "max_depth": 3 }),
        dir.path(),
    )
    .await;
    assert!(deep.contains("deep.rs"), "got: {deep}");
    assert!(!deep.contains("..."), "nothing was cut: {deep}");
}

#[tokio::test]
async fn empty_directory_is_an_observation_not_an_error() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();

    let content = run_structure_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(content.contains("Empty directory"), "got: {content}");
}

/// The result budget: 32,000 chars = 10 memory chunks, each of which costs an
/// embedding, a similarity search and a write. Long names blow the char budget
/// long before the entry budget, so this exercises the render cap alone.
#[tokio::test]
async fn output_budget_trips_and_announces_itself() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // 400 files x ~215 chars a line = ~86,000 chars, well past the budget,
    // with only 400 entries — far under the 4571-entry discovery budget.
    let long = "n".repeat(200);
    for i in 0..400 {
        std::fs::write(dir.path().join(format!("{long}{i}.rs")), "x").expect("seed");
    }

    let content = run_structure_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(
        content.contains("STOPPED: the tree filled the 32000-char result budget"),
        "got tail: {}",
        tail(&content)
    );
    assert!(
        content.contains("10 memory chunks"),
        "the budget must name the constraint it derives from: {}",
        tail(&content)
    );
    assert!(
        content.contains("SUCCEEDED"),
        "a budgeted stop must not read as a failure: {}",
        tail(&content)
    );
    assert!(
        content.contains("(PARTIAL"),
        "the header must flag the partial result: {}",
        &content[..200.min(content.len())]
    );
    // The bound is real: the body stays inside the derived char budget, and the
    // notes that explain it are a small bounded addition on top.
    assert!(
        content.len() < 36_000,
        "result should be bounded near 32,000 chars, got {}",
        content.len()
    );
}

/// The entry budget: 4571 = floor(32000 / 7), the most entries that could ever
/// fit the result budget, since the shortest line the tool can emit is a
/// one-character top-level directory (4-char connector + name + "/" + newline).
/// This is the exact shape of the live failure — pure marshalling cost, paid
/// before anything is rendered.
#[tokio::test]
async fn entry_budget_trips_and_announces_itself() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for i in 0..4_572 {
        std::fs::write(dir.path().join(format!("e{i}.log")), "x").expect("seed");
    }

    let content = run_structure_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(
        content.contains("STOPPED: hit the 4571-entry discovery budget"),
        "got tail: {}",
        tail(&content)
    );
    assert!(
        content.contains("SUCCEEDED"),
        "a budgeted stop must not read as a failure: {}",
        tail(&content)
    );
    assert!(
        content.contains("point `path` at a subdirectory"),
        "must offer narrowing guidance: {}",
        tail(&content)
    );
    // The directory whose listing was cut short is the root itself here, and
    // the root line is rendered separately from every other node — so it is
    // the one place a mark can silently go missing.
    assert!(
        content.contains("/ (partial)"),
        "the half-listed root must be marked: {}",
        &content[..400.min(content.len())]
    );
}

/// When a budget stops the walk mid-tree, the directories it never listed are
/// marked. Without the mark they render as childless, which is exactly how a
/// genuinely empty directory renders — the model cannot tell the two apart.
#[tokio::test]
async fn directories_the_budget_never_reached_are_marked() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // A wide top level that exhausts the entry budget, plus subdirectories
    // queued for depth 2 that the walk will therefore never get to.
    for i in 0..4_572 {
        std::fs::write(dir.path().join(format!("e{i}.log")), "x").expect("seed");
    }
    seed(dir.path(), "aaa_sub/child.rs", "x");

    let content = run_structure_ok(json!({ "path": dir.path().to_string_lossy() }), dir.path()).await;

    assert!(
        content.contains("aaa_sub/ (not walked)"),
        "an unreached directory must not render as an empty one: {}",
        &content[..600.min(content.len())]
    );
    assert!(
        content.contains("mark directories the walk never reached"),
        "got tail: {}",
        tail(&content)
    );
    assert!(!content.contains("child.rs"), "got: {content}");
}

#[tokio::test]
async fn missing_path_returns_structured_failure_not_a_throw() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let ghost = dir.path().join("no_such_dir");

    let result = run_structure(json!({ "path": ghost.to_string_lossy() }), dir.path())
        .await
        .expect("an inaccessible root must be returned, not thrown");

    assert_eq!(
        result["success"],
        Value::Bool(false),
        "expected success:false, got: {result}"
    );
    let content = result["content"].as_str().expect("content string");
    assert!(content.contains("no_such_dir"), "got: {content}");
    assert!(content.contains("Nothing was walked"), "got: {content}");
    assert!(content.contains("exists"), "got: {content}");
}

/// Manual timing check against a REAL large tree (the nanna repo root,
/// including node_modules/.git/target — the workspace shape that produced
/// nothing at all before this was bounded). Ignored by default: wall-time on a
/// shared box is not a stable assertion.
/// Run: cargo test -p nanna-scripting --features boa --test project_structure_skill \
///        -- --ignored --nocapture timing
#[tokio::test]
#[ignore = "manual timing check against the real repo tree"]
async fn timing_against_real_workspace() {
    if skill_missing() {
        return;
    }
    // No canonicalize(): on Windows it yields a `\\?\` extended-length path,
    // and the tool joins child segments with `/`, which `\\?\` paths reject.
    let repo_root = std::env::var("PROJECT_STRUCTURE_TIMING_PATH").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        PathBuf::from,
    );

    let start = std::time::Instant::now();
    let content = run_structure_ok(
        json!({ "path": repo_root.to_string_lossy(), "max_depth": 4 }),
        &repo_root,
    )
    .await;
    let elapsed = start.elapsed();

    println!("project_structure({}) took {elapsed:?}", repo_root.display());
    println!("--- output ({} bytes) ---\n{content}", content.len());
    assert!(
        elapsed < std::time::Duration::from_secs(120),
        "the bounded walk must beat the 120s script deadline, took {elapsed:?}"
    );
}
