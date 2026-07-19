//! Live long-horizon eval suite (P14): the real harness driving a real local
//! model with machine-run acceptance checks.
//!
//! Three `#[ignore]`d entry points (all need a running Ollama):
//!
//! - `live_task_success_at_tokens` — the 5-task smoke suite (Suite 4's live
//!   baseline row).
//! - `live_pass_k` — the smoke suite repeated K times (τ-bench-style pass^k:
//!   single-run success on a small model is noise; reliability is the
//!   long-horizon bottleneck). `NANNA_EVAL_K` (default 3).
//! - `live_endurance` — the "4-hour task": build `minidb`, a POSIX-shell
//!   key-value store CLI, against 42 seeded fail-to-pass feature tests
//!   (SWE-bench-style acceptance-by-tests), dependency-chained so `next()`
//!   walks the ladder. `NANNA_EVAL_HOURS` caps wall clock (default 6).
//!
//! ```text
//! NANNA_EVAL_MODEL=qwen3.5:9b cargo test -p nanna-daemon --test live_long_horizon -- --ignored --nocapture live_endurance
//! ```

use nanna_agent::harness::{LongHorizonConfig, LongHorizonReport, LongHorizonRunner, StopReason};
use nanna_daemon::llm_router::LlmRouter;
use nanna_daemon::tasks::{AgentStepRunner, TursoTaskSource, build_task_services};
use nanna_storage::{NewTask, Storage};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const EVAL_SESSION: &str = "live-eval";

fn eval_model() -> String {
    std::env::var("NANNA_EVAL_MODEL").unwrap_or_else(|_| "qwen3.5:9b".to_string())
}

fn init_tracing() {
    // Default warn-only: a multi-hour run at info would produce a huge log.
    // NANNA_EVAL_LOG overrides for diagnosis. Progress lines use println!
    // and bypass the filter either way.
    let filter = std::env::var("NANNA_EVAL_LOG").unwrap_or_else(|_| "warn".to_string());
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

// ---------------------------------------------------------------------------
// Shared environment
// ---------------------------------------------------------------------------

struct EvalEnv {
    storage: Arc<Storage>,
    runner: AgentStepRunner,
}

async fn build_env(workdir: &Path) -> EvalEnv {
    let storage = Arc::new(Storage::in_memory().await.expect("storage"));

    let tools = Arc::new(nanna_tools::ToolRegistry::new());
    tools
        .set_default_workdir(Some(workdir.to_path_buf()))
        .await;
    tools.set_session_id(Some(EVAL_SESSION.to_string())).await;
    let tools_dir = nanna_tools::skills::defaults::resolve_tools_dir(None)
        .expect("DEV_TOOLS_DIR must resolve in debug builds");
    let workspace_id = Arc::new(tokio::sync::RwLock::new(None));
    let services = build_task_services(storage.clone(), workspace_id);
    let loaded = tools.load_skills_with_services(&tools_dir, &services).await;
    assert!(loaded > 0, "no skills loaded from {tools_dir:?}");

    let router = Arc::new(LlmRouter::new().with_ollama("http://localhost:11434"));
    let runner = AgentStepRunner {
        router,
        tools,
        agent_config: nanna_agent::AgentConfig {
            model: eval_model(),
            max_tokens: 4096,
            temperature: 0.7,
            ..Default::default()
        },
        system_prompt: nanna_agent::prompts::DEFAULT_SYSTEM_PROMPT.to_string(),
        workspace_root: Some(workdir.to_path_buf()),
        stats: None,
    };
    EvalEnv { storage, runner }
}

fn source_for(storage: &Arc<Storage>) -> TursoTaskSource {
    TursoTaskSource::new(
        storage.clone(),
        "session".to_string(),
        Some(EVAL_SESSION.to_string()),
        "harness".to_string(),
        None,
    )
}

fn task(title: &str, description: &str, tools: &[&str], acceptance: serde_json::Value) -> NewTask {
    NewTask {
        scope: "session".to_string(),
        scope_id: Some(EVAL_SESSION.to_string()),
        title: title.to_string(),
        description: Some(description.to_string()),
        priority: 3,
        tool_scope: tools.iter().map(ToString::to_string).collect(),
        acceptance: Some(acceptance),
        ..NewTask::default()
    }
}

async fn print_report(
    label: &str,
    report: &LongHorizonReport,
    storage: &Arc<Storage>,
    plan_ids: &[i64],
    full_activity: bool,
) {
    let repo = storage.tasks();
    println!("\n================ {label} ================");
    println!("model: {}", eval_model());
    println!("report: {}", serde_json::to_string_pretty(report).unwrap());
    println!("tasks ({} total):", plan_ids.len());
    for id in plan_ids {
        let t = repo.get(*id).await.expect("task");
        println!("  #{} [{}] {}", t.id, t.status, t.title);
        if full_activity || t.status != "done" {
            for entry in repo.activity(*id, 8).await.unwrap_or_default() {
                let detail = entry
                    .detail
                    .map(|d| d.to_string())
                    .unwrap_or_default()
                    .chars()
                    .take(220)
                    .collect::<String>();
                println!("      {} {}", entry.action, detail);
            }
        }
    }
    println!("========================================================\n");
}

/// Integrity: every SEEDED task that reached `done` must have a
/// verified-completion activity entry. The model may legitimately create
/// extra tasks of its own through the todo tool (self-decomposition) and
/// those may complete on claim — they are reported, not forbidden.
async fn assert_seeded_verified(storage: &Arc<Storage>, plan_ids: &[i64]) -> usize {
    let repo = storage.tasks();
    for id in plan_ids {
        let t = repo.get(*id).await.expect("seeded task");
        if t.status == "done" {
            let verified = repo
                .activity(*id, 20)
                .await
                .unwrap_or_default()
                .iter()
                .any(|e| {
                    e.action == "completed"
                        && e.detail
                            .as_ref()
                            .and_then(|d| d.get("verified"))
                            .and_then(serde_json::Value::as_bool)
                            == Some(true)
                });
            assert!(
                verified,
                "seeded task #{id} completed without a verified acceptance check"
            );
        }
    }
    let all = repo
        .list("session", Some(EVAL_SESSION), true)
        .await
        .unwrap_or_default();
    let extras: Vec<String> = all
        .iter()
        .filter(|t| !plan_ids.contains(&t.id))
        .map(|t| format!("#{} [{}] {}", t.id, t.status, t.title))
        .collect();
    if !extras.is_empty() {
        println!("model-created extra tasks ({}):", extras.len());
        for line in &extras {
            println!("  {line}");
        }
    }
    // Seeded completions only — extras must not inflate the score.
    let mut done = 0usize;
    for id in plan_ids {
        if repo.get(*id).await.expect("seeded task").status == "done" {
            done += 1;
        }
    }
    done
}

// ---------------------------------------------------------------------------
// Smoke suite (5 minutes-scale tasks)
// ---------------------------------------------------------------------------

async fn seed_smoke_tasks(storage: &Arc<Storage>, workdir: &Path) -> Vec<i64> {
    std::fs::write(workdir.join("calc.sh"), "#!/bin/sh\n# prints 2+2\necho 5\n")
        .expect("seed calc.sh");

    let repo = storage.tasks();
    let t1 = repo
        .create(task(
            "Create the greeting file",
            "Create a file named greeting.txt in the current directory containing exactly \
             this one line of text: hello from nanna",
            &["write_file"],
            serde_json::json!({
                "kind": "regex", "path": "greeting.txt", "pattern": "hello from nanna"
            }),
        ))
        .await
        .expect("t1");
    let t2 = repo
        .create(task(
            "Write the numbers file",
            "Create a file named numbers.txt containing the numbers 1 through 10, one \
             number per line, nothing else.",
            &["write_file", "exec"],
            serde_json::json!({
                "kind": "command",
                "command": "test \"$(grep -c '[0-9]' numbers.txt)\" -eq 10"
            }),
        ))
        .await
        .expect("t2");
    let t3a = repo
        .create(task(
            "Create the data file",
            "Create a CSV file named data.csv with a header line 'name,score' followed by \
             exactly 3 data rows of your choosing (any names and numeric scores).",
            &["write_file"],
            serde_json::json!({
                "kind": "regex", "path": "data.csv", "pattern": "(?m)^name,score"
            }),
        ))
        .await
        .expect("t3a");
    let mut t3b = task(
        "Count the data rows",
        "Count the data rows in data.csv (excluding the header line) and write that \
         count as a single number into a file named rows.txt.",
        &["read_file", "write_file", "exec"],
        serde_json::json!({
            "kind": "regex", "path": "rows.txt", "pattern": "(?m)^\\s*3\\s*$"
        }),
    );
    t3b.depends_on = vec![t3a.id];
    let t3b = repo.create(t3b).await.expect("t3b");
    let t4 = repo
        .create(task(
            "Fix the calc script",
            "The file calc.sh is supposed to print the result of 2+2 but prints the wrong \
             number. Fix it so that running `sh calc.sh` prints 4.",
            &["read_file", "write_file", "exec"],
            serde_json::json!({
                "kind": "command", "command": "test \"$(sh calc.sh)\" = \"4\""
            }),
        ))
        .await
        .expect("t4");
    vec![t1.id, t2.id, t3a.id, t3b.id, t4.id]
}

const SMOKE_GOAL: &str = "Complete every task in the plan. Each task produces a concrete file \
     artifact in the working directory that the harness verifies.";

async fn run_smoke_once() -> (LongHorizonReport, usize, usize) {
    let workspace = tempfile::tempdir().expect("tempdir");
    let workdir = workspace.path().to_path_buf();
    let env = build_env(&workdir).await;
    let plan_ids = seed_smoke_tasks(&env.storage, &workdir).await;
    let tasks_total = plan_ids.len();

    let config = LongHorizonConfig {
        max_wall_clock: Duration::from_secs(15 * 60),
        max_replans_per_item: 1,
        ..LongHorizonConfig::default()
    };
    let source = source_for(&env.storage);
    let report = LongHorizonRunner::new(config)
        .run(SMOKE_GOAL, &source, &env.runner, &workdir, None)
        .await;
    print_report("SMOKE RUN", &report, &env.storage, &plan_ids, false).await;
    let seeded_done = assert_seeded_verified(&env.storage, &plan_ids).await;
    (report, seeded_done, tasks_total)
}

/// Suite 4 live baseline row.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a running Ollama at localhost:11434"]
async fn live_task_success_at_tokens() {
    init_tracing();
    let (report, completed, total) = run_smoke_once().await;
    println!("smoke: {completed}/{total} @ {:?} tokens/item", report.tokens_per_completed_item);
}

/// pass^k over the smoke suite: the probability the harness+model succeed on
/// ALL of k repeats. `NANNA_EVAL_K` (default 3).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a running Ollama at localhost:11434"]
async fn live_pass_k() {
    init_tracing();
    let k: usize = std::env::var("NANNA_EVAL_K")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    let mut per_run = Vec::new();
    for run in 1..=k {
        println!("--- pass^k run {run}/{k} ---");
        let (report, completed, total) = run_smoke_once().await;
        per_run.push((completed, total, report));
    }

    let all_perfect = per_run.iter().filter(|(c, t, _)| c == t).count();
    let total_tokens: u64 = per_run
        .iter()
        .map(|(_, _, r)| r.input_tokens + r.output_tokens)
        .sum();
    let total_completed: usize = per_run.iter().map(|(c, _, _)| *c).sum();
    println!("\n================ PASS^K SUMMARY ================");
    println!("model: {}", eval_model());
    println!("k: {k}");
    for (i, (c, t, r)) in per_run.iter().enumerate() {
        println!(
            "  run {}: {c}/{t} in {}s, {} tokens, stop={:?}",
            i + 1,
            r.wall_clock_secs,
            r.input_tokens + r.output_tokens,
            r.stop
        );
    }
    println!("perfect runs: {all_perfect}/{k} (pass^{k} = {})", all_perfect == k);
    println!(
        "aggregate: {total_completed}/{} items, {} tokens/item",
        per_run.iter().map(|(_, t, _)| t).sum::<usize>(),
        if total_completed > 0 {
            total_tokens / total_completed as u64
        } else {
            0
        }
    );
    println!("================================================\n");
}

// ---------------------------------------------------------------------------
// Endurance: minidb (42 fail-to-pass feature tests)
// ---------------------------------------------------------------------------

/// The pinned goal for the endurance run — carries the global contract so
/// every O(1) step prompt states the storage format and CLI conventions.
const MINIDB_GOAL: &str = "Build `minidb`, a key-value store CLI implemented as ONE POSIX shell \
     script at ./minidb in the current directory (it is always invoked as `sh ./minidb <command> \
     [args...]`). Data lives in the file named by the environment variable MINIDB_FILE \
     (default ./minidb_data) with exactly one record per line in the format key<TAB>value. \
     Keys and values never contain tabs or newlines. Feature test scripts live in tests/; the \
     current task names one. Run it with `sh tests/test_NN.sh` to see what is missing, then \
     extend ./minidb to make it pass WITHOUT breaking the earlier features. Keep the script \
     POSIX sh compatible (no bashisms are required by the tests).";

/// One rung of the feature ladder.
struct Feature {
    name: &'static str,
    spec: &'static str,
    test_body: &'static str,
}

/// The 42-feature minidb ladder. Each test script is self-contained: it
/// resets the db, exercises exactly one feature, and exits nonzero with a
/// FAIL line on the first broken assertion.
fn minidb_features() -> Vec<Feature> {
    vec![
        Feature {
            name: "usage on no args",
            spec: "Running `sh ./minidb` with no arguments exits with code 2 and prints a line containing the word 'usage' (any case).",
            test_body: r#"sh ./minidb >out.txt 2>&1
[ $? -eq 2 ] || fail "exit code should be 2"
grep -qi usage out.txt || fail "should print usage"
rm -f out.txt"#,
        },
        Feature {
            name: "help command",
            spec: "`sh ./minidb help` exits 0 and prints text containing 'minidb' and the word 'set'.",
            test_body: r#"sh ./minidb help >out.txt 2>&1 || fail "help should exit 0"
grep -qi minidb out.txt || fail "help should mention minidb"
grep -q set out.txt || fail "help should mention set"
rm -f out.txt"#,
        },
        Feature {
            name: "set and get",
            spec: "`set <key> <value>` stores a value (exit 0). `get <key>` prints the value and exits 0.",
            test_body: r#"sh ./minidb set name alice || fail "set should exit 0"
v=$(sh ./minidb get name) || fail "get should exit 0"
[ "$v" = "alice" ] || fail "get should print alice, got: $v""#,
        },
        Feature {
            name: "get missing key",
            spec: "`get` on a key that does not exist prints nothing and exits with code 1.",
            test_body: r#"out=$(sh ./minidb get nope 2>/dev/null)
rc=$?
[ $rc -eq 1 ] || fail "exit code should be 1, got $rc"
[ -z "$out" ] || fail "should print nothing, got: $out""#,
        },
        Feature {
            name: "set overwrites",
            spec: "Setting an existing key replaces its value; the file never holds two records for one key.",
            test_body: r#"sh ./minidb set k first || fail set1
sh ./minidb set k second || fail set2
v=$(sh ./minidb get k)
[ "$v" = "second" ] || fail "get should print second, got: $v"
n=$(grep -c "^k	" "$MINIDB_FILE")
[ "$n" -eq 1 ] || fail "db should hold exactly one record for k, got $n""#,
        },
        Feature {
            name: "independent keys",
            spec: "Different keys hold independent values.",
            test_body: r#"sh ./minidb set a 1 && sh ./minidb set b 2 || fail set
[ "$(sh ./minidb get a)" = "1" ] || fail "a should be 1"
[ "$(sh ./minidb get b)" = "2" ] || fail "b should be 2""#,
        },
        Feature {
            name: "del command",
            spec: "`del <key>` removes the key (exit 0); a later get exits 1.",
            test_body: r#"sh ./minidb set k v || fail set
sh ./minidb del k || fail "del should exit 0"
sh ./minidb get k >/dev/null 2>&1
[ $? -eq 1 ] || fail "get after del should exit 1""#,
        },
        Feature {
            name: "del missing key",
            spec: "`del` on a missing key exits with code 1.",
            test_body: r#"sh ./minidb del nothere >/dev/null 2>&1
[ $? -eq 1 ] || fail "del missing should exit 1""#,
        },
        Feature {
            name: "exists command",
            spec: "`exists <key>` prints nothing; exits 0 when the key is present, 1 when absent.",
            test_body: r#"sh ./minidb set k v || fail set
out=$(sh ./minidb exists k) || fail "exists should exit 0 for present key"
[ -z "$out" ] || fail "exists should print nothing"
sh ./minidb exists other >/dev/null 2>&1
[ $? -eq 1 ] || fail "exists should exit 1 for absent key""#,
        },
        Feature {
            name: "count command",
            spec: "`count` prints the number of stored keys (0 for an empty store) and exits 0.",
            test_body: r#"[ "$(sh ./minidb count)" = "0" ] || fail "empty count should be 0"
sh ./minidb set a 1 && sh ./minidb set b 2 || fail set
[ "$(sh ./minidb count)" = "2" ] || fail "count should be 2""#,
        },
        Feature {
            name: "list command",
            spec: "`list` prints all keys sorted (byte order), one per line.",
            test_body: r#"sh ./minidb set b 2 && sh ./minidb set a 1 && sh ./minidb set c 3 || fail set
got=$(sh ./minidb list)
want=$(printf 'a\nb\nc')
[ "$got" = "$want" ] || fail "list should print sorted keys, got: $got""#,
        },
        Feature {
            name: "clear command",
            spec: "`clear` removes every key (exit 0); count is 0 afterwards.",
            test_body: r#"sh ./minidb set a 1 && sh ./minidb set b 2 || fail set
sh ./minidb clear || fail "clear should exit 0"
[ "$(sh ./minidb count)" = "0" ] || fail "count after clear should be 0""#,
        },
        Feature {
            name: "values with spaces",
            spec: "Values may contain spaces: `set greet \"hello world\"` round-trips exactly.",
            test_body: r#"sh ./minidb set greet "hello world" || fail set
v=$(sh ./minidb get greet)
[ "$v" = "hello world" ] || fail "value with spaces should round-trip, got: $v""#,
        },
        Feature {
            name: "case-sensitive keys",
            spec: "Keys are case-sensitive: Key and key are different entries.",
            test_body: r#"sh ./minidb set Key A && sh ./minidb set key B || fail set
[ "$(sh ./minidb get Key)" = "A" ] || fail "Key should be A"
[ "$(sh ./minidb get key)" = "B" ] || fail "key should be B""#,
        },
        Feature {
            name: "MINIDB_FILE env",
            spec: "The MINIDB_FILE environment variable selects the database file.",
            test_body: r#"MINIDB_FILE=./alt_db sh ./minidb set k v || fail set
[ -f ./alt_db ] || fail "alt_db file should exist"
v=$(MINIDB_FILE=./alt_db sh ./minidb get k)
[ "$v" = "v" ] || fail "get from alt_db should print v"
sh ./minidb get k >/dev/null 2>&1
[ $? -eq 1 ] || fail "default db should not have the key"
rm -f ./alt_db"#,
        },
        Feature {
            name: "append command",
            spec: "`append <key> <text>` appends text to the existing value and exits 0.",
            test_body: r#"sh ./minidb set k ab || fail set
sh ./minidb append k cd || fail "append should exit 0"
[ "$(sh ./minidb get k)" = "abcd" ] || fail "append should concatenate""#,
        },
        Feature {
            name: "append creates missing",
            spec: "`append` on a missing key creates it with the given text.",
            test_body: r#"sh ./minidb append fresh xy || fail "append should exit 0"
[ "$(sh ./minidb get fresh)" = "xy" ] || fail "append should create the key""#,
        },
        Feature {
            name: "incr command",
            spec: "`incr <key>` increments a numeric value by 1, prints the new value, exits 0.",
            test_body: r#"sh ./minidb set n 5 || fail set
out=$(sh ./minidb incr n) || fail "incr should exit 0"
[ "$out" = "6" ] || fail "incr should print 6, got: $out"
[ "$(sh ./minidb get n)" = "6" ] || fail "stored value should be 6""#,
        },
        Feature {
            name: "incr creates at 1",
            spec: "`incr` on a missing key creates it at 1 and prints 1.",
            test_body: r#"out=$(sh ./minidb incr m) || fail "incr should exit 0"
[ "$out" = "1" ] || fail "incr missing should print 1, got: $out""#,
        },
        Feature {
            name: "incr non-numeric",
            spec: "`incr` on a non-numeric value exits with code 2 and leaves the value unchanged.",
            test_body: r#"sh ./minidb set s abc || fail set
sh ./minidb incr s >/dev/null 2>&1
[ $? -eq 2 ] || fail "incr non-numeric should exit 2"
[ "$(sh ./minidb get s)" = "abc" ] || fail "value should be unchanged""#,
        },
        Feature {
            name: "decr command",
            spec: "`decr <key>` decrements a numeric value by 1, prints the new value, exits 0.",
            test_body: r#"sh ./minidb set n 5 || fail set
out=$(sh ./minidb decr n) || fail "decr should exit 0"
[ "$out" = "4" ] || fail "decr should print 4, got: $out""#,
        },
        Feature {
            name: "mset command",
            spec: "`mset k1 v1 k2 v2 ...` sets several pairs in one call, exit 0.",
            test_body: r#"sh ./minidb mset a 1 b 2 c 3 || fail "mset should exit 0"
[ "$(sh ./minidb get a)" = "1" ] || fail a
[ "$(sh ./minidb get b)" = "2" ] || fail b
[ "$(sh ./minidb get c)" = "3" ] || fail c"#,
        },
        Feature {
            name: "mget command",
            spec: "`mget k1 k2 ...` prints each value on its own line, in argument order.",
            test_body: r#"sh ./minidb set a 1 && sh ./minidb set b 2 || fail set
got=$(sh ./minidb mget a b)
want=$(printf '1\n2')
[ "$got" = "$want" ] || fail "mget should print values in order, got: $got""#,
        },
        Feature {
            name: "rename command",
            spec: "`rename <old> <new>` moves the value; old is gone, new holds it. Exit 0.",
            test_body: r#"sh ./minidb set old v || fail set
sh ./minidb rename old new || fail "rename should exit 0"
sh ./minidb get old >/dev/null 2>&1
[ $? -eq 1 ] || fail "old key should be gone"
[ "$(sh ./minidb get new)" = "v" ] || fail "new key should hold the value""#,
        },
        Feature {
            name: "rename missing",
            spec: "`rename` on a missing source key exits with code 1.",
            test_body: r#"sh ./minidb rename ghost dst >/dev/null 2>&1
[ $? -eq 1 ] || fail "rename missing should exit 1""#,
        },
        Feature {
            name: "copy command",
            spec: "`copy <src> <dst>` duplicates a value; both keys hold it. Exit 0.",
            test_body: r#"sh ./minidb set src v || fail set
sh ./minidb copy src dst || fail "copy should exit 0"
[ "$(sh ./minidb get src)" = "v" ] || fail "src should keep the value"
[ "$(sh ./minidb get dst)" = "v" ] || fail "dst should hold the value""#,
        },
        Feature {
            name: "search command",
            spec: "`search <substring>` prints all keys containing the substring, sorted, one per line.",
            test_body: r#"sh ./minidb mset apple 1 apricot 2 banana 3 || fail mset
got=$(sh ./minidb search ap)
want=$(printf 'apple\napricot')
[ "$got" = "$want" ] || fail "search ap should print apple+apricot, got: $got""#,
        },
        Feature {
            name: "vgrep command",
            spec: "`vgrep <substring>` prints all keys whose VALUE contains the substring, sorted.",
            test_body: r#"sh ./minidb mset a hello b world c hell || fail mset
got=$(sh ./minidb vgrep hell)
want=$(printf 'a\nc')
[ "$got" = "$want" ] || fail "vgrep hell should print a+c, got: $got""#,
        },
        Feature {
            name: "export command",
            spec: "`export` prints every record as key<TAB>value lines, sorted by key.",
            test_body: r#"sh ./minidb set b 2 && sh ./minidb set a 1 || fail set
got=$(sh ./minidb export)
want=$(printf 'a\t1\nb\t2')
[ "$got" = "$want" ] || fail "export should print sorted TSV, got: $got""#,
        },
        Feature {
            name: "import command",
            spec: "`import <file>` reads key<TAB>value lines from the file and stores them all. Exit 0.",
            test_body: r#"printf 'x\t9\ny\t8\n' > imp.tsv
sh ./minidb import imp.tsv || fail "import should exit 0"
[ "$(sh ./minidb get x)" = "9" ] || fail x
[ "$(sh ./minidb get y)" = "8" ] || fail y
rm -f imp.tsv"#,
        },
        Feature {
            name: "import merges",
            spec: "`import` overwrites keys that already exist and keeps unrelated keys.",
            test_body: r#"sh ./minidb set x 1 && sh ./minidb set z 5 || fail set
printf 'x\t9\ny\t8\n' > imp.tsv
sh ./minidb import imp.tsv || fail import
[ "$(sh ./minidb get x)" = "9" ] || fail "x should be overwritten to 9"
[ "$(sh ./minidb get y)" = "8" ] || fail "y should be added"
[ "$(sh ./minidb get z)" = "5" ] || fail "z should be kept"
rm -f imp.tsv"#,
        },
        Feature {
            name: "sum command",
            spec: "`sum` prints the sum of all values that are integers, ignoring non-numeric values.",
            test_body: r#"sh ./minidb mset a 1 b 2 c abc || fail mset
[ "$(sh ./minidb sum)" = "3" ] || fail "sum should be 3""#,
        },
        Feature {
            name: "top command",
            spec: "`top <N>` prints the N keys with the largest integer values as 'key value' lines, descending.",
            test_body: r#"sh ./minidb mset a 5 b 9 c 1 || fail mset
got=$(sh ./minidb top 2)
want=$(printf 'b 9\na 5')
[ "$got" = "$want" ] || fail "top 2 should print b 9 then a 5, got: $got""#,
        },
        Feature {
            name: "delp prefix delete",
            spec: "`delp <prefix>` deletes every key starting with the prefix and prints the number removed.",
            test_body: r#"sh ./minidb mset user:a 1 user:b 2 other 3 || fail mset
out=$(sh ./minidb delp user:) || fail "delp should exit 0"
[ "$out" = "2" ] || fail "delp should print 2, got: $out"
[ "$(sh ./minidb count)" = "1" ] || fail "one key should remain""#,
        },
        Feature {
            name: "stats command",
            spec: "`stats` prints a line 'keys=N' and a line 'file=<db path>'.",
            test_body: r#"sh ./minidb set k v || fail set
sh ./minidb stats > out.txt || fail "stats should exit 0"
grep -q '^keys=1$' out.txt || fail "stats should print keys=1"
grep -q '^file=' out.txt || fail "stats should print file="
rm -f out.txt"#,
        },
        Feature {
            name: "backup and restore",
            spec: "`backup <file>` snapshots the db to the file; `restore <file>` replaces the db from it.",
            test_body: r#"sh ./minidb set k v1 || fail set
sh ./minidb backup b.db || fail "backup should exit 0"
sh ./minidb set k v2 || fail set2
sh ./minidb restore b.db || fail "restore should exit 0"
[ "$(sh ./minidb get k)" = "v1" ] || fail "restore should bring back v1"
rm -f b.db"#,
        },
        Feature {
            name: "validate command",
            spec: "`validate` exits 0 when every db line is key<TAB>value; exits 3 if any line is malformed.",
            test_body: r#"sh ./minidb set k v || fail set
sh ./minidb validate || fail "validate should exit 0 on clean db"
printf 'brokenlinewithnotab\n' >> "$MINIDB_FILE"
sh ./minidb validate >/dev/null 2>&1
[ $? -eq 3 ] || fail "validate should exit 3 on malformed db""#,
        },
        Feature {
            name: "repair command",
            spec: "`repair` drops malformed db lines, keeps valid records, exits 0; validate passes afterwards.",
            test_body: r#"sh ./minidb set a 1 || fail set
printf 'brokenlinewithnotab\n' >> "$MINIDB_FILE"
sh ./minidb repair || fail "repair should exit 0"
sh ./minidb validate || fail "validate should pass after repair"
[ "$(sh ./minidb get a)" = "1" ] || fail "valid record should survive repair"
[ "$(sh ./minidb count)" = "1" ] || fail "count should be 1""#,
        },
        Feature {
            name: "namespaced set/get",
            spec: "`nset <ns> <key> <value>` and `nget <ns> <key>` store per-namespace entries isolated from plain keys and other namespaces.",
            test_body: r#"sh ./minidb nset app k v1 || fail nset1
sh ./minidb nset web k v2 || fail nset2
sh ./minidb set k plain || fail set
[ "$(sh ./minidb nget app k)" = "v1" ] || fail "app ns should hold v1"
[ "$(sh ./minidb nget web k)" = "v2" ] || fail "web ns should hold v2"
[ "$(sh ./minidb get k)" = "plain" ] || fail "plain key should be isolated""#,
        },
        Feature {
            name: "nlist command",
            spec: "`nlist <ns>` prints the keys in that namespace sorted, one per line (without the namespace prefix).",
            test_body: r#"sh ./minidb nset app b 2 || fail n1
sh ./minidb nset app a 1 || fail n2
sh ./minidb nset web c 3 || fail n3
got=$(sh ./minidb nlist app)
want=$(printf 'a\nb')
[ "$got" = "$want" ] || fail "nlist app should print a+b, got: $got""#,
        },
        Feature {
            name: "jexport command",
            spec: "`jexport` prints the store as one JSON object on one line, keys sorted: {\"a\":\"1\",\"b\":\"2\"}.",
            test_body: r#"sh ./minidb set b 2 && sh ./minidb set a 1 || fail set
got=$(sh ./minidb jexport)
[ "$got" = '{"a":"1","b":"2"}' ] || fail "jexport mismatch, got: $got""#,
        },
        Feature {
            name: "readonly mode",
            spec: "When MINIDB_READONLY=1 is set, `set` exits with code 4 and changes nothing; `get` still works.",
            test_body: r#"sh ./minidb set k v || fail set
MINIDB_READONLY=1 sh ./minidb set k w >/dev/null 2>&1
[ $? -eq 4 ] || fail "readonly set should exit 4"
[ "$(sh ./minidb get k)" = "v" ] || fail "value should be unchanged"
[ "$(MINIDB_READONLY=1 sh ./minidb get k)" = "v" ] || fail "readonly get should work""#,
        },
    ]
}

/// Write tests/test_NN.sh scripts and seed the dependency-chained task ladder.
async fn seed_minidb_tasks(storage: &Arc<Storage>, workdir: &Path) -> Vec<i64> {
    let tests_dir = workdir.join("tests");
    std::fs::create_dir_all(&tests_dir).expect("tests dir");

    let mut features = minidb_features();
    // NANNA_EVAL_FEATURES truncates the ladder for cheap diagnosis runs.
    if let Some(limit) = std::env::var("NANNA_EVAL_FEATURES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        features.truncate(limit.max(1));
    }
    let repo = storage.tasks();
    let mut ids = Vec::new();
    let mut prev: Option<i64> = None;
    for (index, feature) in features.iter().enumerate() {
        let nn = index + 1;
        let script = format!(
            "#!/bin/sh\n# Feature {nn}: {name}\nexport LC_ALL=C\nexport MINIDB_FILE=./minidb_data\n\
             rm -f ./minidb_data\nfail() {{ echo \"FAIL(test_{nn:02}): $1\"; exit 1; }}\n\
             [ -f ./minidb ] || fail \"./minidb does not exist\"\n{body}\nexit 0\n",
            name = feature.name,
            body = feature.test_body,
        );
        std::fs::write(tests_dir.join(format!("test_{nn:02}.sh")), script).expect("write test");

        let mut new = task(
            &format!("Feature {nn:02}: {}", feature.name),
            &format!(
                "{} The test script tests/test_{nn:02}.sh checks exactly this; run it to see \
                 what fails, then extend ./minidb until it passes.",
                feature.spec
            ),
            &["read_file", "write_file", "exec"],
            serde_json::json!({
                "kind": "command",
                "command": format!("sh tests/test_{nn:02}.sh"),
                "timeout_secs": 60
            }),
        );
        new.sort_order = nn as i64;
        if let Some(prev_id) = prev {
            new.depends_on = vec![prev_id];
        }
        let created = repo.create(new).await.expect("seed feature task");
        prev = Some(created.id);
        ids.push(created.id);
    }
    ids
}

/// The endurance run: 42 dependency-chained fail-to-pass features. Progress
/// prints every 2 minutes so a multi-hour run is observable from the log.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a running Ollama at localhost:11434; runs for hours"]
async fn live_endurance() {
    init_tracing();
    let hours: f64 = std::env::var("NANNA_EVAL_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6.0);

    let workspace = tempfile::tempdir().expect("tempdir");
    let workdir = workspace.path().to_path_buf();
    let env = build_env(&workdir).await;
    let plan_ids = seed_minidb_tasks(&env.storage, &workdir).await;
    let total = plan_ids.len();
    println!(
        "endurance: {total} features seeded, wall-clock cap {hours}h, model {}",
        eval_model()
    );

    // Progress reporter: done/total every 2 minutes.
    let progress_storage = env.storage.clone();
    let started = std::time::Instant::now();
    let progress = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(120));
        tick.tick().await; // immediate first tick consumed
        loop {
            tick.tick().await;
            if let Ok((open, closed)) = progress_storage
                .tasks()
                .counts("session", Some(EVAL_SESSION))
                .await
            {
                println!(
                    "[progress] t={}m done={closed}/{} open={open}",
                    started.elapsed().as_secs() / 60,
                    closed as usize + open as usize,
                );
            }
        }
    });

    let config = LongHorizonConfig {
        max_wall_clock: Duration::from_secs_f64(hours * 3600.0),
        max_replans_per_item: 1,
        ..LongHorizonConfig::default()
    };
    let source = source_for(&env.storage);
    let report = LongHorizonRunner::new(config)
        .run(MINIDB_GOAL, &source, &env.runner, &workdir, None)
        .await;
    progress.abort();

    print_report("ENDURANCE RUN", &report, &env.storage, &plan_ids, false).await;
    // Preserve the built artifact for post-mortems.
    if let Ok(minidb) = std::fs::read_to_string(workdir.join("minidb")) {
        println!("---- final ./minidb ({} bytes) ----\n{minidb}\n----", minidb.len());
    }
    let seeded_done = assert_seeded_verified(&env.storage, &plan_ids).await;
    println!(
        "endurance summary: {seeded_done}/{total} features verified in {}s ({:.1}h)",
        report.wall_clock_secs,
        report.wall_clock_secs as f64 / 3600.0
    );
    assert!(
        !matches!(report.stop, StopReason::SourceError { .. }),
        "storage must survive the whole run: {:?}",
        report.stop
    );
}
