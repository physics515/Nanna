//! Behavioral tests for the `code_search` default skill, executed for real
//! through the Boa engine with a bridge scoped to a temp directory.
//!
//! The contract under test (the same failure class PR #149 fixed in `explore`:
//! an unbounded walk that marshals a whole workspace into Boa and blows the
//! 30s script deadline before producing anything) — code_search additionally
//! READ every candidate file to prove a negative, so it had two unbounded
//! costs, not one:
//!
//! - discovery is level-by-level and non-recursive, honors `depth`, and never
//!   descends into the noise dirs (node_modules, .git, target, ...);
//! - a whole-file regex prefilter keeps non-matching files out of the per-line
//!   pass, which measured ~99% of the cost, without dropping anchored matches;
//! - the files that DO reach that pass are split in bounded ~4 KiB slices, so
//!   the walk costs lines x slice instead of lines x file length — a byte cap
//!   alone cannot bound a `split("\n")`, and can be preempted between slices;
//! - wall-clock against the real script deadline is the primary bound, with
//!   derived caps on entries, file size, rendered file size and result chars —
//!   and every one announces itself in the output when it trips;
//! - a bad pattern, a bad regex, or an unreadable root returns a structured
//!   `success: false` result instead of throwing.
//!
//! Tolerant by design (same as `default_skills_params.rs`): if the sibling
//! `nanna-tools/default-skills` tree isn't present, the tests no-op.

#![cfg(feature = "boa")]

mod common;

use nanna_scripting::{ScriptEngine, ScriptedTool, ToolPermissions};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn skill_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../nanna-tools/default-skills/code_search/tool.ts")
}

/// Execute the real code_search tool.ts against `input`, sandboxed to `dir`.
async fn run_search(input: Value, dir: &Path) -> Result<Value, String> {
    let tool = ScriptedTool::from_file(skill_path())
        .expect("read code_search tool.ts")
        .with_permissions(ToolPermissions::none().with_read([dir]))
        // Scaffolding, not an assertion — see `common::FIXTURE_TIMEOUT_MS`.
        .with_timeout(common::FIXTURE_TIMEOUT_MS);
    ScriptEngine::new()
        .execute(&tool, input, None, None)
        .await
        .map(|r| r.value)
        .map_err(|e| e.to_string())
}

/// Run and expect success; returns the result content string.
async fn run_search_ok(input: Value, dir: &Path) -> String {
    let result = run_search(input, dir).await.expect("search should succeed");
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
async fn finds_matches_with_line_numbers_and_context() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(
        dir.path(),
        "a.rs",
        "one\ntwo\nthe needle here\nfour\nfive\n",
    );

    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;

    assert!(
        content.contains("1 match(es) in 1 file(s)"),
        "got: {content}"
    );
    assert!(content.contains("a.rs"), "got: {content}");
    // The match line is marked and numbered; ±2 lines of context come with it.
    assert!(content.contains(">   3: the needle here"), "got: {content}");
    assert!(content.contains("    2: two"), "got: {content}");
    assert!(content.contains("    5: five"), "got: {content}");
}

/// The whole-file prefilter that keeps non-matching files out of the expensive
/// per-line pass must never drop a file the per-line pass would have matched.
/// Anchored patterns are the case that catches it: `^needle` matches line 3 of
/// this file, but against the whole file text `^` would anchor to byte 0.
#[tokio::test]
async fn anchored_patterns_still_match_mid_file() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "a.rs", "one\ntwo\nneedle at line start\nfour\n");

    let start = run_search_ok(
        json!({ "pattern": "^needle", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;
    assert!(start.contains("1 match(es)"), "got: {start}");
    assert!(start.contains(">   3: needle"), "got: {start}");

    let end = run_search_ok(
        json!({ "pattern": "start$", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;
    assert!(end.contains("1 match(es)"), "got: {end}");
}

#[tokio::test]
async fn no_matches_is_an_observation_not_an_error() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "a.rs", "fn main() {}\n");

    let content = run_search_ok(
        json!({ "pattern": "zzz_absent_zzz", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;

    assert!(content.contains("No matches"), "got: {content}");
    // A complete negative must say it is complete — otherwise "no matches"
    // and "gave up early" are indistinguishable to the model.
    assert!(
        content.contains("complete: the whole tree was scanned"),
        "got: {content}"
    );
}

#[tokio::test]
async fn noise_dirs_are_never_descended_and_the_skip_announces_itself() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "src/main.rs", "let needle = 1;\n");
    seed(
        dir.path(),
        "node_modules/pkg/vendored.rs",
        "let needle = 2;\n",
    );
    seed(dir.path(), "target/debug/generated.rs", "let needle = 3;\n");

    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;

    assert!(
        content.contains("1 match(es) in 1 file(s)"),
        "got: {content}"
    );
    assert!(content.contains("main.rs"), "got: {content}");
    assert!(
        !content.contains("vendored.rs") && !content.contains("generated.rs"),
        "must not descend into noise dirs: {content}"
    );
    // The skip must announce itself.
    assert!(
        content.contains("Skipped noise dirs (never descended)"),
        "got: {content}"
    );
    assert!(content.contains("node_modules"), "got: {content}");
    assert!(content.contains("target"), "got: {content}");
}

#[tokio::test]
async fn depth_bound_limits_the_walk() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "shallow.rs", "needle\n");
    seed(dir.path(), "a/b/deep.rs", "needle\n");

    let shallow = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy(), "depth": 1 }),
        dir.path(),
    )
    .await;
    assert!(shallow.contains("1 match(es)"), "got: {shallow}");
    assert!(!shallow.contains("deep.rs"), "got: {shallow}");

    let deep = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy(), "depth": 3 }),
        dir.path(),
    )
    .await;
    assert!(deep.contains("2 match(es)"), "got: {deep}");
    assert!(deep.contains("deep.rs"), "got: {deep}");
}

#[tokio::test]
async fn file_pattern_filters_candidates() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "a.rs", "needle\n");
    seed(dir.path(), "b.md", "needle\n");

    let content = run_search_ok(
        json!({
            "pattern": "needle",
            "path": dir.path().to_string_lossy(),
            "file_pattern": "*.rs"
        }),
        dir.path(),
    )
    .await;

    assert!(
        content.contains("1 match(es) in 1 file(s)"),
        "got: {content}"
    );
    assert!(content.contains("a.rs"), "got: {content}");
    assert!(!content.contains("b.md"), "got: {content}");
}

#[tokio::test]
async fn max_results_stops_the_search_and_says_so() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "a.rs", &"needle\n".repeat(20));

    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy(), "max_results": 3 }),
        dir.path(),
    )
    .await;

    assert!(content.contains("3 match(es)"), "got: {content}");
    assert!(
        content.contains("Stopped at the requested max_results of 3"),
        "got: {content}"
    );
}

#[tokio::test]
async fn binary_files_are_skipped_before_they_are_read() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Text content, binary extension: the extension filter must catch it
    // BEFORE the read, because the marshal is the expensive half.
    seed(dir.path(), "logo.png", "needle\n");

    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;

    assert!(content.contains("No matches"), "got: {content}");
    assert!(content.contains("0 file(s) scanned"), "got: {content}");
    assert!(
        content.contains("Skipped 1 binary/non-text file(s)"),
        "got: {content}"
    );
}

#[tokio::test]
async fn oversized_files_are_skipped_and_announced() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // MAX_FILE_BYTES is MAX_SCAN_BYTES/8 = 1 MiB: no single file may eat an
    // eighth of the corpus budget.
    let mut big = String::with_capacity(1_100_000);
    while big.len() < 1_100_000 {
        big.push_str("needle padding line\n");
    }
    seed(dir.path(), "huge.rs", &big);
    seed(dir.path(), "small.rs", "needle\n");

    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;

    assert!(
        content.contains("1 match(es) in 1 file(s)"),
        "got: {content}"
    );
    assert!(content.contains("small.rs"), "got: {content}");
    assert!(
        content.contains("Skipped 1 file(s) at or above 1.0MB"),
        "got: {content}"
    );
}

/// A file that matches but is too large to render context from is reported as
/// a match, not silently dropped and not mislabelled "no matches".
#[tokio::test]
async fn oversized_matches_are_reported_without_context() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Over RENDER_MAX_CHARS (128 KiB) but under MAX_FILE_BYTES (1 MiB).
    let mut big = String::from("the needle is here\n");
    while big.len() < 200_000 {
        big.push_str("padding line that does not match\n");
    }
    seed(dir.path(), "huge.rs", &big);

    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;

    assert!(
        !content.contains("No matches"),
        "a file that contains the pattern must not be reported as no-match: {content}"
    );
    assert!(content.contains("CONTAIN"), "got: {content}");
    assert!(
        content.contains("larger than 128.0KB"),
        "the skip must name its bound: {content}"
    );
    assert!(content.contains("search_file"), "got: {content}");
}

/// The deadline bug the byte cap could not catch. `RENDER_MAX_CHARS` is a
/// BYTE cap, but `split("\n")` costs the PRODUCT of line count and string
/// length (~1e-8 x lines x chars), so a file can sit right at the cap and
/// still be catastrophically expensive: 128 KiB of one-character lines —
/// minified JS, a CSV, a log, a base64 blob wrapped narrow — is 65,536 lines,
/// and one `split` over it measured ~84s. That is past the 30s script
/// deadline, and the budget check happens BEFORE each file, so the clock
/// cannot preempt a split once entered: the whole call returns NOTHING.
///
/// Splitting in bounded ~4 KiB slices turns O(lines x length) into
/// O(lines x slice), which is what makes the byte cap true — the same file
/// walks in ~2.7s. The needle sits on the LAST line so the whole file must be
/// walked; nothing here can pass by exiting early.
#[tokio::test]
async fn dense_short_lines_at_the_render_cap_finish_inside_the_deadline() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Exactly at RENDER_MAX_CHARS (128 KiB = 131,072), not over it: at one
    // byte more the file would be skipped as un-rendered and this would prove
    // nothing. 65,532 one-char lines + "needle\n" = 131,071 bytes.
    let mut dense = String::with_capacity(131_072);
    for _ in 0..65_532 {
        dense.push_str("x\n");
    }
    dense.push_str("needle\n");
    assert!(
        dense.len() <= 128 * 1024,
        "seed must stay under the render cap"
    );
    seed(dir.path(), "dense.js", &dense);

    let started = std::time::Instant::now();
    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;
    let elapsed = started.elapsed();

    // The whole point: before the sliced walk this call returned nothing at
    // all, because one split over this file outruns the script deadline.
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "must return inside the 30s script deadline, took {elapsed:?}"
    );
    assert!(
        content.contains("1 match(es) in 1 file(s)"),
        "got: {content}"
    );
    // Walked, not skipped: the line context is really there, and the line
    // number is only right if every slice contributed exactly its own lines.
    assert!(
        content.contains(">65533: needle"),
        "got tail: {}",
        tail(&content)
    );
    assert!(
        !content.contains("larger than"),
        "the file is at the render cap, not over it — it must be walked, \
         not reported as un-rendered: {content}"
    );
    assert!(
        !content.contains("STOPPED"),
        "a file at the render cap must finish well inside the work budget: {}",
        tail(&content)
    );
}

/// The sliced walk must produce exactly the array one `split("\n")` would.
/// Line lengths here (37 bytes) divide the 4 KiB slice unevenly, so slice
/// boundaries land mid-line constantly — if the walk cut a line in half, or
/// dropped/doubled the empty segment a slice ending on "\n" leaves behind,
/// the reported line number would drift from the true one.
#[tokio::test]
async fn sliced_walk_numbers_lines_exactly_like_one_split() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::new();
    for i in 1..=3_000 {
        if i == 2_345 {
            body.push_str("needle\n");
        } else {
            body.push_str("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n");
        }
    }
    seed(dir.path(), "wide.rs", &body);

    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;

    assert!(
        content.contains("1 match(es) in 1 file(s)"),
        "got: {content}"
    );
    assert!(
        content.contains(">2345: needle"),
        "line number must survive ~27 slice boundaries: {}",
        tail(&content)
    );
}

/// The primary bound: wall-clock against the real script deadline. Every one
/// of these files matches, so every one pays the per-line walk — the budget
/// must cut the search off and hand back a partial result rather than let the
/// whole call hit the 30s deadline and return nothing.
///
/// The corpus is sized against the SLICED walk. It used to be 80 x 58 KB of
/// ordinary source, which cost ~45s when each file was one `split("\n")`;
/// slicing dropped that to ~4s, so the budget stopped tripping and the test
/// stopped testing anything. The bound did not change — what it takes to
/// reach it did. So each file here is now the worst shape the render cap
/// admits: 128 KiB of one-character lines, ~2.7s to walk in slices, with the
/// needle on the LAST line so no file can exit early. 24 of them is ~65s of
/// walk work against a 20s budget; the run stops around the seventh, and the
/// surplus files cost nothing because they are never reached.
#[tokio::test]
async fn time_budget_trips_and_announces_itself() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::with_capacity(131_072);
    for _ in 0..65_526 {
        body.push_str("x\n");
    }
    body.push_str("the needle is here\n");
    assert!(
        body.len() <= 128 * 1024,
        "each file must stay under the render cap"
    );
    for i in 0..24 {
        seed(dir.path(), &format!("f{i}.rs"), &body);
    }

    let started = std::time::Instant::now();
    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy(), "max_results": 500 }),
        dir.path(),
    )
    .await;
    let elapsed = started.elapsed();

    assert!(
        content.contains("STOPPED: spent the 20000ms work budget"),
        "got tail: {}",
        tail(&content)
    );
    assert!(
        content.contains("SUCCEEDED"),
        "a budgeted stop must not read as a failure: {}",
        tail(&content)
    );
    assert!(
        content.contains("path/depth/file_pattern"),
        "must offer narrowing guidance: {}",
        tail(&content)
    );
    // The whole point: it returns instead of hitting the 30s deadline.
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "must return inside the script deadline, took {elapsed:?}"
    );
}

#[tokio::test]
async fn output_budget_trips_and_announces_itself() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // Each file yields one ~15,000-char section (3 lines x 5,000 chars), so
    // the third would pass the 32,000-char (10 memory chunk) result budget.
    let pad = "p".repeat(5_000);
    let body = format!("{pad}\n{pad}needle\n{pad}\n");
    for i in 0..6 {
        seed(dir.path(), &format!("f{i}.rs"), &body);
    }

    let content = run_search_ok(
        json!({ "pattern": "needle", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await;

    assert!(
        content.contains("STOPPED: the next match section would pass the 32000-char result budget"),
        "got: {}",
        &content[content.len().saturating_sub(600)..]
    );
    assert!(
        content.contains("10 memory chunks"),
        "the budget must name the constraint it derives from: {}",
        &content[content.len().saturating_sub(600)..]
    );
    assert!(
        content.contains("(PARTIAL"),
        "the header must flag the partial result: {}",
        &content[..200.min(content.len())]
    );
}

#[tokio::test]
async fn missing_pattern_returns_structured_failure_not_a_throw() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();

    let result = run_search(json!({ "path": dir.path().to_string_lossy() }), dir.path())
        .await
        .expect("a missing parameter must be returned, not thrown");

    assert_eq!(
        result["success"],
        Value::Bool(false),
        "expected success:false, got: {result}"
    );
    let content = result["content"].as_str().expect("content string");
    assert!(content.contains("pattern"), "got: {content}");
    assert!(content.contains("Nothing was searched"), "got: {content}");
}

#[tokio::test]
async fn invalid_regex_returns_structured_failure_not_a_throw() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "a.rs", "fn main() {}\n");

    let result = run_search(
        json!({ "pattern": "fn main(", "path": dir.path().to_string_lossy() }),
        dir.path(),
    )
    .await
    .expect("a bad regex must be returned, not thrown");

    assert_eq!(
        result["success"],
        Value::Bool(false),
        "expected success:false, got: {result}"
    );
    let content = result["content"].as_str().expect("content string");
    assert!(content.contains("not a valid regex"), "got: {content}");
    assert!(content.contains("Nothing was searched"), "got: {content}");
}

#[tokio::test]
async fn missing_path_returns_structured_failure_not_a_throw() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let ghost = dir.path().join("no_such_dir");

    let result = run_search(
        json!({ "pattern": "needle", "path": ghost.to_string_lossy() }),
        dir.path(),
    )
    .await
    .expect("an inaccessible root must be returned, not thrown");

    assert_eq!(
        result["success"],
        Value::Bool(false),
        "expected success:false, got: {result}"
    );
    let content = result["content"].as_str().expect("content string");
    assert!(content.contains("no_such_dir"), "got: {content}");
    assert!(content.contains("exists"), "got: {content}");
}

/// The entry budget: 12,000 entries, derived from a tenth of the work budget
/// at the measured 8,800 entries/s marshal rate. `file_pattern` excludes every
/// file, so this exercises pure discovery cost — the exact shape of the live
/// failure, where marshalling a workspace's entries blew the deadline before
/// the script ran a line.
#[tokio::test]
async fn entry_budget_trips_and_announces_itself() {
    if skill_missing() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for i in 0..12_001 {
        std::fs::write(dir.path().join(format!("e{i}.log")), "x").expect("seed");
    }

    let content = run_search_ok(
        json!({
            "pattern": "zzz_absent_zzz",
            "path": dir.path().to_string_lossy(),
            "file_pattern": "*.rs"
        }),
        dir.path(),
    )
    .await;

    assert!(
        content.contains("STOPPED: hit the 12000-entry discovery budget"),
        "got: {content}"
    );
    assert!(content.contains("12000 entries walked"), "got: {content}");
}

fn tail(s: &str) -> &str {
    &s[s.len().saturating_sub(800)..]
}

/// The measurements every budget in the skill is derived from. Ignored by
/// default — throughput on a shared box is not a stable assertion — but the
/// numbers quoted in the skill's budget comments came from exactly this run,
/// and this is how to re-derive them when the engine or the machine changes.
///
/// Measured 2026-08-03, debug build, over 200 files x 58 KB (1100 lines):
///   read only            1.52s   ->  7.7 MB/s
///   read + regex(whole)  1.34s   ->  the prefilter is free
///   read + split("\n")   112.5s  ->  the split is ~99% of the cost
/// and 20,000 flat entries marshalled in 2.27s -> 8,800 entries/s.
///
/// The split's cost is super-linear in LINE COUNT, not bytes: the same 11.6 MB
/// as 2000 x 110-line files costs 28s, as 200 x 1100-line files 126s. That is
/// why the skill bounds wall-clock directly and caps the file size it will
/// render context from, rather than budgeting bytes.
///
/// Run: cargo test -p nanna-scripting --features boa --test code_search_skill \
///        -- --ignored --nocapture throughput
#[tokio::test]
#[ignore = "throughput measurement, not an assertion"]
async fn throughput_of_the_scan_and_discovery_paths() {
    const READ_ONLY: &str = r#"
export default { name: "probe", execute: function(input) {
  var root = String(input.path);
  var entries = Nanna.listDir(root, false, 200000);
  var bytes = 0;
  for (var i = 0; i < entries.length; i++) {
    if (entries[i].entry_type !== "file") continue;
    bytes += Nanna.readFile(root + "/" + entries[i].name).length;
  }
  return "bytes=" + bytes;
} }
"#;
    const READ_PREFILTER: &str = r#"
export default { name: "probe", execute: function(input) {
  var root = String(input.path);
  var entries = Nanna.listDir(root, false, 200000);
  var re = new RegExp("zzz_no_such_token_zzz", "i");
  var n = 0;
  for (var i = 0; i < entries.length; i++) {
    if (entries[i].entry_type !== "file") continue;
    if (re.test(Nanna.readFile(root + "/" + entries[i].name))) n++;
  }
  return "hits=" + n;
} }
"#;
    const READ_SPLIT: &str = r#"
export default { name: "probe", execute: function(input) {
  var root = String(input.path);
  var entries = Nanna.listDir(root, false, 200000);
  var re = new RegExp("zzz_no_such_token_zzz", "i");
  var n = 0;
  for (var i = 0; i < entries.length; i++) {
    if (entries[i].entry_type !== "file") continue;
    var lines = Nanna.readFile(root + "/" + entries[i].name).split("\n");
    for (var j = 0; j < lines.length; j++) { if (re.test(lines[j])) n++; }
  }
  return "hits=" + n;
} }
"#;
    const MARSHAL_ONLY: &str = r#"
export default { name: "probe", execute: function(input) {
  var entries = Nanna.listDir(String(input.path), false, 20000);
  var n = 0;
  for (var i = 0; i < entries.length; i++) { n += entries[i].name.length; }
  return "entries=" + entries.length;
} }
"#;

    async fn timed(src: &str, dir: &Path, label: &str) {
        let tool = ScriptedTool::new("probe.ts", src)
            .with_permissions(ToolPermissions::none().with_read([dir]))
            .with_timeout(600_000);
        let start = std::time::Instant::now();
        let out = ScriptEngine::new()
            .execute(&tool, json!({ "path": dir.to_string_lossy() }), None, None)
            .await
            .expect("probe ran")
            .value;
        println!("{label}: {out} in {:?}", start.elapsed());
    }

    // Stage isolation over 200 x 58 KB / 1100 lines — the shape a real source
    // tree has, and the shape that used to eat the deadline.
    let dir = tempfile::tempdir().unwrap();
    let body = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n".repeat(1100);
    for i in 0..200 {
        std::fs::write(dir.path().join(format!("f{i}.rs")), &body).unwrap();
    }
    println!("corpus: 200 files x {} B", body.len());
    timed(READ_ONLY, dir.path(), "read only          ").await;
    timed(READ_PREFILTER, dir.path(), "read + regex(whole)").await;
    timed(READ_SPLIT, dir.path(), "read + split(lines)").await;

    // Shape sensitivity: same total bytes, different line counts.
    for (lines, files) in [(110usize, 2000usize), (1100, 200)] {
        let d = tempfile::tempdir().unwrap();
        let b = "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n".repeat(lines);
        for i in 0..files {
            std::fs::write(d.path().join(format!("f{i}.rs")), &b).unwrap();
        }
        timed(
            READ_SPLIT,
            d.path(),
            &format!(
                "split: {files} x {lines} lines ({} B total)",
                files * b.len()
            ),
        )
        .await;
    }

    // Entry marshalling.
    let dir2 = tempfile::tempdir().unwrap();
    for i in 0..20_000 {
        std::fs::write(dir2.path().join(format!("e{i}.txt")), "x").unwrap();
    }
    timed(MARSHAL_ONLY, dir2.path(), "marshal 20k entries").await;
}

/// Manual timing check against a REAL large tree (the nanna repo root,
/// including node_modules/.git/target). Ignored by default: wall-time on a
/// shared box is not a stable assertion.
/// Run: cargo test -p nanna-scripting --features boa --test code_search_skill \
///        -- --ignored --nocapture timing
#[tokio::test]
#[ignore = "manual timing check against the real repo tree"]
async fn timing_against_real_workspace() {
    if skill_missing() {
        return;
    }
    // No canonicalize(): on Windows it yields a `\\?\` extended-length path,
    // and the tool joins child segments with `/`, which `\\?\` paths reject.
    let repo_root = std::env::var("CODE_SEARCH_TIMING_PATH").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        PathBuf::from,
    );

    let start = std::time::Instant::now();
    let content = run_search_ok(
        json!({ "pattern": "zzz_absent_needle_zzz", "path": repo_root.to_string_lossy() }),
        &repo_root,
    )
    .await;
    let elapsed = start.elapsed();

    println!("code_search({}) took {elapsed:?}", repo_root.display());
    println!("--- output ({} bytes) ---\n{content}", content.len());
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "the bounded search must beat the 30s script deadline, took {elapsed:?}"
    );
}
