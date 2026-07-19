//! Long-horizon harness (P14): drive hours of agent work from the task store
//! with an O(1) re-anchored prompt per step.
//!
//! The design bet: *the agent should never need to remember; the harness makes
//! forgetting survivable.* Every step rebuilds a small prompt from the pinned
//! goal + the one actionable task + the last result — never a growing
//! transcript. Acceptance checks are machine-run by the harness (`done` is a
//! verdict, not an assertion), progress is measured by checks flipping, and a
//! stalled item is re-planned instead of ground on.
//!
//! The engine is pure orchestration over two traits — [`TaskSource`] (the P15
//! store) and [`StepRunner`] (a fresh-context agent per step) — so the whole
//! control loop is deterministically testable without a model.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub use crate::loop_runner::StepKind;

/// Default acceptance-check timeout.
///
/// Bound justification: a check runs between every step; one slower than the
/// step it verifies starves the loop. Two minutes covers a test-suite run on
/// the reference tier.
pub const ACCEPTANCE_TIMEOUT_SECS_DEFAULT: u64 = 120;

/// Hard ceiling on a single acceptance-check timeout.
///
/// Bound justification: a wedged verification command must never hang the run
/// — ten minutes is beyond any sane per-item check and keeps the loop live.
pub const ACCEPTANCE_TIMEOUT_SECS_MAX: u64 = 600;

/// Maximum bytes read from a file (or captured from a command) for a regex
/// acceptance check.
///
/// Bound justification: the harness loads the target into memory to match;
/// 4 MiB caps that memory and covers any log or report worth matching.
pub const ACCEPTANCE_READ_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Maximum bytes of a step's output fed forward into the next prompt and
/// recorded as a task note.
///
/// Bound justification: the re-anchored prompt is O(1) by construction — the
/// last result is one screenful; anything larger belongs in task notes and
/// memory, not the window.
pub const STEP_RESULT_TAIL_MAX_BYTES: usize = 2000;

// ---------------------------------------------------------------------------
// Acceptance checks
// ---------------------------------------------------------------------------

/// A machine-checkable done-condition, run by the harness — never the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcceptanceCheck {
    /// Passes when the command exits 0.
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
    /// Passes when the path exists (relative paths resolve in the workdir).
    FileExists { path: String },
    /// Passes when the pattern matches the file content (if `path`) or the
    /// combined output of `command`.
    Regex {
        pattern: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_secs: Option<u64>,
    },
}

/// The harness's verdict after running an acceptance check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceVerdict {
    pub passed: bool,
    /// Human/model-readable evidence (exit code, missing path, match info).
    pub detail: String,
}

impl AcceptanceCheck {
    /// Parse the store's acceptance JSON (`{kind: ..., ...}`).
    pub fn from_json(value: &serde_json::Value) -> Result<Self, String> {
        serde_json::from_value(value.clone()).map_err(|e| format!("invalid acceptance check: {e}"))
    }

    /// Short human-readable description for the step prompt.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Command { command, .. } => {
                format!("command `{command}` must exit 0")
            }
            Self::FileExists { path } => format!("file `{path}` must exist"),
            Self::Regex {
                pattern,
                path: Some(path),
                ..
            } => format!("file `{path}` must match /{pattern}/"),
            Self::Regex {
                pattern,
                command: Some(command),
                ..
            } => format!("output of `{command}` must match /{pattern}/"),
            Self::Regex { pattern, .. } => format!("output must match /{pattern}/"),
        }
    }

    fn effective_timeout(timeout_secs: Option<u64>) -> Duration {
        let secs = timeout_secs
            .unwrap_or(ACCEPTANCE_TIMEOUT_SECS_DEFAULT)
            .clamp(1, ACCEPTANCE_TIMEOUT_SECS_MAX);
        Duration::from_secs(secs)
    }

    /// Run the check against real environment state. This is deliberately the
    /// only place a "task is done" signal can originate when a check exists.
    pub async fn run(&self, workdir: &Path) -> AcceptanceVerdict {
        match self {
            Self::Command {
                command,
                timeout_secs,
            } => run_command_check(command, workdir, Self::effective_timeout(*timeout_secs)).await,
            Self::FileExists { path } => {
                let resolved = resolve_in_workdir(workdir, path);
                if resolved.exists() {
                    AcceptanceVerdict {
                        passed: true,
                        detail: format!("file exists: {}", resolved.display()),
                    }
                } else {
                    AcceptanceVerdict {
                        passed: false,
                        detail: format!("file does not exist: {}", resolved.display()),
                    }
                }
            }
            Self::Regex {
                pattern,
                path,
                command,
                timeout_secs,
            } => {
                let regex = match regex::Regex::new(pattern) {
                    Ok(r) => r,
                    Err(e) => {
                        return AcceptanceVerdict {
                            passed: false,
                            detail: format!("invalid regex /{pattern}/: {e}"),
                        };
                    }
                };
                let haystack = if let Some(path) = path {
                    let resolved = resolve_in_workdir(workdir, path);
                    match read_bounded(&resolved) {
                        Ok(content) => content,
                        Err(e) => {
                            return AcceptanceVerdict {
                                passed: false,
                                detail: format!("cannot read {}: {e}", resolved.display()),
                            };
                        }
                    }
                } else if let Some(command) = command {
                    let output =
                        run_shell(command, workdir, Self::effective_timeout(*timeout_secs)).await;
                    match output {
                        Ok((_, combined)) => combined,
                        Err(e) => {
                            return AcceptanceVerdict {
                                passed: false,
                                detail: format!("command failed: {e}"),
                            };
                        }
                    }
                } else {
                    return AcceptanceVerdict {
                        passed: false,
                        detail: "regex check has neither path nor command".to_string(),
                    };
                };
                if regex.is_match(&haystack) {
                    AcceptanceVerdict {
                        passed: true,
                        detail: format!("pattern /{pattern}/ matched"),
                    }
                } else {
                    AcceptanceVerdict {
                        passed: false,
                        detail: format!(
                            "pattern /{pattern}/ did not match ({} bytes searched)",
                            haystack.len()
                        ),
                    }
                }
            }
        }
    }
}

fn resolve_in_workdir(workdir: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workdir.join(p)
    }
}

fn read_bounded(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(ACCEPTANCE_READ_MAX_BYTES as u64)
        .read_to_end(&mut bytes)?;
    // Lossy: a log truncated mid-char (or with stray binary) must still be
    // matchable — an acceptance check failing on encoding would wedge runs.
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn run_command_check(command: &str, workdir: &Path, timeout: Duration) -> AcceptanceVerdict {
    match run_shell(command, workdir, timeout).await {
        Ok((code, combined)) => {
            let passed = code == Some(0);
            let tail: String = combined
                .chars()
                .rev()
                .take(400)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            AcceptanceVerdict {
                passed,
                detail: format!(
                    "`{command}` exited {} — {}",
                    code.map_or_else(|| "signal".to_string(), |c| c.to_string()),
                    tail.trim()
                ),
            }
        }
        Err(e) => AcceptanceVerdict {
            passed: false,
            detail: format!("`{command}` failed to run: {e}"),
        },
    }
}

/// Run a shell command, returning (exit code, combined stdout+stderr).
///
/// On Windows this prefers Git Bash `sh` when on PATH (matching the exec
/// tool's POSIX routing) and falls back to `cmd /C`.
///
/// Known limitation: on timeout, `kill_on_drop` kills the shell but not its
/// grandchildren (Windows has no process groups without Job Objects) — a
/// wedged workload can outlive the check. The timeout still keeps the run
/// loop live, which is the property that matters here.
async fn run_shell(
    command: &str,
    workdir: &Path,
    timeout: Duration,
) -> Result<(Option<i32>, String), String> {
    let mut cmd = shell_command(command);
    cmd.current_dir(workdir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| format!("timed out after {}s", timeout.as_secs()))?
        .map_err(|e| e.to_string())?;
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    if combined.len() > ACCEPTANCE_READ_MAX_BYTES {
        // Cut on a char boundary: String::truncate panics mid-char.
        let mut cut = ACCEPTANCE_READ_MAX_BYTES;
        while cut > 0 && !combined.is_char_boundary(cut) {
            cut -= 1;
        }
        combined.truncate(cut);
    }
    Ok((output.status.code(), combined))
}

/// Locate Git-for-Windows `bash.exe`, cached. Mirrors the exec tool's routing
/// in `nanna-scripting/src/bridge.rs` — explicitly NOT WSL's
/// `C:\Windows\System32\bash.exe` (different filesystem semantics).
#[cfg(windows)]
fn git_bash_path() -> Option<&'static Path> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let mut candidates: Vec<PathBuf> = Vec::new();
            for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                if let Ok(base) = std::env::var(var) {
                    candidates.push(PathBuf::from(base).join("Git").join("bin").join("bash.exe"));
                }
            }
            if let Ok(local) = std::env::var("LOCALAPPDATA") {
                candidates.push(
                    PathBuf::from(local)
                        .join("Programs")
                        .join("Git")
                        .join("bin")
                        .join("bash.exe"),
                );
            }
            candidates.into_iter().find(|p| p.is_file())
        })
        .as_deref()
}

#[cfg(windows)]
fn shell_command(command: &str) -> tokio::process::Command {
    // Acceptance commands are POSIX like every other command in this repo.
    // Route exactly like the exec tool: Git Bash first — a bare `sh` on PATH
    // is rare on Windows, and cmd cannot run `test`/`$(...)` at all, so a
    // silent cmd fallback makes POSIX checks unwinnable.
    if let Some(bash) = git_bash_path() {
        let mut cmd = tokio::process::Command::new(bash);
        cmd.arg("-c").arg(command);
        return cmd;
    }
    let sh_available = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join("sh.exe").exists()));
    if sh_available {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    } else {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    }
}

#[cfg(not(windows))]
fn shell_command(command: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}

// ---------------------------------------------------------------------------
// Task source + step runner traits
// ---------------------------------------------------------------------------

/// One actionable item as the harness sees it: exactly what fits in an O(1)
/// prompt — the task, its done-condition, its tool scope, and a bounded tail
/// of working notes.
#[derive(Debug, Clone)]
pub struct TaskStep {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub acceptance: Option<AcceptanceCheck>,
    /// Tool names to activate for this step (P14 per-item tool scoping).
    pub tool_scope: Vec<String>,
    /// Recent working notes, oldest first (bounded by the source).
    pub notes_tail: Vec<String>,
}

/// A subtask emitted by a replan step.
#[derive(Debug, Clone)]
pub struct NewSubtask {
    pub title: String,
    pub description: Option<String>,
    pub acceptance: Option<serde_json::Value>,
    pub tool_scope: Vec<String>,
}

/// The task store as the harness consumes it (implemented over the P15
/// `TaskRepository` in production, in memory in tests).
#[async_trait::async_trait]
pub trait TaskSource: Send + Sync {
    /// The one actionable item (unblocked, highest priority, leaf), or None
    /// when the plan is finished.
    async fn next(&self) -> Result<Option<TaskStep>, String>;
    /// Mark a step's item started (idempotent).
    async fn start(&self, id: i64) -> Result<(), String>;
    /// Record the completion verdict for an item.
    async fn complete(&self, id: i64, detail: serde_json::Value) -> Result<(), String>;
    /// Append a working note (the durable scratchpad).
    async fn add_note(&self, id: i64, content: &str) -> Result<(), String>;
    /// Record a harness event in the item's activity log.
    async fn log(&self, id: i64, action: &str, detail: serde_json::Value) -> Result<(), String>;
    /// Give up on an item after repeated failed replans — close it so the run
    /// can move on instead of grinding.
    async fn abandon(&self, id: i64, reason: &str) -> Result<(), String>;
}

/// Request for one step: everything the runner needs to build a fresh-context
/// agent run.
#[derive(Debug, Clone)]
pub struct StepRequest {
    pub item_id: i64,
    pub step_index: usize,
    pub step_kind: StepKind,
    pub prompt: String,
    /// Tools to activate for the step (≤ a handful — small models degrade
    /// past 5-10 tool definitions).
    pub tool_scope: Vec<String>,
    pub token_budget: Option<u64>,
    pub max_iterations: Option<usize>,
    pub max_wall_clock: Option<Duration>,
}

/// One tool call as seen from outside a step (digests, not payloads — the
/// parent's context must not grow when a child runs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepToolCall {
    pub name: String,
    pub input_digest: String,
    pub output_digest: String,
}

/// What came back from one step.
#[derive(Debug, Clone)]
pub struct StepOutcome {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub tool_calls: Vec<StepToolCall>,
}

/// Runs one re-anchored step in a fresh context (a new `Agent` + empty
/// `AgentContext` in production; scripted in tests).
#[async_trait::async_trait]
pub trait StepRunner: Send + Sync {
    async fn run_step(&self, request: StepRequest) -> Result<StepOutcome, String>;
}

// ---------------------------------------------------------------------------
// Config / report
// ---------------------------------------------------------------------------

/// Harness configuration. Every bound is per-run blast radius, not model
/// truncation: the outer loop continues across steps, so step bounds set the
/// re-anchor cadence rather than cutting work short.
#[derive(Debug, Clone)]
pub struct LongHorizonConfig {
    /// Per-run wall-clock ceiling. Default 4h — the flagship benchmark is a
    /// "4-hour task" (ROADMAP P14); callers override per run.
    pub max_wall_clock: Duration,
    /// Per-run total token ceiling (None = unbounded).
    pub max_total_tokens: Option<u64>,
    /// Steps on one item with no acceptance flip before a replan step.
    /// Progress-or-replan: 5 fruitless attempts is grinding, not working.
    pub max_steps_per_item: usize,
    /// Replans per item before the harness abandons it and moves on.
    pub max_replans_per_item: usize,
    /// Loop-iteration bound inside one step. The harness re-anchors every
    /// step, so iterations past the re-anchor window only grow context —
    /// which is exactly what P14 exists to prevent.
    pub step_iterations: usize,
    /// Token budget per step. A step that burns more than this has lost the
    /// O(1) property; forcing a step boundary re-anchors it.
    pub step_token_budget: Option<u64>,
    /// Consecutive runner errors before the run stops (circuit breaker for a
    /// dead model endpoint).
    pub max_consecutive_errors: usize,
    /// Actor name recorded in the activity log.
    pub actor: String,
}

impl Default for LongHorizonConfig {
    fn default() -> Self {
        Self {
            max_wall_clock: Duration::from_secs(4 * 3600),
            max_total_tokens: None,
            max_steps_per_item: 5,
            max_replans_per_item: 2,
            step_iterations: 8,
            step_token_budget: Some(20_000),
            max_consecutive_errors: 3,
            actor: "harness".to_string(),
        }
    }
}

/// Why the run stopped. Every exit produces a report — the harness analogue
/// of "always emit done:true on every exit".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum StopReason {
    /// `next()` returned None — the plan is finished.
    AllTasksDone,
    WallClockExhausted,
    TokenBudgetExhausted,
    Cancelled,
    /// The task source failed (storage error).
    SourceError {
        message: String,
    },
    /// Too many consecutive step-runner errors.
    RunnerErrors {
        message: String,
    },
}

/// Final report: the governing metric is `tokens_per_completed_item`
/// (task success @ tokens), not tokens per turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongHorizonReport {
    pub stop: StopReason,
    pub steps_taken: usize,
    pub items_completed: usize,
    /// Items completed on the model's claim alone (no acceptance check
    /// existed) — logged so unverified completions are visible.
    pub items_completed_unverified: usize,
    pub items_abandoned: usize,
    pub replans: usize,
    /// Completion claims that the acceptance check refuted (the
    /// "false success" counter — the P14 anti-drift keystone at work).
    pub false_success_claims: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_clock_secs: u64,
    /// total tokens / items completed. None when nothing completed.
    pub tokens_per_completed_item: Option<u64>,
}

// ---------------------------------------------------------------------------
// Prompt building (pure, byte-stable prefix)
// ---------------------------------------------------------------------------

/// Build the re-anchored step prompt.
///
/// Layout is deliberate and load-bearing:
/// - The **prefix** (goal + working rules) is byte-stable across every step
///   of a run — the shape KV-prefix caching rewards. The goal is pinned
///   verbatim and never summarized: everything else is compressible, intent
///   is not.
/// - The **dynamic tail** (current task, verdict, budget) comes last, in
///   recent attention where a small model actually looks.
#[must_use]
pub fn build_step_prompt(
    goal: &str,
    step: &TaskStep,
    last_result: Option<&str>,
    budget_line: &str,
) -> String {
    let mut prompt = stable_prefix(goal);
    prompt.push_str("== CURRENT TASK ==\n");
    prompt.push_str(&format!("Task #{}: {}\n", step.id, step.title));
    if let Some(description) = &step.description {
        if !description.is_empty() {
            prompt.push_str(description);
            prompt.push('\n');
        }
    }
    match &step.acceptance {
        Some(check) => {
            prompt.push_str(&format!(
                "Done when (checked by the harness, not by you): {}\n",
                check.describe()
            ));
        }
        None => {
            prompt.push_str(
                "No machine check exists for this task. When it is genuinely finished, say \
                 TASK COMPLETE on its own line.\n",
            );
        }
    }
    if !step.notes_tail.is_empty() {
        prompt.push_str("Working notes so far:\n");
        for note in &step.notes_tail {
            prompt.push_str(&format!("- {note}\n"));
        }
    }
    if let Some(last) = last_result {
        if !last.is_empty() {
            prompt.push_str(&format!("\n== LAST RESULT ==\n{last}\n"));
        }
    }
    prompt.push_str(&format!("\n{budget_line}\n"));
    prompt
}

/// Build the replan prompt: decompose a stalled item into subtasks *in the
/// task store* (via the todo tool) — no fragile text parsing of plans.
#[must_use]
pub fn build_replan_prompt(goal: &str, step: &TaskStep, stall_summary: &str) -> String {
    let mut prompt = stable_prefix(goal);
    prompt.push_str("== REPLAN REQUIRED ==\n");
    prompt.push_str(&format!(
        "Task #{}: {} has made no verifiable progress ({stall_summary}).\n",
        step.id, step.title
    ));
    prompt.push_str(&format!(
        "Break it into 2-5 smaller subtasks using the todo tool: call \
         todo(action='add', parent_id={}, title='...', acceptance={{...}}) once per subtask. \
         Each subtask needs a machine-checkable acceptance (kind: command | file_exists | regex) \
         wherever possible. Make the first subtask small enough to finish in one step. \
         Do not attempt the work itself in this step — only decompose.\n",
        step.id
    ));
    prompt
}

/// The byte-stable prompt prefix: system-adjacent rules + the immutable goal.
fn stable_prefix(goal: &str) -> String {
    format!(
        "== GOAL (immutable — this is the whole point of the run) ==\n{goal}\n\n\
         == HOW TO WORK ==\n\
         You are executing one step of a long-running plan. The plan lives in the todo \
         store — you do not need to remember anything between steps.\n\
         - Advance the CURRENT TASK below by exactly one concrete action.\n\
         - Use the tools provided; do not narrate actions you did not take.\n\
         - Leave findings for future steps with todo(action='note', ...): notes are your \
           only memory.\n\
         - Never mark work done yourself: the harness verifies the task's done-condition \
           and records completion.\n\n"
    )
}

/// One-line budget status the model can see — an agent that knows its budget
/// plans around it.
#[must_use]
pub fn budget_line(
    steps_taken: usize,
    items_completed: usize,
    tokens_used: u64,
    max_total_tokens: Option<u64>,
    elapsed: Duration,
    max_wall_clock: Duration,
) -> String {
    let token_part = max_total_tokens.map_or_else(
        || format!("{tokens_used} tokens used"),
        |max| format!("{tokens_used} of {max} tokens used"),
    );
    format!(
        "== BUDGET == step {}; {} items completed; {}; {} of {} minutes elapsed",
        steps_taken + 1,
        items_completed,
        token_part,
        elapsed.as_secs() / 60,
        max_wall_clock.as_secs() / 60
    )
}

/// Did the model claim the current task is finished?
///
/// Only honored for items *without* an acceptance check — with a check, the
/// environment is the only judge (false-success claims are the top documented
/// long-horizon failure mode).
#[must_use]
pub fn step_claims_completion(text: &str) -> bool {
    text.lines()
        .any(|line| line.trim().eq_ignore_ascii_case("TASK COMPLETE"))
}

/// Cross-step repetition: two consecutive steps on the same item made the
/// same non-empty tool-call sequence with the same results — semantically a
/// stall even when the narration varies.
#[must_use]
pub fn steps_repeat(previous: &[StepToolCall], current: &[StepToolCall]) -> bool {
    !current.is_empty() && previous == current
}

// ---------------------------------------------------------------------------
// The runner
// ---------------------------------------------------------------------------

/// Per-item progress bookkeeping.
#[derive(Debug, Default, Clone)]
struct ItemProgress {
    steps_without_progress: usize,
    replans: usize,
    tokens_spent: u64,
    last_result: Option<String>,
    last_tool_calls: Vec<StepToolCall>,
}

/// The long-horizon control loop.
pub struct LongHorizonRunner {
    pub config: LongHorizonConfig,
}

impl LongHorizonRunner {
    #[must_use]
    pub const fn new(config: LongHorizonConfig) -> Self {
        Self { config }
    }

    /// Drive the plan to completion (or to a budget/error stop). Every exit
    /// path returns a report.
    pub async fn run(
        &self,
        goal: &str,
        source: &dyn TaskSource,
        runner: &dyn StepRunner,
        workdir: &Path,
        cancel: Option<Arc<AtomicBool>>,
    ) -> LongHorizonReport {
        let cfg = &self.config;
        let started = Instant::now();
        let mut steps_taken = 0usize;
        let mut items_completed = 0usize;
        let mut items_completed_unverified = 0usize;
        let mut items_abandoned = 0usize;
        let mut replans = 0usize;
        let mut false_success_claims = 0usize;
        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;
        let mut consecutive_errors = 0usize;
        let mut progress: HashMap<i64, ItemProgress> = HashMap::new();

        let stop = loop {
            if cancel
                .as_ref()
                .is_some_and(|flag| flag.load(Ordering::Relaxed))
            {
                break StopReason::Cancelled;
            }
            if started.elapsed() >= cfg.max_wall_clock {
                break StopReason::WallClockExhausted;
            }
            let tokens_used = input_tokens + output_tokens;
            if cfg.max_total_tokens.is_some_and(|max| tokens_used >= max) {
                break StopReason::TokenBudgetExhausted;
            }

            // Re-anchor: the store, not the transcript, is the state.
            let step = match source.next().await {
                Ok(Some(step)) => step,
                Ok(None) => break StopReason::AllTasksDone,
                Err(message) => break StopReason::SourceError { message },
            };
            let item = progress.entry(step.id).or_default();
            let is_replan = item.steps_without_progress >= cfg.max_steps_per_item;

            if is_replan && item.replans >= cfg.max_replans_per_item {
                // Grinding AND replanning failed — close the item and move on.
                let reason = format!(
                    "abandoned after {} fruitless steps and {} replans",
                    item.steps_without_progress, item.replans
                );
                if let Err(message) = source.abandon(step.id, &reason).await {
                    break StopReason::SourceError { message };
                }
                items_abandoned += 1;
                progress.remove(&step.id);
                continue;
            }

            let _ = source.start(step.id).await;

            let (prompt, step_kind) = if is_replan {
                let stall_summary = format!(
                    "{} steps without the done-condition flipping",
                    item.steps_without_progress
                );
                (
                    build_replan_prompt(goal, &step, &stall_summary),
                    StepKind::Plan,
                )
            } else {
                let line = budget_line(
                    steps_taken,
                    items_completed,
                    tokens_used,
                    cfg.max_total_tokens,
                    started.elapsed(),
                    cfg.max_wall_clock,
                );
                (
                    build_step_prompt(goal, &step, item.last_result.as_deref(), &line),
                    StepKind::Execute,
                )
            };

            let remaining_wall = cfg.max_wall_clock.saturating_sub(started.elapsed());
            let request = StepRequest {
                item_id: step.id,
                step_index: steps_taken,
                step_kind,
                prompt,
                tool_scope: step.tool_scope.clone(),
                token_budget: cfg.step_token_budget,
                max_iterations: Some(cfg.step_iterations),
                max_wall_clock: Some(remaining_wall),
            };

            let outcome = match runner.run_step(request).await {
                Ok(outcome) => {
                    consecutive_errors = 0;
                    outcome
                }
                Err(message) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= cfg.max_consecutive_errors {
                        break StopReason::RunnerErrors { message };
                    }
                    continue;
                }
            };
            steps_taken += 1;
            input_tokens += outcome.input_tokens;
            output_tokens += outcome.output_tokens;

            let item = progress.entry(step.id).or_default();
            item.tokens_spent += outcome.input_tokens + outcome.output_tokens;

            if is_replan {
                // The replan step adds subtasks through the store; the next
                // next() will surface them. Reset the grind counter so the
                // new decomposition gets a fresh allowance.
                item.replans += 1;
                item.steps_without_progress = 0;
                item.last_result = None;
                item.last_tool_calls.clear();
                replans += 1;
                let _ = source
                    .log(
                        step.id,
                        "replanned",
                        serde_json::json!({ "replans": item.replans }),
                    )
                    .await;
                continue;
            }

            // Leave the step's findings in the store (the durable scratchpad),
            // not in any transcript.
            let tail = text_tail(&outcome.text, STEP_RESULT_TAIL_MAX_BYTES);
            if !tail.is_empty() {
                let _ = source.add_note(step.id, &tail).await;
            }

            // Cross-step repetition = a stall the in-run detector cannot see.
            let repeated = steps_repeat(&item.last_tool_calls, &outcome.tool_calls);
            item.last_tool_calls.clone_from(&outcome.tool_calls);

            // Verdict time. With a check, the environment is the only judge.
            match &step.acceptance {
                Some(check) => {
                    let verdict = check.run(workdir).await;
                    let _ = source
                        .log(
                            step.id,
                            "acceptance_checked",
                            serde_json::json!({
                                "passed": verdict.passed,
                                "detail": verdict.detail,
                            }),
                        )
                        .await;
                    if verdict.passed {
                        let detail = serde_json::json!({
                            "verified": true,
                            "verdict": verdict.detail,
                            "tokens_spent": item.tokens_spent,
                        });
                        match source.complete(step.id, detail).await {
                            Ok(()) => {
                                consecutive_errors = 0;
                                items_completed += 1;
                                progress.remove(&step.id);
                            }
                            Err(message) => {
                                // A completion can legitimately fail (e.g. a
                                // concurrent decomposition opened a child).
                                // Retry via next() instead of killing the run.
                                let _ = source
                                    .log(
                                        step.id,
                                        "complete_failed",
                                        serde_json::json!({ "error": message }),
                                    )
                                    .await;
                                consecutive_errors += 1;
                                if consecutive_errors >= cfg.max_consecutive_errors {
                                    break StopReason::SourceError { message };
                                }
                            }
                        }
                    } else {
                        if step_claims_completion(&outcome.text) {
                            // The model said done; the environment disagrees.
                            false_success_claims += 1;
                            let _ = source
                                .log(
                                    step.id,
                                    "false_success_claim",
                                    serde_json::json!({ "verdict": verdict.detail }),
                                )
                                .await;
                        }
                        item.steps_without_progress += 1;
                        if repeated {
                            item.steps_without_progress += 1;
                        }
                        item.last_result = Some(format!(
                            "Done-condition NOT met: {}{}",
                            verdict.detail,
                            if repeated {
                                " (you repeated the exact same tool calls as last step — \
                                 change approach)"
                            } else {
                                ""
                            }
                        ));
                    }
                }
                None => {
                    if step_claims_completion(&outcome.text) {
                        let detail = serde_json::json!({
                            "verified": false,
                            "tokens_spent": item.tokens_spent,
                        });
                        match source.complete(step.id, detail).await {
                            Ok(()) => {
                                consecutive_errors = 0;
                                items_completed += 1;
                                items_completed_unverified += 1;
                                let _ = source
                                    .log(step.id, "completed_unverified", serde_json::Value::Null)
                                    .await;
                                progress.remove(&step.id);
                            }
                            Err(message) => {
                                let _ = source
                                    .log(
                                        step.id,
                                        "complete_failed",
                                        serde_json::json!({ "error": message }),
                                    )
                                    .await;
                                consecutive_errors += 1;
                                if consecutive_errors >= cfg.max_consecutive_errors {
                                    break StopReason::SourceError { message };
                                }
                            }
                        }
                    } else {
                        item.steps_without_progress += 1;
                        if repeated {
                            item.steps_without_progress += 1;
                        }
                        item.last_result =
                            Some(text_tail(&outcome.text, STEP_RESULT_TAIL_MAX_BYTES));
                    }
                }
            }
        };

        let total_tokens = input_tokens + output_tokens;
        LongHorizonReport {
            stop,
            steps_taken,
            items_completed,
            items_completed_unverified,
            items_abandoned,
            replans,
            false_success_claims,
            input_tokens,
            output_tokens,
            wall_clock_secs: started.elapsed().as_secs(),
            tokens_per_completed_item: if items_completed > 0 {
                Some(total_tokens / items_completed as u64)
            } else {
                None
            },
        }
    }
}

/// Last `max_bytes` of `text`, on a char boundary, trimmed.
fn text_tail(text: &str, max_bytes: usize) -> String {
    let trimmed = text.trim();
    if trimmed.len() <= max_bytes {
        return trimmed.to_string();
    }
    let mut start = trimmed.len() - max_bytes;
    while !trimmed.is_char_boundary(start) {
        start += 1;
    }
    trimmed[start..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use tokio::sync::Mutex;

    // -----------------------------------------------------------------
    // In-memory task source + scripted step runners
    // -----------------------------------------------------------------

    #[derive(Debug, Clone)]
    struct MemItem {
        step: TaskStep,
        done: bool,
        abandoned: bool,
    }

    #[derive(Default)]
    struct MemorySource {
        items: Mutex<Vec<MemItem>>,
        completions: Mutex<Vec<(i64, serde_json::Value)>>,
        notes: Mutex<Vec<(i64, String)>>,
        log_entries: Mutex<Vec<(i64, String)>>,
        fail_next: Mutex<bool>,
    }

    impl MemorySource {
        async fn push(&self, step: TaskStep) {
            self.items.lock().await.push(MemItem {
                step,
                done: false,
                abandoned: false,
            });
        }
    }

    #[async_trait::async_trait]
    impl TaskSource for MemorySource {
        async fn next(&self) -> Result<Option<TaskStep>, String> {
            if *self.fail_next.lock().await {
                return Err("storage exploded".to_string());
            }
            Ok(self
                .items
                .lock()
                .await
                .iter()
                .find(|i| !i.done && !i.abandoned)
                .map(|i| i.step.clone()))
        }
        async fn start(&self, _id: i64) -> Result<(), String> {
            Ok(())
        }
        async fn complete(&self, id: i64, detail: serde_json::Value) -> Result<(), String> {
            let mut items = self.items.lock().await;
            if let Some(item) = items.iter_mut().find(|i| i.step.id == id) {
                item.done = true;
            }
            self.completions.lock().await.push((id, detail));
            Ok(())
        }
        async fn add_note(&self, id: i64, content: &str) -> Result<(), String> {
            self.notes.lock().await.push((id, content.to_string()));
            Ok(())
        }
        async fn log(
            &self,
            id: i64,
            action: &str,
            _detail: serde_json::Value,
        ) -> Result<(), String> {
            self.log_entries.lock().await.push((id, action.to_string()));
            Ok(())
        }
        async fn abandon(&self, id: i64, _reason: &str) -> Result<(), String> {
            let mut items = self.items.lock().await;
            if let Some(item) = items.iter_mut().find(|i| i.step.id == id) {
                item.abandoned = true;
            }
            Ok(())
        }
    }

    /// Replays a fixed script of outcomes; captures every request.
    struct ScriptedRunner {
        script: Mutex<VecDeque<Result<StepOutcome, String>>>,
        requests: Mutex<Vec<StepRequest>>,
    }

    impl ScriptedRunner {
        fn new(script: Vec<Result<StepOutcome, String>>) -> Self {
            Self {
                script: Mutex::new(script.into()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl StepRunner for ScriptedRunner {
        async fn run_step(&self, request: StepRequest) -> Result<StepOutcome, String> {
            self.requests.lock().await.push(request);
            self.script
                .lock()
                .await
                .pop_front()
                .unwrap_or_else(|| Err("script exhausted".to_string()))
        }
    }

    fn outcome(text: &str) -> StepOutcome {
        StepOutcome {
            text: text.to_string(),
            input_tokens: 1000,
            output_tokens: 200,
            tool_calls: vec![],
        }
    }

    fn step(id: i64, title: &str, acceptance: Option<AcceptanceCheck>) -> TaskStep {
        TaskStep {
            id,
            title: title.to_string(),
            description: None,
            acceptance,
            tool_scope: vec!["exec".to_string()],
            notes_tail: vec![],
        }
    }

    fn fast_config() -> LongHorizonConfig {
        LongHorizonConfig {
            max_steps_per_item: 2,
            max_replans_per_item: 1,
            ..LongHorizonConfig::default()
        }
    }

    // -----------------------------------------------------------------
    // Acceptance checks
    // -----------------------------------------------------------------

    #[test]
    fn acceptance_parses_all_kinds_and_rejects_unknown() {
        let cmd = AcceptanceCheck::from_json(
            &serde_json::json!({"kind": "command", "command": "cargo test"}),
        )
        .unwrap();
        assert!(matches!(cmd, AcceptanceCheck::Command { .. }));
        let file = AcceptanceCheck::from_json(
            &serde_json::json!({"kind": "file_exists", "path": "out.txt"}),
        )
        .unwrap();
        assert!(matches!(file, AcceptanceCheck::FileExists { .. }));
        let re = AcceptanceCheck::from_json(
            &serde_json::json!({"kind": "regex", "pattern": "ok", "path": "log.txt"}),
        )
        .unwrap();
        assert!(matches!(re, AcceptanceCheck::Regex { .. }));
        assert!(AcceptanceCheck::from_json(&serde_json::json!({"kind": "vibes"})).is_err());
    }

    #[tokio::test]
    async fn file_exists_check_reflects_real_filesystem_state() {
        let dir = tempfile::tempdir().unwrap();
        let check = AcceptanceCheck::FileExists {
            path: "artifact.txt".to_string(),
        };
        let verdict = check.run(dir.path()).await;
        assert!(
            !verdict.passed,
            "missing file must fail: {}",
            verdict.detail
        );

        std::fs::write(dir.path().join("artifact.txt"), "x").unwrap();
        let verdict = check.run(dir.path()).await;
        assert!(
            verdict.passed,
            "existing file must pass: {}",
            verdict.detail
        );
    }

    #[tokio::test]
    async fn regex_check_matches_file_content() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("log.txt"), "42 tests passed, 0 failed").unwrap();
        let check = AcceptanceCheck::Regex {
            pattern: r"\d+ tests passed, 0 failed".to_string(),
            path: Some("log.txt".to_string()),
            command: None,
            timeout_secs: None,
        };
        assert!(check.run(dir.path()).await.passed);

        let no_match = AcceptanceCheck::Regex {
            pattern: "impossible-marker".to_string(),
            path: Some("log.txt".to_string()),
            command: None,
            timeout_secs: None,
        };
        assert!(!no_match.run(dir.path()).await.passed);
    }

    #[tokio::test]
    async fn regex_check_fails_cleanly_on_invalid_pattern_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let bad = AcceptanceCheck::Regex {
            pattern: "([unclosed".to_string(),
            path: Some("log.txt".to_string()),
            command: None,
            timeout_secs: None,
        };
        let verdict = bad.run(dir.path()).await;
        assert!(!verdict.passed);
        assert!(
            verdict.detail.contains("invalid regex"),
            "{}",
            verdict.detail
        );

        let missing = AcceptanceCheck::Regex {
            pattern: "x".to_string(),
            path: Some("nope.txt".to_string()),
            command: None,
            timeout_secs: None,
        };
        assert!(!missing.run(dir.path()).await.passed);
    }

    #[tokio::test]
    async fn command_check_uses_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        // `exit 0` / `exit 1` work under both sh and cmd.
        let pass = AcceptanceCheck::Command {
            command: "exit 0".to_string(),
            timeout_secs: None,
        };
        assert!(pass.run(dir.path()).await.passed);
        let fail = AcceptanceCheck::Command {
            command: "exit 1".to_string(),
            timeout_secs: None,
        };
        assert!(!fail.run(dir.path()).await.passed);
    }

    #[tokio::test]
    async fn command_check_runs_posix_syntax() {
        // Regression from the first live eval: `test`/`$(...)` checks were
        // silently unwinnable when the shell fell back to cmd.exe because a
        // bare `sh` was not on PATH. The runner must route through Git Bash.
        #[cfg(windows)]
        if git_bash_path().is_none() {
            eprintln!("skipping command_check_runs_posix_syntax: Git Bash not installed");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let pass = AcceptanceCheck::Command {
            command: "test \"$(echo 4)\" = \"4\"".to_string(),
            timeout_secs: None,
        };
        let verdict = pass.run(dir.path()).await;
        assert!(verdict.passed, "POSIX check must pass: {}", verdict.detail);
        let fail = AcceptanceCheck::Command {
            command: "test \"$(echo 5)\" = \"4\"".to_string(),
            timeout_secs: None,
        };
        assert!(!fail.run(dir.path()).await.passed);
    }

    #[test]
    fn acceptance_timeout_is_clamped_to_ceiling_and_floor() {
        assert_eq!(
            AcceptanceCheck::effective_timeout(Some(9999)),
            Duration::from_secs(ACCEPTANCE_TIMEOUT_SECS_MAX)
        );
        assert_eq!(
            AcceptanceCheck::effective_timeout(Some(0)),
            Duration::from_secs(1)
        );
        assert_eq!(
            AcceptanceCheck::effective_timeout(None),
            Duration::from_secs(ACCEPTANCE_TIMEOUT_SECS_DEFAULT)
        );
    }

    // -----------------------------------------------------------------
    // Prompt building
    // -----------------------------------------------------------------

    #[test]
    fn step_prompt_prefix_is_byte_stable_across_steps() {
        let goal = "Refactor the parser crate without breaking the test suite";
        let a = build_step_prompt(goal, &step(1, "first", None), None, "== BUDGET == x");
        let b = build_step_prompt(
            goal,
            &step(2, "second", None),
            Some("previous result"),
            "== BUDGET == y",
        );
        let marker = "== CURRENT TASK ==";
        let prefix_a = &a[..a.find(marker).unwrap()];
        let prefix_b = &b[..b.find(marker).unwrap()];
        assert_eq!(
            prefix_a, prefix_b,
            "the prompt prefix must never move — KV-prefix caching depends on it"
        );
    }

    #[test]
    fn step_prompt_pins_goal_verbatim_and_puts_dynamics_last() {
        let goal = "Ship the exact goal text, unsummarized & unmodified.";
        let prompt = build_step_prompt(
            goal,
            &step(7, "do the thing", None),
            Some("last output"),
            "== BUDGET == step 3",
        );
        assert!(prompt.contains(goal), "goal must appear verbatim");
        let goal_pos = prompt.find(goal).unwrap();
        let task_pos = prompt.find("Task #7").unwrap();
        let last_pos = prompt.find("last output").unwrap();
        let budget_pos = prompt.find("== BUDGET ==").unwrap();
        assert!(goal_pos < task_pos && task_pos < last_pos && last_pos < budget_pos);
    }

    #[test]
    fn step_prompt_describes_acceptance_when_present() {
        let check = AcceptanceCheck::Command {
            command: "cargo test".to_string(),
            timeout_secs: None,
        };
        let with = build_step_prompt("g", &step(1, "t", Some(check)), None, "b");
        assert!(with.contains("cargo test"));
        assert!(with.contains("checked by the harness"));
        let without = build_step_prompt("g", &step(1, "t", None), None, "b");
        assert!(without.contains("TASK COMPLETE"));
    }

    #[test]
    fn completion_claim_requires_marker_on_own_line() {
        assert!(step_claims_completion("did the work\nTASK COMPLETE\n"));
        assert!(step_claims_completion("  task complete  "));
        assert!(!step_claims_completion("the task completes eventually"));
        assert!(!step_claims_completion("almost TASK COMPLETE but inline"));
    }

    #[test]
    fn steps_repeat_requires_identical_nonempty_sequences() {
        let call = StepToolCall {
            name: "exec".to_string(),
            input_digest: "a".to_string(),
            output_digest: "b".to_string(),
        };
        assert!(steps_repeat(
            std::slice::from_ref(&call),
            std::slice::from_ref(&call)
        ));
        assert!(
            !steps_repeat(&[], &[]),
            "empty sequences are not a loop signal"
        );
        let other = StepToolCall {
            output_digest: "c".to_string(),
            ..call.clone()
        };
        assert!(!steps_repeat(&[call], &[other]));
    }

    #[test]
    fn text_tail_bounds_and_respects_char_boundaries() {
        assert_eq!(text_tail("short", 100), "short");
        let long = format!("{}end", "x".repeat(5000));
        let tail = text_tail(&long, 100);
        assert!(tail.len() <= 100);
        assert!(tail.ends_with("end"));
        // Multi-byte chars must not be split.
        let emoji = "🌀".repeat(100);
        let tail = text_tail(&emoji, 10);
        assert!(tail.chars().all(|c| c == '🌀'));
    }

    // -----------------------------------------------------------------
    // Control loop
    // -----------------------------------------------------------------

    #[tokio::test]
    async fn empty_plan_stops_immediately_with_all_done() {
        let source = MemorySource::default();
        let runner = ScriptedRunner::new(vec![]);
        let dir = tempfile::tempdir().unwrap();
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.stop, StopReason::AllTasksDone);
        assert_eq!(report.steps_taken, 0);
        assert_eq!(report.tokens_per_completed_item, None);
    }

    #[tokio::test]
    async fn verified_completion_requires_the_acceptance_check_to_pass() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "produce artifact",
                Some(AcceptanceCheck::FileExists {
                    path: "artifact.txt".to_string(),
                }),
            ))
            .await;
        // Step 1: model works but produces nothing. Step 2: file appears.
        let artifact = dir.path().join("artifact.txt");
        struct Producer {
            artifact: PathBuf,
            calls: Mutex<usize>,
        }
        #[async_trait::async_trait]
        impl StepRunner for Producer {
            async fn run_step(&self, _request: StepRequest) -> Result<StepOutcome, String> {
                let mut calls = self.calls.lock().await;
                *calls += 1;
                if *calls == 2 {
                    std::fs::write(&self.artifact, "done").unwrap();
                }
                Ok(StepOutcome {
                    text: "worked on it".to_string(),
                    input_tokens: 1000,
                    output_tokens: 200,
                    tool_calls: vec![],
                })
            }
        }
        let runner = Producer {
            artifact,
            calls: Mutex::new(0),
        };
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.stop, StopReason::AllTasksDone);
        assert_eq!(report.items_completed, 1);
        assert_eq!(report.steps_taken, 2);
        assert_eq!(report.items_completed_unverified, 0);
        let completions = source.completions.lock().await;
        assert_eq!(completions.len(), 1);
        assert_eq!(completions[0].1["verified"], serde_json::json!(true));
    }

    #[tokio::test]
    async fn unchecked_item_completes_on_claim_but_is_flagged_unverified() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "fuzzy item", None)).await;
        let runner = ScriptedRunner::new(vec![Ok(outcome("all wrapped up\nTASK COMPLETE"))]);
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.items_completed, 1);
        assert_eq!(report.items_completed_unverified, 1);
        let log = source.log_entries.lock().await;
        assert!(log.iter().any(|(_, a)| a == "completed_unverified"));
    }

    #[tokio::test]
    async fn false_success_claim_is_refuted_replanned_then_abandoned() {
        // Suite 4 fixture: the model *claims* completion every step, the
        // environment never changes. The harness must never record a
        // completion — this is the anti-drift keystone.
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "impossible item",
                Some(AcceptanceCheck::FileExists {
                    path: "never-created.txt".to_string(),
                }),
            ))
            .await;
        let claim = || Ok(outcome("I finished!\nTASK COMPLETE"));
        let runner = ScriptedRunner::new(vec![claim(), claim(), claim(), claim(), claim()]);
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(
            report.stop,
            StopReason::AllTasksDone,
            "abandoned ⇒ plan drains"
        );
        assert_eq!(
            report.items_completed, 0,
            "false success must never complete"
        );
        assert_eq!(report.items_abandoned, 1);
        assert!(
            report.false_success_claims >= 2,
            "claims were counted: {report:?}"
        );
        assert_eq!(report.replans, 1);
        // 2 execute steps -> replan -> 2 more execute steps -> abandon = 5 steps max
        assert!(
            report.steps_taken <= 5,
            "grinding must be bounded: {report:?}"
        );
    }

    #[tokio::test]
    async fn replan_step_uses_plan_kind_and_replan_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "stubborn item",
                Some(AcceptanceCheck::FileExists {
                    path: "never.txt".to_string(),
                }),
            ))
            .await;
        let runner = ScriptedRunner::new(vec![
            Ok(outcome("try 1")),
            Ok(outcome("try 2")),
            Ok(outcome("decomposed")),
            Ok(outcome("try 3")),
            Ok(outcome("try 4")),
        ]);
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.replans, 1);
        let requests = runner.requests.lock().await;
        let plan_steps: Vec<_> = requests
            .iter()
            .filter(|r| r.step_kind == StepKind::Plan)
            .collect();
        assert_eq!(plan_steps.len(), 1, "exactly one replan step");
        assert!(plan_steps[0].prompt.contains("== REPLAN REQUIRED =="));
        assert!(plan_steps[0].prompt.contains("parent_id=1"));
        // Execute steps carry the item's tool scope and step bounds.
        let exec = requests
            .iter()
            .find(|r| r.step_kind == StepKind::Execute)
            .unwrap();
        assert_eq!(exec.tool_scope, vec!["exec".to_string()]);
        assert_eq!(exec.max_iterations, Some(fast_config().step_iterations));
    }

    #[tokio::test]
    async fn repeated_tool_signatures_accelerate_the_stall_counter() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "loopy item",
                Some(AcceptanceCheck::FileExists {
                    path: "never.txt".to_string(),
                }),
            ))
            .await;
        let looped = || {
            Ok(StepOutcome {
                text: "trying the same thing".to_string(),
                input_tokens: 1000,
                output_tokens: 200,
                tool_calls: vec![StepToolCall {
                    name: "exec".to_string(),
                    input_digest: "same".to_string(),
                    output_digest: "same".to_string(),
                }],
            })
        };
        let runner = ScriptedRunner::new(vec![looped(), looped(), looped(), looped()]);
        let config = LongHorizonConfig {
            max_steps_per_item: 4,
            max_replans_per_item: 0,
            ..LongHorizonConfig::default()
        };
        let report = LongHorizonRunner::new(config)
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        // Identical signatures double the stall increment: 1 + 2 + 2 = 5 ≥ 4
        // after 3 steps, instead of 4 — the loop is cut short.
        assert_eq!(report.items_abandoned, 1);
        assert!(
            report.steps_taken < 4,
            "repetition must accelerate abandonment: {report:?}"
        );
    }

    #[tokio::test]
    async fn wall_clock_budget_stops_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "any", None)).await;
        let runner = ScriptedRunner::new(vec![]);
        let config = LongHorizonConfig {
            max_wall_clock: Duration::ZERO,
            ..LongHorizonConfig::default()
        };
        let report = LongHorizonRunner::new(config)
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.stop, StopReason::WallClockExhausted);
        assert_eq!(report.steps_taken, 0);
    }

    #[tokio::test]
    async fn token_budget_stops_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "endless", None)).await;
        let runner = ScriptedRunner::new(vec![
            Ok(outcome("no claim")),
            Ok(outcome("no claim")),
            Ok(outcome("no claim")),
        ]);
        let config = LongHorizonConfig {
            max_total_tokens: Some(1500),
            max_steps_per_item: 100,
            ..LongHorizonConfig::default()
        };
        let report = LongHorizonRunner::new(config)
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.stop, StopReason::TokenBudgetExhausted);
        assert_eq!(
            report.steps_taken, 2,
            "1200 tokens after step 1 < 1500; stop after step 2"
        );
    }

    #[tokio::test]
    async fn cancellation_flag_stops_before_the_next_step() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "any", None)).await;
        let runner = ScriptedRunner::new(vec![]);
        let cancel = Arc::new(AtomicBool::new(true));
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), Some(cancel))
            .await;
        assert_eq!(report.stop, StopReason::Cancelled);
        assert_eq!(report.steps_taken, 0);
    }

    #[tokio::test]
    async fn consecutive_runner_errors_trip_the_circuit_breaker() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "any", None)).await;
        let runner = ScriptedRunner::new(vec![
            Err("boom".to_string()),
            Err("boom".to_string()),
            Err("boom".to_string()),
        ]);
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert!(matches!(report.stop, StopReason::RunnerErrors { .. }));
        assert_eq!(report.steps_taken, 0, "failed steps are not progress");
    }

    #[tokio::test]
    async fn source_error_stops_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        *source.fail_next.lock().await = true;
        let runner = ScriptedRunner::new(vec![]);
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert!(matches!(report.stop, StopReason::SourceError { .. }));
    }

    #[tokio::test]
    async fn step_findings_are_written_to_task_notes() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "note me", None)).await;
        let runner = ScriptedRunner::new(vec![Ok(outcome(
            "found the config in crates/nanna-config\nTASK COMPLETE",
        ))]);
        let _ = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        let notes = source.notes.lock().await;
        assert_eq!(notes.len(), 1);
        assert!(notes[0].1.contains("nanna-config"));
    }

    // -----------------------------------------------------------------
    // Suite 4 benchmark fixtures (deterministic; cited by bench/BASELINE.md)
    // -----------------------------------------------------------------

    /// Deterministic task-success @ tokens for a fully compliant scripted
    /// model: 3 items, 1 step each at 1200 tokens ⇒ 3600 total, 1200/item.
    #[tokio::test]
    async fn compliant_run_success_at_tokens_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        for id in 1..=3 {
            source.push(step(id, "item", None)).await;
        }
        let runner = ScriptedRunner::new(vec![
            Ok(outcome("TASK COMPLETE")),
            Ok(outcome("TASK COMPLETE")),
            Ok(outcome("TASK COMPLETE")),
        ]);
        let report = LongHorizonRunner::new(LongHorizonConfig::default())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.stop, StopReason::AllTasksDone);
        assert_eq!(report.items_completed, 3);
        assert_eq!(report.input_tokens + report.output_tokens, 3600);
        assert_eq!(report.tokens_per_completed_item, Some(1200));
    }

    /// Deterministic drift containment: a permanently-false-claiming model
    /// spends at most 6000 tokens (5 steps) before its item is closed, and
    /// records zero completions.
    #[tokio::test]
    async fn drift_containment_cost_baseline() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "impossible",
                Some(AcceptanceCheck::FileExists {
                    path: "never.txt".to_string(),
                }),
            ))
            .await;
        let claim = || Ok(outcome("TASK COMPLETE"));
        let runner =
            ScriptedRunner::new(vec![claim(), claim(), claim(), claim(), claim(), claim()]);
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.items_completed, 0);
        assert_eq!(report.items_abandoned, 1);
        assert!(
            report.input_tokens + report.output_tokens <= 6000,
            "drift cost must be bounded: {report:?}"
        );
    }
}
