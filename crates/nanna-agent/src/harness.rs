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
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use crate::cancel::CancelToken;
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

    /// The shell command the model can run itself to see the same verdict the
    /// harness will apply, or `None` when the check is pure inspection with
    /// no command to run (a bare file/regex test the model can just look at).
    ///
    /// Deliberately NOT synthesized for `FileExists`/path-`Regex`: inventing
    /// a `test -f`/`grep` line would put words in the check's mouth, and the
    /// description already states the condition plainly.
    #[must_use]
    pub fn self_check_command(&self) -> Option<&str> {
        match self {
            Self::Command { command, .. } => Some(command),
            Self::Regex {
                command: Some(command),
                ..
            } => Some(command),
            Self::FileExists { .. } | Self::Regex { .. } => None,
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

/// Kill a process and its descendants, best-effort. Mirrors
/// `kill_process_tree` in `nanna-scripting/src/bridge.rs` (this crate can't
/// reach it — nanna-scripting sits behind nanna-tools' optional `scripting`
/// feature); keep the two in sync.
async fn kill_process_tree(pid: u32) {
    #[cfg(windows)]
    {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
    }
    #[cfg(not(windows))]
    {
        // pid is a fresh process-group id from the kernel: it fits i32, and a
        // failed signal (group already gone) is exactly the no-op we want.
        #[allow(clippy::cast_possible_wrap)]
        let pgid = -(pid as i32);
        // SAFETY: kill(2) with a negative pgid signals a process group we
        // created; `process_group(0)` at spawn put the child in its own
        // group, so this can never reach our own.
        unsafe {
            libc::kill(pgid, libc::SIGKILL);
        }
    }
}

/// Run a shell command, returning (exit code, combined stdout+stderr).
///
/// On Windows this prefers Git Bash `sh` when on PATH (matching the exec
/// tool's POSIX routing) and falls back to `cmd /C`.
///
/// Lifecycle matches the exec tool (`bridge.rs`): the child runs in its own
/// process group so a check that signals its group can't reach us, and a
/// timeout kills the whole tree — the shell *and* any wedged grandchild —
/// instead of `kill_on_drop`'s shell-only reap, which let a stuck workload
/// outlive the check and hold workspace/build locks.
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
    #[cfg(windows)]
    {
        // Console events (Ctrl+C / Ctrl+Break) raised by or for the child
        // stop at the child. Same flag as the exec tool.
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        // Own process group: isolates group signals AND gives the timeout
        // path a pgid to kill.
        cmd.process_group(0);
    }
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    // Capture the pid before the wait future consumes the child, so a timeout
    // can kill the whole tree rooted here (not just the shell).
    let pid = child.id();
    let wait = child.wait_with_output();
    tokio::pin!(wait);
    let output = tokio::select! {
        res = &mut wait => res.map_err(|e| e.to_string())?,
        () = tokio::time::sleep(timeout) => {
            if let Some(pid) = pid {
                kill_process_tree(pid).await;
            }
            return Err(format!("timed out after {}s", timeout.as_secs()));
        }
    };
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

    /// How many OPEN children an item has, or `None` when this source cannot
    /// report it.
    ///
    /// A replan step decomposes a stalled item by adding subtasks through the
    /// store, so this is how the runner tells a replan that produced work
    /// from one that produced nothing. `None` means "unknown", and the runner
    /// then treats every replan as productive — the pre-existing behavior, so
    /// a source that does not implement this is unaffected.
    async fn open_subtasks(&self, _id: i64) -> Result<Option<usize>, String> {
        Ok(None)
    }
}

/// Request for one step: everything the runner needs to build a fresh-context
/// agent run.
#[derive(Debug, Clone)]
pub struct StepRequest {
    pub item_id: i64,
    pub step_index: usize,
    pub step_kind: StepKind,
    /// The item's title, for status display.
    ///
    /// Carried explicitly rather than parsed back out of `prompt`: the old
    /// reader took everything after `Task #id: `, so a replan step — whose
    /// prompt line reads "…title has made no verifiable progress (5 steps
    /// without the done-condition flipping)" — put that whole sentence on
    /// screen as if it were the task name.
    pub item_title: String,
    pub prompt: String,
    /// Tools to activate for the step (≤ a handful — small models degrade
    /// past 5-10 tool definitions).
    pub tool_scope: Vec<String>,
    pub token_budget: Option<u64>,
    pub max_iterations: Option<usize>,
    pub max_wall_clock: Option<Duration>,
    /// The run's shared cancellation token — the SAME token the Stop button
    /// cancels. The harness polls it only at step boundaries, and a step can
    /// legitimately run for many minutes (a chat turn's one-task plan IS a
    /// single step); the runner must thread this into the in-step agent loop
    /// so Stop aborts the in-flight LLM stream and skips queued tool calls
    /// instead of waiting the step out.
    pub cancel: Option<CancelToken>,
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

/// Folds newly-arrived user input into the running plan.
///
/// A long-horizon run can last hours, so a user message that arrives mid-run
/// must not wait for the run to end — it is admitted at the **first available
/// opportunity**, which is a step boundary. Mid-step is not an opportunity:
/// a step owns a fresh context and an item, and interrupting it would strand
/// half-finished work the acceptance check would then refute.
///
/// The split of duties is deliberate: the harness owns *when* input may
/// enter (here, and only here); the implementation owns *what* entering
/// means — in the daemon it plans the message and inserts the tasks at top
/// priority, so the very next `next()` surfaces them ahead of existing work.
#[async_trait::async_trait]
pub trait Interjector: Send + Sync {
    /// Admit any pending input into the plan. Returns the number of items
    /// admitted (0 when nothing was waiting — the overwhelmingly common
    /// case, so implementations must make that path cheap).
    ///
    /// Errors are logged and the run continues: failing to admit a message
    /// must never kill work already in flight.
    async fn interject(&self) -> Result<usize, String>;
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
    /// Optional token budget per step. None by default: the re-anchor
    /// cadence is already enforced by `step_iterations`, and a token cap
    /// proved to truncate PRODUCTIVE steps mid-work once the artifact under
    /// construction grew (observed live: steps dying at exactly the budget on
    /// later features, feeding stall → replan pressure). Set it only when a
    /// hard per-step spend ceiling matters more than step completion.
    pub step_token_budget: Option<u64>,
    /// Consecutive runner errors before the run stops (circuit breaker for a
    /// dead model endpoint).
    pub max_consecutive_errors: usize,
    /// Item ids whose acceptance check may run BEFORE a step: when one of
    /// these items' checks already passes with nothing run, complete it on
    /// that verdict and take no step.
    ///
    /// This is the harness's own philosophy applied one moment earlier — the
    /// check is the only judge of "done", so a check that already passes means
    /// the work is already done, and running a step to discover that burns a
    /// model round to learn what one command already said.
    ///
    /// **Empty by default, and the emptiness is the bound.** On the FIRST plan
    /// of a turn nothing has run yet, so a done-condition that already passes
    /// says the planner wrote a condition the world happened to satisfy — not
    /// that the user's request is moot. Completing there would delete real
    /// work. It is populated only for mission CONTINUATION rounds, where the
    /// run has already acted on the world this turn and the question "is this
    /// re-proposal already satisfied?" is exactly the mission's termination
    /// question. See `chat_harness`'s continuation loop.
    ///
    /// **A SET OF IDS, not a flag, and that is the invariant.** The pre-check
    /// exists to skip work the CONTINUATION PLANNER re-proposed that the
    /// environment already satisfies; it must NEVER apply to work a user just
    /// asked for. A flag scoped to the round would cover every item the round
    /// happens to select, and the harness polls the [`Interjector`] at the top
    /// of each iteration — so a message the user sent mid-round is planned
    /// into a new item and selected under the same flag. If its planned
    /// acceptance happened to pass already, that item would close with zero
    /// steps run and the user would never be answered. Interjections are the
    /// one thing that must never be skipped. Carrying the exact ids
    /// `seed_continuation` returned makes the wrong item unreachable rather
    /// than merely unlikely: leftovers, replan subtasks and interjected asks
    /// are all outside the set and all run normally.
    pub precheck_acceptance_items: HashSet<i64>,
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
            step_token_budget: None,
            max_consecutive_errors: 3,
            precheck_acceptance_items: HashSet::new(),
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
    /// Total tool calls made across every step.
    ///
    /// The signal for "this run ACTED on the world", which is what separates a
    /// conversational reply from work in progress. Step and item counts cannot
    /// carry that: a 42-feature build whose planner emits one task and whose
    /// first step completes it looks numerically identical to "hi".
    pub tool_calls: usize,
    /// The subset of `tool_calls` that CHANGED SOMETHING — the work-evidence
    /// set the completion-claim rung accepts as proof (`is_work_evidence_tool`:
    /// writes, edits, the shell). Reads and searches can run forever without
    /// touching anything outside the context window, so `tool_calls > 0`
    /// answers "did this run act?" while only this field answers "did this run
    /// change the world?".
    ///
    /// The caller that needs the distinction is the chat harness's
    /// mission-continuation loop: a round that changed nothing and closed
    /// nothing is not progress, whatever its tasks were titled.
    #[serde(default)]
    pub side_effect_tool_calls: usize,
    pub items_completed: usize,
    /// Items completed on the model's claim alone (no acceptance check
    /// existed) — logged so unverified completions are visible.
    pub items_completed_unverified: usize,
    /// The subset of `items_completed` closed by the acceptance PRE-CHECK:
    /// their done-condition already passed before any step ran, so the run
    /// completed them for free (see
    /// `LongHorizonConfig::precheck_acceptance_items`).
    ///
    /// Counted apart because these completions are evidence the goal was
    /// ALREADY met, not evidence this run advanced it. The chat harness's
    /// mission loop needs that distinction: a continuation round whose every
    /// item was already satisfied changed nothing, and must count dry.
    #[serde(default)]
    pub items_already_satisfied: usize,
    pub items_abandoned: usize,
    /// The most recent step-runner error seen during the run, kept so a
    /// caller can say WHY when the plan drained through abandonment rather
    /// than completion. Poison containment turns a deterministic runner
    /// fault (e.g. "no provider for the configured model") into abandoned
    /// items and a clean `AllTasksDone` — without this field that failure is
    /// indistinguishable from a finished run.
    #[serde(default)]
    pub last_runner_error: Option<String>,
    pub replans: usize,
    /// Completion claims that the acceptance check refuted (the
    /// "false success" counter — the P14 anti-drift keystone at work).
    pub false_success_claims: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_clock_secs: u64,
    /// total tokens / items completed. None when nothing completed.
    pub tokens_per_completed_item: Option<u64>,
    /// Items admitted mid-run by an [`Interjector`] — user messages that
    /// joined the plan at a step boundary instead of waiting for the run.
    #[serde(default)]
    pub interjected_items: usize,
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
            // Close the feedback loop INSIDE the step. Measured live
            // (gemma4:12b, 42-feature ladder): across two hours the model
            // never once ran its own acceptance command, even though the
            // line above names it — so it edited blind, learned the verdict
            // only at the step boundary, and re-submitted the same broken
            // implementation repeatedly. Checking costs one `exec`; not
            // checking costs a whole step.
            if let Some(command) = check.self_check_command() {
                prompt.push_str(&format!(
                    "Before you finish this step, RUN THAT CHECK YOURSELF: exec `{command}`. \
                     Read its output, fix what it reports, and run it again until it passes. \
                     Do not end the step on an unverified guess.\n"
                ));
            }
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
    // Describes the OPERATION, not a tool signature. Naming a tool and its
    // exact arguments in a prompt that may not ship that tool's schema is how
    // the model ends up obeying blind and guessing parameter names — measured
    // at 53-66% malformed calls. Discovery is how it gets the real signature.
    prompt.push_str(&format!(
        "Break it into 2-5 smaller subtasks, added to the task store as children of \
         task #{} — discover the task-management tool if you do not already have it. \
         Each subtask needs a machine-checkable acceptance condition (a command to run, \
         a file that must exist, or a pattern that must match) wherever possible. Make \
         the first subtask small enough to finish in one step. Do not attempt the work \
         itself in this step — only decompose.\n",
        step.id
    ));
    prompt
}

/// The byte-stable prompt prefix: system-adjacent rules + the immutable goal.
fn stable_prefix(goal: &str) -> String {
    format!(
        "== GOAL (immutable — this is the whole point of the run) ==\n{goal}\n\n\
         == HOW TO WORK ==\n\
         You are executing one step of a long-running plan. You do not need to remember \
         anything between steps — the plan and everything you have already done are kept \
         for you.\n\
         - Advance the CURRENT TASK below by exactly one concrete action.\n\
         - SAY WHAT YOU ARE DOING: open with ONE short sentence naming the action you are \
           about to take and why it follows from the last result. Someone is watching this \
           run and that sentence is all they see.\n\
         - GET THE RIGHT TOOL FIRST. You start each run able to see only one tool, for \
           discovering the others. Everything you need — running commands, reading, \
           writing and editing files, searching your memory — exists and is waiting behind \
           it. Ask for what the task needs, then use the real tool.\n\
           Do not improvise around a tool you cannot see: writing a file by shell \
           redirection instead of the file tool, or piping a heredoc instead of editing, \
           skips the checks that keep this run alive and is how work gets silently lost.\n\
         - Use the tools you have; do not narrate actions you did not take. Describe what \
           actually happened, including failures — a wrong result reported honestly is worth \
           more than a confident guess.\n\
         - ONE artifact per job: change the real file in place. Do not build a parallel copy \
           (`thing.sh` beside `thing`, `thing_v2`, `thing.new`) — whatever reads the real file \
           keeps reading it, so work in a copy scores zero no matter how good it is. This \
           applies to shell redirects too, not just the file tools.\n\
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

/// Errors from the step runner charged to ONE item before that item is
/// abandoned as poisoned.
///
/// Bound justification: the runner already retries transient provider
/// failures internally, so an `Err` here means several consecutive failed
/// generations. When that happens twice on the same item while other items
/// proceed, the failure follows the item (something in its prompt/notes
/// deterministically breaks the model) — containing it at item granularity
/// costs one feature instead of the whole run. Provider-wide death still
/// trips the run-level breaker, whose counter spans items.
pub const ITEM_RUNNER_ERRORS_MAX: usize = 2;

/// Per-item progress bookkeeping.
#[derive(Debug, Default, Clone)]
struct ItemProgress {
    steps_without_progress: usize,
    replans: usize,
    tokens_spent: u64,
    last_result: Option<String>,
    last_tool_calls: Vec<StepToolCall>,
    runner_errors: usize,
    /// Has the acceptance pre-check already run for this item in this run?
    ///
    /// The bound on the pre-check's cost: ONE extra acceptance execution per
    /// item per run, taken at the item's first selection. Without it a
    /// grinding item would re-run its check before every step as well as
    /// after it, doubling the command executions of the slowest thing the
    /// harness does — and re-asking a question the post-step verdict answered
    /// moments earlier, from the same state.
    acceptance_prechecked: bool,
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
        cancel: Option<CancelToken>,
    ) -> LongHorizonReport {
        self.run_with_interjector(goal, source, runner, workdir, cancel, None)
            .await
    }

    /// [`Self::run`], plus a hook that admits new user input at each step
    /// boundary. This is the shape chat uses: a run is live for as long as
    /// the plan takes, and messages sent meanwhile join it rather than
    /// queueing behind it.
    pub async fn run_with_interjector(
        &self,
        goal: &str,
        source: &dyn TaskSource,
        runner: &dyn StepRunner,
        workdir: &Path,
        cancel: Option<CancelToken>,
        interjector: Option<&dyn Interjector>,
    ) -> LongHorizonReport {
        let cfg = &self.config;
        let mut interjected_items = 0usize;
        let started = Instant::now();
        let mut steps_taken = 0usize;
        let mut tool_calls_total = 0usize;
        let mut side_effect_tool_calls = 0usize;
        let mut items_completed = 0usize;
        let mut items_completed_unverified = 0usize;
        let mut items_already_satisfied = 0usize;
        let mut items_abandoned = 0usize;
        let mut replans = 0usize;
        let mut false_success_claims = 0usize;
        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;
        let mut consecutive_errors = 0usize;
        let mut last_runner_error: Option<String> = None;
        let mut progress: HashMap<i64, ItemProgress> = HashMap::new();

        let stop = loop {
            if cancel.as_ref().is_some_and(CancelToken::is_cancelled) {
                break StopReason::Cancelled;
            }
            if started.elapsed() >= cfg.max_wall_clock {
                break StopReason::WallClockExhausted;
            }
            let tokens_used = input_tokens + output_tokens;
            if cfg.max_total_tokens.is_some_and(|max| tokens_used >= max) {
                break StopReason::TokenBudgetExhausted;
            }

            // The first available opportunity: admit anything the user sent
            // while the previous step ran, BEFORE choosing the next item, so
            // a fresh request is picked up by this very iteration rather
            // than after the rest of the plan drains.
            if let Some(interjector) = interjector {
                match interjector.interject().await {
                    Ok(0) => {}
                    Ok(admitted) => {
                        interjected_items += admitted;
                    }
                    // A failed admission must not kill work in flight; the
                    // message stays pending and is retried next boundary.
                    Err(message) => {
                        tracing::warn!(%message, "interjection failed — retrying next step");
                    }
                }
            }

            // Re-anchor: the store, not the transcript, is the state.
            let step = match source.next().await {
                Ok(Some(step)) => step,
                Ok(None) => break StopReason::AllTasksDone,
                Err(message) => break StopReason::SourceError { message },
            };
            // Scoped so the pre-check below (which may `progress.remove`) is
            // not fighting a live borrow of the same map.
            let (is_replan, fruitless_steps, item_replans, precheck_due) = {
                let item = progress.entry(step.id).or_default();
                let precheck_due = !item.acceptance_prechecked;
                item.acceptance_prechecked = true;
                (
                    item.steps_without_progress >= cfg.max_steps_per_item,
                    item.steps_without_progress,
                    item.replans,
                    precheck_due,
                )
            };

            if is_replan && item_replans >= cfg.max_replans_per_item {
                // Grinding AND replanning failed — close the item and move on.
                let reason = format!(
                    "abandoned after {fruitless_steps} fruitless steps and {item_replans} replans"
                );
                if let Err(message) = source.abandon(step.id, &reason).await {
                    break StopReason::SourceError { message };
                }
                items_abandoned += 1;
                progress.remove(&step.id);
                continue;
            }

            let _ = source.start(step.id).await;

            // ACCEPTANCE PRE-CHECK. The environment is the judge of "done";
            // ask it before spending a step, not only after. When the
            // done-condition already passes with nothing run, the work is
            // provably already there and a step could only rediscover that.
            //
            // Opt-in (`precheck_acceptance_items`) because on a first plan a
            // pre-passing condition means the check is wrong, not the work
            // done — see the field's docs. Scoped to the exact ids the caller
            // named rather than to "every item this run selects": the
            // interjection poll above can add a USER'S message to the plan
            // mid-round, and skipping that on a pre-passing condition would
            // leave the user unanswered. Bounded to one execution per item per
            // run by `acceptance_prechecked`, and each execution reuses
            // `AcceptanceCheck::run`, so it inherits the same workdir
            // resolution and the same clamped timeout as every other
            // acceptance run — no second execution path to keep in sync.
            let precheck = if precheck_due && cfg.precheck_acceptance_items.contains(&step.id) {
                step.acceptance.as_ref()
            } else {
                None
            };
            if let Some(check) = precheck {
                let verdict = check.run(workdir).await;
                if verdict.passed {
                    let detail = serde_json::json!({
                        "verified": true,
                        "verdict": verdict.detail,
                        "already_satisfied": true,
                        "steps_run": 0,
                        "tokens_spent": 0,
                    });
                    let _ = source
                        .log(
                            step.id,
                            "acceptance_already_satisfied",
                            serde_json::json!({
                                "detail": verdict.detail,
                                "check": check.describe(),
                            }),
                        )
                        .await;
                    match source.complete(step.id, detail).await {
                        Ok(()) => {
                            consecutive_errors = 0;
                            items_completed += 1;
                            items_already_satisfied += 1;
                            progress.remove(&step.id);
                            tracing::info!(
                                item = step.id,
                                title = %step.title,
                                verdict = %verdict.detail,
                                "acceptance already passed before any step — completing \
                                 without running one"
                            );
                            continue;
                        }
                        Err(message) => {
                            // Same containment as the post-step path: a
                            // completion can legitimately fail (a concurrent
                            // decomposition opened a child). Fall through and
                            // run the step normally.
                            let _ = source
                                .log(
                                    step.id,
                                    "complete_failed",
                                    serde_json::json!({ "error": message }),
                                )
                                .await;
                        }
                    }
                }
            }

            let item = progress.entry(step.id).or_default();

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

            // Open-children count BEFORE a replan runs, so "did this replan
            // actually decompose anything?" is answerable afterwards. Only
            // taken for replan steps — an execute step has no such contract,
            // and this is a store round-trip.
            let subtasks_before = if is_replan {
                source.open_subtasks(step.id).await.unwrap_or(None)
            } else {
                None
            };

            let remaining_wall = cfg.max_wall_clock.saturating_sub(started.elapsed());
            let request = StepRequest {
                item_id: step.id,
                step_index: steps_taken,
                step_kind,
                item_title: step.title.clone(),
                prompt,
                tool_scope: step.tool_scope.clone(),
                token_budget: cfg.step_token_budget,
                max_iterations: Some(cfg.step_iterations),
                max_wall_clock: Some(remaining_wall),
                cancel: cancel.clone(),
            };

            let outcome = match runner.run_step(request).await {
                Ok(outcome) => {
                    consecutive_errors = 0;
                    outcome
                }
                Err(message) => {
                    consecutive_errors += 1;
                    last_runner_error = Some(message.clone());
                    let item = progress.entry(step.id).or_default();
                    item.runner_errors += 1;
                    // Poison containment: when the failure follows one item
                    // (its prompt/notes deterministically break the model),
                    // abandon that item and keep the run alive. Provider-wide
                    // death still trips the run-level breaker below, whose
                    // counter spans items.
                    if item.runner_errors >= ITEM_RUNNER_ERRORS_MAX {
                        let reason =
                            format!("abandoned after persistent runner errors: {message}");
                        if let Err(message) = source.abandon(step.id, &reason).await {
                            break StopReason::SourceError { message };
                        }
                        items_abandoned += 1;
                        progress.remove(&step.id);
                        continue;
                    }
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
                // next() will surface them. A replan that actually decomposed
                // resets the grind counter so the new work gets a fresh
                // allowance — but a DRY replan must not buy one.
                //
                // Observed live 2026-08-02 (session 05775d1d): a replan
                // re-proposed the title the run had just abandoned, the store
                // refused to resurrect it, and the reset handed the item
                // another full allowance of fruitless steps — the item span
                // grew by `max_steps_per_item` per empty replan instead of
                // converging. With the reset withheld the item is still at
                // its grind threshold, so the next iteration replans again
                // and the existing `max_replans_per_item` rung reaches
                // abandonment directly. A source that cannot report its
                // children (`None`) is treated as productive, exactly as
                // before.
                let produced_work = match (subtasks_before, source.open_subtasks(step.id).await) {
                    (Some(before), Ok(Some(after))) => after > before,
                    _ => true,
                };
                item.replans += 1;
                if produced_work {
                    item.steps_without_progress = 0;
                    item.last_result = None;
                    item.last_tool_calls.clear();
                } else {
                    tracing::info!(
                        item = step.id,
                        replans = item.replans,
                        "replan added no subtasks — not resetting the grind counter"
                    );
                }
                replans += 1;
                let _ = source
                    .log(
                        step.id,
                        "replanned",
                        serde_json::json!({
                            "replans": item.replans,
                            "produced_work": produced_work,
                        }),
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

            // A cancelled step is a truncated step: its text is not a
            // verdictable claim, and an acceptance command run now is work
            // after the user said stop. Findings are already noted above.
            if cancel.as_ref().is_some_and(CancelToken::is_cancelled) {
                break StopReason::Cancelled;
            }

            // Cross-step repetition = a stall the in-run detector cannot see.
            let repeated = steps_repeat(&item.last_tool_calls, &outcome.tool_calls);
            item.last_tool_calls.clone_from(&outcome.tool_calls);
            tool_calls_total += outcome.tool_calls.len();
            // Counted by NAME, not by outcome: a [`StepToolCall`] is a digest
            // and carries no success flag. An attempted-but-failed `exec`
            // therefore reads as "the step reached for the world", which errs
            // toward letting a mission continue — the cheap mistake, since the
            // opposite one ends a live mission a round early.
            side_effect_tool_calls += outcome
                .tool_calls
                .iter()
                .filter(|call| crate::loop_runner::is_work_evidence_tool(&call.name))
                .count();

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
                        item.last_result =
                            Some(failed_acceptance_result(check, &verdict, repeated));
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
            tool_calls: tool_calls_total,
            side_effect_tool_calls,
            items_completed,
            items_completed_unverified,
            items_already_satisfied,
            items_abandoned,
            last_runner_error,
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
            interjected_items,
        }
    }
}

/// Feedback handed to the next step after a failed acceptance check —
/// rendered verbatim into the `== LAST RESULT ==` block of the retry prompt.
///
/// Measured live (gemma4:12b, 2026-08-02 endurance evals): 320+ failed
/// checks and ~130 steps per feature, because after a failure the model saw
/// only "Done-condition NOT met: `FAIL(test_04)`: ..." — it was never shown
/// WHAT command judges the task, nor told to read the test it must satisfy,
/// so it retried by guessing. qwen3.5:9b infers the command from context;
/// gemma4:12b does not. Naming the command and directing a read-first turns
/// the guess loop into a feedback loop.
///
/// Bounds: `verdict.detail` embeds planner-authored strings (command,
/// pattern, path) that carry no upstream byte cap — only the command-output
/// tail inside it is bounded (400 chars) — so it is clamped here to
/// [`STEP_RESULT_TAIL_MAX_BYTES`], the same one-screenful bound step notes
/// use, with the cut announced (the full detail is already durably logged
/// via the `acceptance_checked` entry before this runs). The command itself
/// is included verbatim: the model must be able to run and read it exactly,
/// truncating it would destroy the one thing this message exists to hand
/// over, and the step prompt already embeds it verbatim ("RUN THAT CHECK
/// YOURSELF"). Everything else is a fixed-size frame, so the whole message
/// is O(one screenful).
///
/// The file the command runs is deliberately NOT inlined here: the
/// directive names it via the command, and the model must exercise
/// `read_file` itself — inlining would bloat every retry and the file can
/// be large.
fn failed_acceptance_result(
    check: &AcceptanceCheck,
    verdict: &AcceptanceVerdict,
    repeated: bool,
) -> String {
    let detail = if verdict.detail.len() <= STEP_RESULT_TAIL_MAX_BYTES {
        verdict.detail.clone()
    } else {
        format!(
            "[showing last {STEP_RESULT_TAIL_MAX_BYTES} of {} bytes — full verdict is in the \
             acceptance_checked run log] {}",
            verdict.detail.len(),
            text_tail(&verdict.detail, STEP_RESULT_TAIL_MAX_BYTES)
        )
    };
    let mut result = format!("Done-condition NOT met: {detail}");
    if repeated {
        result.push_str(
            " (you repeated the exact same tool calls as last step — change approach)",
        );
    }
    if let Some(command) = check.self_check_command() {
        result.push_str(&format!(
            "\nThat verdict came from the harness running: `{command}`\n\
             Before changing anything, read the file(s) this command runs (use read_file), \
             understand every assertion, then make them pass. Do not guess."
        ));
    }
    result
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
    use std::sync::Arc;
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
        /// Open-children ledger. `None` (the default) means "this source
        /// cannot report", which is the pre-existing path every other test
        /// exercises: the runner then treats every replan as productive.
        subtasks: Option<Arc<Mutex<HashMap<i64, usize>>>>,
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
        async fn open_subtasks(&self, id: i64) -> Result<Option<usize>, String> {
            match &self.subtasks {
                Some(ledger) => Ok(Some(ledger.lock().await.get(&id).copied().unwrap_or(0))),
                None => Ok(None),
            }
        }
    }

    /// Replays a script and, on every Plan step, adds `per_replan` open
    /// children to the shared ledger — what a replan step does when it
    /// actually decomposes something. At 0 it is the DRY replan: the step
    /// ran, and the store holds nothing new (every title it proposed was
    /// already closed this turn, so `tasks.add` refused to resurrect it).
    struct ReplanningRunner {
        inner: ScriptedRunner,
        subtasks: Arc<Mutex<HashMap<i64, usize>>>,
        per_replan: usize,
    }

    #[async_trait::async_trait]
    impl StepRunner for ReplanningRunner {
        async fn run_step(&self, request: StepRequest) -> Result<StepOutcome, String> {
            if request.step_kind == StepKind::Plan && self.per_replan > 0 {
                *self
                    .subtasks
                    .lock()
                    .await
                    .entry(request.item_id)
                    .or_insert(0) += self.per_replan;
            }
            self.inner.run_step(request).await
        }
    }

    /// Creates a file on every step — the environment changing UNDER the
    /// harness, which is the only thing an acceptance check exists to notice.
    struct FileWritingRunner {
        path: PathBuf,
        steps: Mutex<usize>,
    }

    impl FileWritingRunner {
        fn new(path: PathBuf) -> Self {
            Self {
                path,
                steps: Mutex::new(0),
            }
        }
    }

    #[async_trait::async_trait]
    impl StepRunner for FileWritingRunner {
        async fn run_step(&self, _request: StepRequest) -> Result<StepOutcome, String> {
            *self.steps.lock().await += 1;
            std::fs::write(&self.path, b"done").map_err(|e| e.to_string())?;
            Ok(outcome("wrote the artifact"))
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

    #[tokio::test]
    async fn run_shell_timeout_errors_and_returns_promptly() {
        let dir = tempfile::tempdir().unwrap();
        // A sleeper that works under every shell run_shell can route to.
        #[cfg(windows)]
        let command = "ping -n 30 127.0.0.1";
        #[cfg(not(windows))]
        let command = "sleep 30";

        let started = std::time::Instant::now();
        let result = run_shell(command, dir.path(), Duration::from_secs(1)).await;
        let err = result.expect_err("a 30s sleeper must time out at 1s");
        assert!(err.contains("timed out"), "unexpected error: {err}");
        // Bound: 1s timeout + tree-kill (taskkill subprocess on Windows).
        // 10s is a generous ceiling for a loaded CI machine; the pre-fix
        // hang mode was the full 30s sleeper duration.
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "timeout path must not wait for the workload"
        );
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

    /// gemma4:12b ran its acceptance command ZERO times across two hours of
    /// the 42-feature ladder, so it edited blind and re-submitted the same
    /// broken implementation. The prompt now hands it the command.
    #[test]
    fn step_prompt_tells_the_model_to_run_the_check_itself() {
        let check = AcceptanceCheck::from_json(&serde_json::json!({
            "kind": "command", "command": "sh tests/test_05.sh"
        }))
        .unwrap();
        let prompt = build_step_prompt("g", &step(1, "t", Some(check)), None, "b");
        assert!(prompt.contains("RUN THAT CHECK YOURSELF"));
        assert!(prompt.contains("exec `sh tests/test_05.sh`"));
    }

    /// A pure file/regex check has no command to run — inventing one would
    /// put words in the check's mouth, so the instruction is omitted.
    #[test]
    fn a_file_check_gets_no_self_check_instruction() {
        let check = AcceptanceCheck::from_json(&serde_json::json!({
            "kind": "file_exists", "path": "greeting.txt"
        }))
        .unwrap();
        assert!(check.self_check_command().is_none());
        let prompt = build_step_prompt("g", &step(1, "t", Some(check)), None, "b");
        assert!(!prompt.contains("RUN THAT CHECK YOURSELF"));
        assert!(prompt.contains("must exist"), "the condition is still stated");
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
    // Failed-acceptance feedback
    // -----------------------------------------------------------------

    /// 2026-08-02 endurance evals (gemma4:12b): 320+ failed checks at ~130
    /// steps/feature because the retry feedback never named the judging
    /// command. The failure message must hand the model the command and
    /// direct it to read the test before editing.
    #[test]
    fn failed_verdict_feedback_names_command_and_directs_read_first() {
        let check = AcceptanceCheck::Command {
            command: "sh tests/test_04.sh".to_string(),
            timeout_secs: None,
        };
        let verdict = AcceptanceVerdict {
            passed: false,
            detail: "FAIL(test_04): exit code should be 1, got 0".to_string(),
        };
        let result = failed_acceptance_result(&check, &verdict, false);
        assert!(
            result.starts_with("Done-condition NOT met: FAIL(test_04)"),
            "{result}"
        );
        assert!(
            result.contains("running: `sh tests/test_04.sh`"),
            "the exact acceptance command must be named: {result}"
        );
        assert!(
            result.contains("read the file(s) this command runs (use read_file)"),
            "{result}"
        );
        assert!(result.contains("Do not guess"), "{result}");
        assert!(
            !result.contains("repeated the exact same tool calls"),
            "no repetition admonition without repetition: {result}"
        );
    }

    #[test]
    fn failed_verdict_feedback_keeps_the_repeated_steps_admonition() {
        let check = AcceptanceCheck::Command {
            command: "sh run_tests.sh".to_string(),
            timeout_secs: None,
        };
        let verdict = AcceptanceVerdict {
            passed: false,
            detail: "`sh run_tests.sh` exited 1 — 3 failures".to_string(),
        };
        let result = failed_acceptance_result(&check, &verdict, true);
        assert!(
            result.contains("repeated the exact same tool calls as last step — change approach"),
            "{result}"
        );
        // The admonition must not displace the read-first directive.
        assert!(result.contains("Do not guess"), "{result}");
    }

    /// A pure file check has no command to read — the directive would name
    /// nothing, so it is omitted (the condition is already stated in the
    /// step prompt's "Done when" line).
    #[test]
    fn failed_file_check_feedback_omits_the_read_directive() {
        let check = AcceptanceCheck::FileExists {
            path: "out.txt".to_string(),
        };
        let verdict = AcceptanceVerdict {
            passed: false,
            detail: "file does not exist: out.txt".to_string(),
        };
        let result = failed_acceptance_result(&check, &verdict, false);
        assert!(result.starts_with("Done-condition NOT met:"), "{result}");
        assert!(
            !result.contains("Before changing anything"),
            "no command means no read-first directive: {result}"
        );
    }

    /// The verdict detail embeds planner-authored strings with no upstream
    /// byte cap; the feedback clamps it to `STEP_RESULT_TAIL_MAX_BYTES` —
    /// the step-note bound — keeping the newest evidence and announcing the
    /// cut, without displacing the command or the directive.
    #[test]
    fn failed_verdict_feedback_bounds_a_runaway_detail() {
        let check = AcceptanceCheck::Command {
            command: "sh tests/test_04.sh".to_string(),
            timeout_secs: None,
        };
        let verdict = AcceptanceVerdict {
            passed: false,
            detail: format!("{}NEWEST-EVIDENCE", "x".repeat(STEP_RESULT_TAIL_MAX_BYTES * 10)),
        };
        let result = failed_acceptance_result(&check, &verdict, true);
        assert!(
            result.len() <= STEP_RESULT_TAIL_MAX_BYTES + 512,
            "detail clamped to the step-note bound plus a fixed frame: {} bytes",
            result.len()
        );
        assert!(
            result.contains("NEWEST-EVIDENCE"),
            "tail truncation keeps the newest evidence"
        );
        assert!(
            result.contains("[showing last"),
            "the cut must announce itself: {result}"
        );
        assert!(result.contains("`sh tests/test_04.sh`"), "{result}");
        assert!(result.contains("Do not guess"), "{result}");
    }

    /// A short detail passes through byte-identical — no truncation marker.
    #[test]
    fn failed_verdict_feedback_leaves_short_detail_untouched() {
        let check = AcceptanceCheck::FileExists {
            path: "a.txt".to_string(),
        };
        let verdict = AcceptanceVerdict {
            passed: false,
            detail: "file does not exist: a.txt".to_string(),
        };
        let result = failed_acceptance_result(&check, &verdict, false);
        assert_eq!(result, "Done-condition NOT met: file does not exist: a.txt");
    }

    /// End-to-end: after a failed check, the NEXT step's prompt carries the
    /// command and the read-first directive in its LAST RESULT block.
    #[tokio::test]
    async fn failed_acceptance_enriches_the_next_step_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "make the test pass",
                Some(AcceptanceCheck::Command {
                    // `exit 1` works under both sh and cmd.
                    command: "exit 1".to_string(),
                    timeout_secs: None,
                }),
            ))
            .await;
        let runner = ScriptedRunner::new(vec![Ok(outcome("try 1")), Ok(outcome("try 2"))]);
        let _report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        let requests = runner.requests.lock().await;
        assert!(requests.len() >= 2, "the item must get a retry step");
        let retry = &requests[1].prompt;
        assert!(retry.contains("== LAST RESULT =="), "{retry}");
        assert!(retry.contains("Done-condition NOT met"), "{retry}");
        assert!(
            retry.contains("That verdict came from the harness running: `exit 1`"),
            "{retry}"
        );
        assert!(
            retry.contains("read the file(s) this command runs (use read_file)"),
            "{retry}"
        );
        // First attempt has no verdict yet — the enrichment is failure-only.
        assert!(!requests[0].prompt.contains("Done-condition NOT met"));
    }

    // -----------------------------------------------------------------
    // Control loop
    // -----------------------------------------------------------------

    /// The report must separate "called tools" from "changed something": the
    /// chat harness's mission loop treats a round that only READ as making no
    /// progress, and reads are most of a round's tool calls.
    #[tokio::test]
    async fn the_report_counts_side_effects_apart_from_tool_calls() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "look around",
                Some(AcceptanceCheck::FileExists {
                    path: "never.txt".to_string(),
                }),
            ))
            .await;
        let called = |names: &[&str]| {
            Ok(StepOutcome {
                text: "had a look".to_string(),
                input_tokens: 1000,
                output_tokens: 200,
                tool_calls: names
                    .iter()
                    .map(|name| StepToolCall {
                        name: (*name).to_string(),
                        input_digest: (*name).to_string(),
                        output_digest: String::new(),
                    })
                    .collect(),
            })
        };
        let runner = ScriptedRunner::new(vec![
            called(&["read_file", "code_search", "list_dir"]),
            called(&["read_file", "exec"]),
        ]);
        let config = LongHorizonConfig {
            max_steps_per_item: 2,
            max_replans_per_item: 0,
            ..fast_config()
        };
        let report = LongHorizonRunner::new(config)
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.tool_calls, 5, "every call counts here");
        assert_eq!(
            report.side_effect_tool_calls, 1,
            "only the shell call changed anything: {report:?}"
        );
    }

    // -----------------------------------------------------------------
    // Acceptance pre-check: ask the environment before spending a step
    // -----------------------------------------------------------------

    /// The decisive convergence signal. A seeded item whose done-condition
    /// ALREADY passes is finished work: complete it on that verdict, run no
    /// step, and record it apart so the caller can tell "already met" from
    /// "this run met it".
    ///
    /// The runner's script is EMPTY on purpose — any step at all fails the
    /// test loudly instead of quietly costing a model round.
    #[tokio::test]
    async fn an_already_passing_acceptance_completes_without_running_a_step() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("artifact.txt"), b"already here").unwrap();
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "produce the artifact",
                Some(AcceptanceCheck::FileExists {
                    path: "artifact.txt".to_string(),
                }),
            ))
            .await;
        let runner = ScriptedRunner::new(vec![]);
        let config = LongHorizonConfig {
            precheck_acceptance_items: HashSet::from([1]),
            ..fast_config()
        };
        let report = LongHorizonRunner::new(config)
            .run("goal", &source, &runner, dir.path(), None)
            .await;

        assert_eq!(report.stop, StopReason::AllTasksDone);
        assert_eq!(report.steps_taken, 0, "no step: {report:?}");
        assert!(
            runner.requests.lock().await.is_empty(),
            "the runner was never asked to do anything"
        );
        assert_eq!(report.items_completed, 1, "the item IS closed");
        assert_eq!(
            report.items_already_satisfied, 1,
            "…and closed as already-satisfied, which is not progress"
        );
        assert_eq!(report.input_tokens + report.output_tokens, 0);

        // The verdict says WHY it closed — a completion with no step behind it
        // must announce itself, in the store the user can read.
        let completions = source.completions.lock().await;
        let (id, detail) = completions.first().expect("one completion");
        assert_eq!(*id, 1);
        assert_eq!(detail["already_satisfied"], serde_json::json!(true));
        assert_eq!(detail["verified"], serde_json::json!(true));
        assert_eq!(detail["steps_run"], serde_json::json!(0));
        assert!(
            source
                .log_entries
                .lock()
                .await
                .iter()
                .any(|(_, action)| action == "acceptance_already_satisfied"),
            "the activity log records the pre-check"
        );
    }

    /// The other direction, and the larger half of the contract: a check that
    /// does NOT already pass changes nothing about how the item runs. The step
    /// happens, the post-step verdict closes it, and the completion is
    /// ordinary — `items_already_satisfied` stays 0.
    ///
    /// This also pins the pre-check's bound: it runs at most ONCE per item per
    /// run. The environment flips mid-run (the step writes the file), and the
    /// item still closes through the post-step verdict rather than a second
    /// pre-check.
    #[tokio::test]
    async fn a_failing_pre_check_runs_the_step_exactly_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("artifact.txt");
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "produce the artifact",
                Some(AcceptanceCheck::FileExists {
                    path: "artifact.txt".to_string(),
                }),
            ))
            .await;
        let runner = FileWritingRunner::new(artifact.clone());
        let config = LongHorizonConfig {
            precheck_acceptance_items: HashSet::from([1]),
            ..fast_config()
        };
        let report = LongHorizonRunner::new(config)
            .run("goal", &source, &runner, dir.path(), None)
            .await;

        assert_eq!(report.stop, StopReason::AllTasksDone);
        assert_eq!(report.steps_taken, 1, "the step ran: {report:?}");
        assert_eq!(*runner.steps.lock().await, 1);
        assert_eq!(report.items_completed, 1);
        assert_eq!(
            report.items_already_satisfied, 0,
            "the work was done BY this run, not found already done"
        );
        assert!(artifact.exists());
        let completions = source.completions.lock().await;
        let (_, detail) = completions.first().expect("one completion");
        assert!(
            detail.get("already_satisfied").is_none(),
            "an ordinary completion, unmarked: {detail}"
        );
    }

    /// The pre-check is OPT-IN, and the default is the bound: on a first plan
    /// nothing has run yet, so a condition that already passes means the
    /// planner wrote a weak check — not that the user's request is moot.
    /// Completing there would delete real work, so the default runs the step.
    #[tokio::test]
    async fn the_pre_check_is_off_unless_the_caller_asks_for_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("artifact.txt"), b"already here").unwrap();
        let source = MemorySource::default();
        source
            .push(step(
                1,
                "produce the artifact",
                Some(AcceptanceCheck::FileExists {
                    path: "artifact.txt".to_string(),
                }),
            ))
            .await;
        let runner = ScriptedRunner::new(vec![Ok(outcome("had a go"))]);
        assert!(
            LongHorizonConfig::default()
                .precheck_acceptance_items
                .is_empty(),
            "off by default"
        );
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.steps_taken, 1, "the step ran: {report:?}");
        assert_eq!(report.items_completed, 1);
        assert_eq!(report.items_already_satisfied, 0);
    }

    /// THE PRE-CHECK MUST NEVER SWALLOW A LIVE USER MESSAGE.
    ///
    /// The pre-check is scoped to the ids the continuation planner seeded, and
    /// this is the reason. The harness polls the interjector at the top of
    /// every iteration, BEFORE `next()`, so a message the user sends during a
    /// continuation round is planned into a fresh item and selected inside the
    /// very round the pre-check is enabled for. A round-wide flag would cover
    /// it, and an interjected ask whose planned acceptance happened to pass
    /// already ("artifact.txt exists" — trivially true) would be completed
    /// with ZERO steps run: the user asked a question and got silence.
    ///
    /// Driven through the interjector rather than by calling the pre-check
    /// directly, because the path IS the bug: nothing about the item itself
    /// says "a user just asked for this", only where it came from.
    ///
    /// Both halves of the contract in one run — the seeded item is skipped and
    /// counted `already_satisfied` (so the caller's round still reads dry),
    /// while the interjected item, whose acceptance is the same trivially-true
    /// condition, runs its step.
    #[tokio::test]
    async fn an_interjected_item_runs_its_step_even_when_acceptance_already_passes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("artifact.txt"), b"already here").unwrap();
        let passing = || {
            Some(AcceptanceCheck::FileExists {
                path: "artifact.txt".to_string(),
            })
        };
        let source = Arc::new(MemorySource::default());
        source
            .push(step(1, "the continuation re-proposal", passing()))
            .await;
        // Nothing waiting when the seeded item is selected; the user's message
        // lands on the next boundary, mid-round.
        let interjector = ScriptedInterjector::new(
            source.clone(),
            vec![vec![], vec![step(2, "and what about the logs?", passing())]],
        );
        let runner = ScriptedRunner::new(vec![Ok(outcome("here are the logs\nTASK COMPLETE"))]);
        let config = LongHorizonConfig {
            // Only the continuation planner's seed is in scope.
            precheck_acceptance_items: HashSet::from([1]),
            ..fast_config()
        };

        let report = LongHorizonRunner::new(config)
            .run_with_interjector(
                "goal",
                source.as_ref(),
                &runner,
                dir.path(),
                None,
                Some(&interjector),
            )
            .await;

        assert_eq!(report.stop, StopReason::AllTasksDone);
        assert_eq!(report.interjected_items, 1);
        assert_eq!(report.items_completed, 2, "both closed: {report:?}");
        assert_eq!(
            report.items_already_satisfied, 1,
            "ONLY the seeded re-proposal was skipped for free: {report:?}"
        );
        assert_eq!(
            report.steps_taken, 1,
            "one step, and it belongs to the user's message: {report:?}"
        );
        let prompts: Vec<String> = runner
            .requests
            .lock()
            .await
            .iter()
            .map(|r| r.prompt.clone())
            .collect();
        assert_eq!(prompts.len(), 1);
        assert!(
            prompts[0].contains("and what about the logs?"),
            "the user's mid-round message must reach a step prompt: {prompts:?}"
        );
        let completions = source.completions.lock().await;
        let seeded = completions
            .iter()
            .find(|(id, _)| *id == 1)
            .expect("the seeded item closed");
        assert_eq!(seeded.1["already_satisfied"], serde_json::json!(true));
        let interjected = completions
            .iter()
            .find(|(id, _)| *id == 2)
            .expect("the interjected item closed");
        assert!(
            interjected.1.get("already_satisfied").is_none(),
            "the user's item closed by running, not by being skipped: {}",
            interjected.1
        );
    }

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
        assert_eq!(report.last_runner_error, None, "no error, nothing to carry");
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
        // The stalled item must be named so the subtasks are parented to it —
        // but as the OPERATION, not as `parent_id=1` inside a literal tool
        // signature. A prompt that spells out a call it may not ship the schema
        // for teaches the model to guess argument names.
        assert!(plan_steps[0].prompt.contains("children of task #1"));
        assert!(
            !plan_steps[0].prompt.contains("todo("),
            "the replan prompt must not name a tool call verbatim"
        );
        // Execute steps carry the item's tool scope and step bounds.
        let exec = requests
            .iter()
            .find(|r| r.step_kind == StepKind::Execute)
            .unwrap();
        assert_eq!(exec.tool_scope, vec!["exec".to_string()]);
        assert_eq!(exec.max_iterations, Some(fast_config().step_iterations));
    }

    /// Build a stalled item whose acceptance can never pass, with an
    /// open-children ledger the runner can report from.
    fn replan_fixture(per_replan: usize) -> (MemorySource, ReplanningRunner) {
        let ledger = Arc::new(Mutex::new(HashMap::new()));
        let source = MemorySource {
            subtasks: Some(ledger.clone()),
            ..MemorySource::default()
        };
        let runner = ReplanningRunner {
            // Long enough that the script never runs dry first — the
            // assertions are about how many steps the LADDER takes, not
            // about the script running out.
            inner: ScriptedRunner::new((0..10).map(|i| Ok(outcome(&format!("try {i}")))).collect()),
            subtasks: ledger,
            per_replan,
        };
        (source, runner)
    }

    /// REGRESSION (live 2026-08-02, session 05775d1d): a replan re-proposed
    /// the title the run had just abandoned, the store refused to resurrect
    /// it, and the unconditional grind-counter reset handed the item another
    /// full allowance of fruitless steps anyway. A replan that decomposed
    /// NOTHING has produced no new work, so it earns no fresh allowance —
    /// the item stays at its grind threshold and the existing
    /// `max_replans_per_item` rung reaches abandonment directly.
    #[tokio::test]
    async fn a_dry_replan_does_not_buy_a_fresh_allowance() {
        let dir = tempfile::tempdir().unwrap();
        let (source, runner) = replan_fixture(0);
        source
            .push(step(
                1,
                "stubborn item",
                Some(AcceptanceCheck::FileExists {
                    path: "never.txt".to_string(),
                }),
            ))
            .await;

        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;

        // 2 execute steps reach the grind threshold, 1 replan produces
        // nothing, and the next iteration abandons — no second allowance.
        assert_eq!(report.replans, 1);
        assert_eq!(report.items_abandoned, 1);
        assert_eq!(
            report.steps_taken, 3,
            "a dry replan must not restart the grind allowance: {report:?}"
        );
    }

    /// The other half of the contract: a replan that DID decompose still
    /// resets the counter, so real new work gets its full allowance.
    #[tokio::test]
    async fn a_productive_replan_still_resets_the_grind_counter() {
        let dir = tempfile::tempdir().unwrap();
        let (source, runner) = replan_fixture(2);
        source
            .push(step(
                1,
                "stubborn item",
                Some(AcceptanceCheck::FileExists {
                    path: "never.txt".to_string(),
                }),
            ))
            .await;

        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;

        // 2 execute + 1 replan + 2 more execute, then abandonment.
        assert_eq!(report.replans, 1);
        assert_eq!(report.items_abandoned, 1);
        assert_eq!(
            report.steps_taken, 5,
            "a replan that added subtasks keeps its fresh allowance: {report:?}"
        );
    }

    /// A source that cannot report its children is unchanged: `None` means
    /// unknown, and unknown is treated as productive rather than starving a
    /// legitimate decomposition.
    #[tokio::test]
    async fn a_source_that_cannot_report_children_keeps_the_old_behavior() {
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
        let runner =
            ScriptedRunner::new((0..10).map(|i| Ok(outcome(&format!("try {i}")))).collect());

        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.steps_taken, 5, "unreported children ⇒ productive");
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
        let cancel = CancelToken::new();
        cancel.cancel();
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), Some(cancel))
            .await;
        assert_eq!(report.stop, StopReason::Cancelled);
        assert_eq!(report.steps_taken, 0);
    }

    /// REGRESSION (GUI live drive, 2026-07-30): Stop had no effect for
    /// minutes because the flag reached only the harness boundary — the
    /// in-step agent loop got `cancel: None`. Every step request
    /// must carry the SAME flag the Stop button flips, so the runner can
    /// abort the in-flight stream, not just decline the next step.
    #[tokio::test]
    async fn every_step_request_carries_the_runs_cancel_flag() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "answer", None)).await;
        let runner = ScriptedRunner::new(vec![Ok(outcome("TASK COMPLETE"))]);
        let cancel = CancelToken::new();
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), Some(cancel.clone()))
            .await;
        assert_eq!(report.stop, StopReason::AllTasksDone, "{report:?}");

        let requests = runner.requests.lock().await;
        assert!(!requests.is_empty());
        for request in requests.iter() {
            let threaded = request
                .cancel
                .as_ref()
                .expect("the step request must carry the run's cancel flag");
            assert!(
                CancelToken::same_token(threaded, &cancel),
                "must be the SAME token the Stop button cancels, not a fresh one"
            );
        }
    }

    /// Cancels its step's own token mid-step and returns partial output —
    /// the scripted stand-in for the real agent loop observing
    /// `RunOptions.cancel` while a stream is in flight.
    struct CancelsDuringItsStep;

    #[async_trait::async_trait]
    impl StepRunner for CancelsDuringItsStep {
        async fn run_step(&self, request: StepRequest) -> Result<StepOutcome, String> {
            request
                .cancel
                .as_ref()
                .expect("the token is threaded into the step")
                .cancel();
            Ok(outcome("partial work\n\n[Cancelled by user]"))
        }
    }

    #[tokio::test]
    async fn a_cancellation_injected_mid_step_stops_the_run_at_that_step() {
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "first", None)).await;
        source.push(step(2, "second", None)).await;
        let cancel = CancelToken::new();
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &CancelsDuringItsStep, dir.path(), Some(cancel))
            .await;
        assert_eq!(report.stop, StopReason::Cancelled);
        assert_eq!(report.steps_taken, 1, "the second item must never start");
        assert!(
            source.completions.lock().await.is_empty(),
            "a truncated step is not a verdictable claim — nothing completes"
        );
        assert_eq!(
            source.notes.lock().await.len(),
            1,
            "the cancelled step's partial findings still land in the store"
        );
    }

    #[tokio::test]
    async fn poisoned_item_is_abandoned_and_the_run_continues() {
        // An item whose prompt deterministically breaks the model must cost
        // one feature, not the whole run: errors follow the item, the item
        // gets abandoned, and the next item proceeds.
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "poisoned", None)).await;
        source.push(step(2, "healthy", None)).await;
        let runner = ScriptedRunner::new(vec![
            Err("empty completion".to_string()),
            Err("empty completion".to_string()),
            Ok(outcome("TASK COMPLETE")),
        ]);
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.stop, StopReason::AllTasksDone, "{report:?}");
        assert_eq!(report.items_abandoned, 1, "poisoned item abandoned");
        assert_eq!(report.items_completed, 1, "healthy item still completed");
        assert_eq!(
            report.last_runner_error.as_deref(),
            Some("empty completion"),
            "the error that poisoned the item survives into the report"
        );
    }

    #[tokio::test]
    async fn deterministic_runner_error_reports_why_the_plan_drained() {
        // The live 2026-07-31 failure: a model whose provider is not
        // configured fails every run_step call identically. Poison
        // containment abandons the only item and the run exits AllTasksDone
        // with zero steps — numerically identical to a finished run. The
        // report must still carry the error so the caller can say WHY.
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "the user's prompt", None)).await;
        let runner = ScriptedRunner::new(vec![
            Err("No provider available for model 'claude-fable-5'".to_string()),
            Err("No provider available for model 'claude-fable-5'".to_string()),
        ]);
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert_eq!(report.stop, StopReason::AllTasksDone, "{report:?}");
        assert_eq!(report.steps_taken, 0);
        assert_eq!(report.items_completed, 0);
        assert_eq!(report.items_abandoned, 1);
        assert_eq!(
            report.last_runner_error.as_deref(),
            Some("No provider available for model 'claude-fable-5'"),
            "the report must name the fault that emptied the plan"
        );
    }

    #[tokio::test]
    async fn consecutive_runner_errors_trip_the_circuit_breaker() {
        // Provider-wide death: errors SPAN items (poison containment first
        // abandons the item they follow), so the run-level breaker trips
        // once the error streak crosses into a second item.
        let dir = tempfile::tempdir().unwrap();
        let source = MemorySource::default();
        source.push(step(1, "first", None)).await;
        source.push(step(2, "second", None)).await;
        let runner = ScriptedRunner::new(vec![
            Err("boom".to_string()),
            Err("boom".to_string()),
            Err("boom".to_string()),
        ]);
        let report = LongHorizonRunner::new(fast_config())
            .run("goal", &source, &runner, dir.path(), None)
            .await;
        assert!(matches!(report.stop, StopReason::RunnerErrors { .. }), "{report:?}");
        assert_eq!(report.steps_taken, 0, "failed steps are not progress");
        assert_eq!(report.items_abandoned, 1, "first item was contained as poisoned");
        assert_eq!(report.last_runner_error.as_deref(), Some("boom"));
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

    // -----------------------------------------------------------------
    // Interjection — user input joining a run at a step boundary
    // -----------------------------------------------------------------

    /// Admits a scripted batch of items the first time it is polled, by
    /// pushing them into the shared source (as the daemon impl does).
    struct ScriptedInterjector {
        source: Arc<MemorySource>,
        /// Batches to admit, one per poll; empty batches mean "nothing waiting".
        batches: Mutex<VecDeque<Vec<TaskStep>>>,
        polls: Mutex<usize>,
        fail_first: Mutex<bool>,
    }

    impl ScriptedInterjector {
        fn new(source: Arc<MemorySource>, batches: Vec<Vec<TaskStep>>) -> Self {
            Self {
                source,
                batches: Mutex::new(batches.into()),
                polls: Mutex::new(0),
                fail_first: Mutex::new(false),
            }
        }
    }

    #[async_trait::async_trait]
    impl Interjector for ScriptedInterjector {
        async fn interject(&self) -> Result<usize, String> {
            *self.polls.lock().await += 1;
            {
                let mut fail = self.fail_first.lock().await;
                if *fail {
                    *fail = false;
                    return Err("store unavailable".to_string());
                }
            }
            let batch = self.batches.lock().await.pop_front().unwrap_or_default();
            let admitted = batch.len();
            for step in batch {
                self.source.push(step).await;
            }
            Ok(admitted)
        }
    }

    #[tokio::test]
    async fn interjected_task_is_executed_in_the_same_run() {
        let source = Arc::new(MemorySource::default());
        source.push(step(1, "original work", None)).await;
        // Nothing waiting on the first boundary; a message lands on the second.
        let interjector = ScriptedInterjector::new(
            source.clone(),
            vec![vec![], vec![step(2, "the interjected ask", None)]],
        );
        let runner = ScriptedRunner::new(vec![Ok(outcome("did 1
TASK COMPLETE")), Ok(outcome("did 2
TASK COMPLETE"))]);

        let report = LongHorizonRunner::new(fast_config())
            .run_with_interjector(
                "goal",
                source.as_ref(),
                &runner,
                Path::new("."),
                None,
                Some(&interjector),
            )
            .await;

        assert_eq!(report.stop, StopReason::AllTasksDone);
        assert_eq!(report.interjected_items, 1);
        assert_eq!(report.items_completed, 2, "both items ran: {report:?}");
        let titles: Vec<String> = runner
            .requests
            .lock()
            .await
            .iter()
            .map(|r| r.prompt.clone())
            .collect();
        assert!(
            titles.iter().any(|p| p.contains("the interjected ask")),
            "the mid-run message must reach a step prompt"
        );
    }

    #[tokio::test]
    async fn interjection_is_polled_before_every_step_selection() {
        let source = Arc::new(MemorySource::default());
        source.push(step(1, "a", None)).await;
        source.push(step(2, "b", None)).await;
        let interjector = ScriptedInterjector::new(source.clone(), vec![]);
        let runner = ScriptedRunner::new(vec![Ok(outcome("x
TASK COMPLETE")), Ok(outcome("y
TASK COMPLETE"))]);

        LongHorizonRunner::new(fast_config())
            .run_with_interjector(
                "goal",
                source.as_ref(),
                &runner,
                Path::new("."),
                None,
                Some(&interjector),
            )
            .await;

        // Two steps plus the final poll that discovers the empty plan.
        assert_eq!(*interjector.polls.lock().await, 3);
    }

    #[tokio::test]
    async fn a_failing_interjection_does_not_kill_work_in_flight() {
        let source = Arc::new(MemorySource::default());
        source.push(step(1, "keep going", None)).await;
        let interjector = ScriptedInterjector::new(source.clone(), vec![]);
        *interjector.fail_first.lock().await = true;
        let runner = ScriptedRunner::new(vec![Ok(outcome("done
TASK COMPLETE"))]);

        let report = LongHorizonRunner::new(fast_config())
            .run_with_interjector(
                "goal",
                source.as_ref(),
                &runner,
                Path::new("."),
                None,
                Some(&interjector),
            )
            .await;

        assert_eq!(report.stop, StopReason::AllTasksDone);
        assert_eq!(report.items_completed, 1);
        assert_eq!(report.interjected_items, 0);
    }

    #[tokio::test]
    async fn interjection_revives_a_plan_that_had_drained() {
        // The message lands exactly as the plan empties — the boundary poll
        // happens BEFORE next(), so the run continues instead of reporting
        // AllTasksDone and making the user start over.
        let source = Arc::new(MemorySource::default());
        source.push(step(1, "only item", None)).await;
        let interjector = ScriptedInterjector::new(
            source.clone(),
            vec![vec![], vec![step(2, "arrived at the buzzer", None)]],
        );
        let runner = ScriptedRunner::new(vec![Ok(outcome("first
TASK COMPLETE")), Ok(outcome("second
TASK COMPLETE"))]);

        let report = LongHorizonRunner::new(fast_config())
            .run_with_interjector(
                "goal",
                source.as_ref(),
                &runner,
                Path::new("."),
                None,
                Some(&interjector),
            )
            .await;

        assert_eq!(report.items_completed, 2);
        assert_eq!(report.interjected_items, 1);
    }

    #[tokio::test]
    async fn run_without_an_interjector_behaves_exactly_as_before() {
        let source = Arc::new(MemorySource::default());
        source.push(step(1, "a", None)).await;
        let runner = ScriptedRunner::new(vec![Ok(outcome("x
TASK COMPLETE"))]);

        let report = LongHorizonRunner::new(fast_config())
            .run("goal", source.as_ref(), &runner, Path::new("."), None)
            .await;

        assert_eq!(report.stop, StopReason::AllTasksDone);
        assert_eq!(report.items_completed, 1);
        assert_eq!(report.interjected_items, 0);
    }
}
