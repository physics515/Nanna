//! Live long-horizon eval (P14): the real harness driving a real local model
//! over a real task plan with machine-run acceptance checks.
//!
//! This is the on-model half of bench/BASELINE.md Suite 4. It is `#[ignore]`d
//! because it needs a running Ollama; run it explicitly with:
//!
//! ```text
//! NANNA_EVAL_MODEL=qwen3.5:9b cargo test -p nanna-daemon --test live_long_horizon -- --ignored --nocapture
//! ```
//!
//! Design per ROADMAP P14 research: minutes-scale tasks (a raw 7-9B agent
//! resolves ~0.7% of repo-scale SWE-bench items — that grain would only
//! measure zero), every task verified by environment state (file_exists /
//! regex / command exit code), success reported as task-success @ tokens.

use nanna_agent::harness::{LongHorizonConfig, LongHorizonRunner};
use nanna_daemon::llm_router::LlmRouter;
use nanna_daemon::tasks::{AgentStepRunner, TursoTaskSource, build_task_services};
use nanna_storage::{NewTask, Storage};
use std::sync::Arc;
use std::time::Duration;

const EVAL_SESSION: &str = "live-eval";

fn eval_model() -> String {
    std::env::var("NANNA_EVAL_MODEL").unwrap_or_else(|_| "qwen3.5:9b".to_string())
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

/// The live eval. Wall-clock bounded; every result is recorded by hand into
/// bench/BASELINE.md with the model + hardware named.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a running Ollama at localhost:11434"]
async fn live_task_success_at_tokens() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("info,nanna_agent=info,nanna_daemon=info")
        .try_init();

    let model = eval_model();
    let workspace = tempfile::tempdir().expect("tempdir");
    let workdir = workspace.path().to_path_buf();

    // Seed the fix-the-bug task's broken artifact.
    std::fs::write(
        workdir.join("calc.sh"),
        "#!/bin/sh\n# prints 2+2\necho 5\n",
    )
    .expect("seed calc.sh");

    // --- storage + plan ------------------------------------------------
    let storage = Arc::new(Storage::in_memory().await.expect("storage"));
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
    let plan_ids = [t1.id, t2.id, t3a.id, t3b.id, t4.id];
    let tasks_total = plan_ids.len();

    // --- tools ---------------------------------------------------------
    let tools = Arc::new(nanna_tools::ToolRegistry::new());
    tools.set_default_workdir(Some(workdir.clone())).await;
    tools.set_session_id(Some(EVAL_SESSION.to_string())).await;
    let tools_dir = nanna_tools::skills::defaults::resolve_tools_dir(None)
        .expect("DEV_TOOLS_DIR must resolve in debug builds");
    let workspace_id = Arc::new(tokio::sync::RwLock::new(None));
    let services = build_task_services(storage.clone(), workspace_id);
    let loaded = tools.load_skills_with_services(&tools_dir, &services).await;
    assert!(loaded > 0, "no skills loaded from {tools_dir:?}");

    // --- model ---------------------------------------------------------
    let router = Arc::new(LlmRouter::new().with_ollama("http://localhost:11434"));
    let runner = AgentStepRunner {
        router,
        tools,
        agent_config: nanna_agent::AgentConfig {
            model: model.clone(),
            max_tokens: 4096,
            temperature: 0.7,
            ..Default::default()
        },
        system_prompt: nanna_agent::prompts::DEFAULT_SYSTEM_PROMPT.to_string(),
        workspace_root: Some(workdir.clone()),
        stats: None,
    };
    let source = TursoTaskSource::new(
        storage.clone(),
        "session".to_string(),
        Some(EVAL_SESSION.to_string()),
        "harness".to_string(),
        None,
    );

    // --- run -----------------------------------------------------------
    let config = LongHorizonConfig {
        max_wall_clock: Duration::from_secs(15 * 60),
        max_replans_per_item: 1,
        ..LongHorizonConfig::default()
    };
    let started = std::time::Instant::now();
    let report = LongHorizonRunner::new(config)
        .run(
            "Complete every task in the plan. Each task produces a concrete file artifact \
             in the working directory that the harness verifies.",
            &source,
            &runner,
            &workdir,
            None,
        )
        .await;
    let elapsed = started.elapsed();

    // --- results -------------------------------------------------------
    let mut per_task = Vec::new();
    for id in plan_ids {
        let t = repo.get(id).await.expect("task");
        per_task.push(format!("  #{} [{}] {}", t.id, t.status, t.title));
        // Activity tail: the verdict evidence for post-mortems.
        for entry in repo.activity(id, 8).await.unwrap_or_default() {
            let detail = entry
                .detail
                .map(|d| d.to_string())
                .unwrap_or_default()
                .chars()
                .take(220)
                .collect::<String>();
            per_task.push(format!("      {} {}", entry.action, detail));
        }
    }
    println!("\n================ LIVE LONG-HORIZON EVAL ================");
    println!("model: {model}");
    println!("wall clock: {}s", elapsed.as_secs());
    println!("report: {}", serde_json::to_string_pretty(&report).unwrap());
    println!("tasks ({tasks_total} total):");
    for line in &per_task {
        println!("{line}");
    }
    println!("========================================================\n");

    // The eval records numbers; the only hard assertions are harness
    // integrity properties that must hold regardless of model quality.
    assert!(
        report.items_completed_unverified == 0,
        "every task has an acceptance check; no unverified completions possible"
    );
    let done = per_task.iter().filter(|l| l.contains("[done]")).count();
    assert_eq!(
        done, report.items_completed,
        "store state and report must agree on completions"
    );
}
