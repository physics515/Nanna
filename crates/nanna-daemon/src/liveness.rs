//! Session-scoped liveness: the daemon's own answer to "working, wedged, or
//! finished" — without grepping logs.
//!
//! Motivation (2026-08-10 bench forensics): a dead daemon and a slow model
//! look identical from outside. The ministral leg's driver polled a corpse 14
//! times and scored it; leg 7 burned 4 hours on a mission that had been dead
//! nine minutes in, because the only stall signals available were proxies
//! (log bytes, global tool-execution counts) for facts the daemon already
//! knows. This module holds those facts per session, updated by the chat
//! sink's own event stream, and serves them three ways:
//!
//! 1. the **liveness beat** — a low-frequency log line + IPC event while a
//!    turn is in flight (cadence derived in [`beat_interval_secs`]),
//! 2. the **`session.liveness` IPC verb** — last step, last tool, last
//!    side-effecting tool, current stop state,
//! 3. the **repeat-completion escalation** — a turn that ends `AllTasksDone`
//!    with zero new side effects, immediately after a turn that did the same,
//!    is an escalation stated in the transcript, not a silent completion
//!    (the lfm leg declared itself done 28 times over four hours with
//!    nothing on disk).
//!
//! Side-effect classification is [`nanna_agent::is_work_evidence_tool`] — the
//! same single classification the completion-claim gate and the harness's
//! convergence signal use. A second list would drift.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use serde::Serialize;

/// How many beats must land inside any legally-silent stretch of a healthy
/// turn. 4 gives a watcher at least three chances to see "alive and lawfully
/// waiting" before the silence budget itself expires — one beat could be
/// racing the boundary, two give a single confirmation, three distinguish a
/// trend; more buys nothing but log noise.
const BEATS_PER_SILENCE_WINDOW: u64 = 4;

/// Beat cadence, DERIVED from the budgets that already govern how long a
/// healthy in-flight turn may legally produce nothing:
///
/// - [`nanna_llm::STREAM_READ_TIMEOUT_SECS`] (120s) — the transport's
///   declared silence tolerance between stream chunks; a healthy stream
///   never exceeds it, a dead one is killed at it.
/// - [`nanna_agent::harness::ACCEPTANCE_TIMEOUT_SECS_DEFAULT`] (120s) — the
///   default budget for a step's acceptance check, the other harness-owned
///   operation that legitimately holds the turn quiet.
///
/// The beat is the tighter of the two divided by [`BEATS_PER_SILENCE_WINDOW`]
/// (30s today). Anything longer risks a whole legal silence window passing
/// between beats, so "beats stopped" could mean "lawful quiet stretch" —
/// exactly the ambiguity the beat exists to remove. If either budget changes,
/// the cadence follows; there is no independent constant to go stale.
#[must_use]
pub fn beat_interval_secs() -> u64 {
    let silence_budget = nanna_llm::STREAM_READ_TIMEOUT_SECS
        .min(nanna_agent::harness::ACCEPTANCE_TIMEOUT_SECS_DEFAULT);
    (silence_budget / BEATS_PER_SILENCE_WINDOW).max(1)
}

/// A tool call's mark in the ledger: what and when, in both clocks (RFC3339
/// for humans and wire, monotonic for "seconds ago" that survives clock skew).
#[derive(Debug, Clone)]
struct ToolMark {
    name: String,
    at: String,
    mono: Instant,
}

impl ToolMark {
    fn now(name: &str) -> Self {
        Self {
            name: name.to_string(),
            at: now_rfc3339(),
            mono: Instant::now(),
        }
    }
}

/// How the previous turn ended, for the repeat-completion escalation and the
/// stop-state readout.
#[derive(Debug, Clone)]
struct StopMark {
    /// Snake-case stop kind (`all_tasks_done`, `cancelled`, ...).
    kind: String,
    at: String,
    /// Fingerprint of the user content that opened the finished turn, so the
    /// escalation only fires on a REPEATED REQUEST: two different questions
    /// both answered read-only are two honest completions, while the same
    /// mission re-sent into an unchanged world (the lfm/driver-resend shape,
    /// or a user hammering send at a spinner) is the corrosive one.
    content_fingerprint: u64,
}

/// Order-stable fingerprint of a turn's user content (whitespace-trimmed).
#[must_use]
pub fn content_fingerprint(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    content.trim().hash(&mut hasher);
    hasher.finish()
}

/// What the turn is currently doing, at the coarse phase level a watcher
/// needs. Finer detail rides in the derived `awaiting` string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// No turn in flight.
    Idle,
    /// Turn admitted; pre-model preparation is running (recall, workspace
    /// context, memory writes) — the stretch that stalls when embedding
    /// providers are benched, so it must be visible to the beat.
    Preparing,
    /// The planner is producing the task list.
    Planning,
    /// A step is in flight but nothing has streamed yet (LLM request sent,
    /// no token received — the phase a wedged stream parks in).
    StepPending,
    /// Assistant text is streaming.
    Streaming,
    /// Thinking/reasoning is streaming.
    Thinking,
    /// A tool call is executing.
    Tool,
}

impl Phase {
    /// Wire spelling, identical to the serde rename — for callers that need
    /// the phase as a plain string (log fields, the beat event).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Preparing => "preparing",
            Self::Planning => "planning",
            Self::StepPending => "step_pending",
            Self::Streaming => "streaming",
            Self::Thinking => "thinking",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug)]
struct LivenessState {
    running: bool,
    turn_started: Option<Instant>,
    turn_started_at: Option<String>,
    model: Option<String>,
    phase: Phase,
    last_event: Option<Instant>,
    step_index: Option<usize>,
    step_kind: Option<String>,
    step_label: Option<String>,
    tool_in_flight: Option<ToolMark>,
    last_tool: Option<ToolMark>,
    last_side_effect: Option<ToolMark>,
    turn_side_effects: u64,
    turn_stream_chars: u64,
    beats: u64,
    /// Fingerprint of the user content that opened the current turn.
    turn_fingerprint: u64,
    last_stop: Option<StopMark>,
    /// Consecutive `all_tasks_done` exits for the SAME request with zero side
    /// effects, AFTER the first one (i.e. the number of exact repeats so far).
    unchanged_done_repeats: u32,
}

impl LivenessState {
    fn new() -> Self {
        Self {
            running: false,
            turn_started: None,
            turn_started_at: None,
            model: None,
            phase: Phase::Idle,
            last_event: None,
            step_index: None,
            step_kind: None,
            step_label: None,
            tool_in_flight: None,
            last_tool: None,
            last_side_effect: None,
            turn_side_effects: 0,
            turn_stream_chars: 0,
            beats: 0,
            turn_fingerprint: 0,
            last_stop: None,
            unchanged_done_repeats: 0,
        }
    }
}

/// Wire/JSON snapshot of a session's liveness — the `session.liveness`
/// response body and the beat's payload source.
#[derive(Debug, Clone, Serialize)]
pub struct LivenessSnapshot {
    pub running: bool,
    pub phase: Phase,
    /// Human-oriented "what is it waiting on right now" line, e.g.
    /// `model output (ollama/qwen3.5:9b): last token 41s ago` or
    /// `tool exec running for 12s`.
    pub awaiting: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_started_at: Option<String>,
    /// Seconds since the turn started (0 when idle).
    pub elapsed_s: u64,
    /// Seconds since the last observed event of any kind in this turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_s: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<ToolMarkSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_side_effect: Option<ToolMarkSnapshot>,
    pub turn_side_effects: u64,
    pub turn_stream_chars: u64,
    pub beats: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_stop: Option<StopSnapshot>,
    pub unchanged_done_repeats: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolMarkSnapshot {
    pub name: String,
    pub at: String,
    pub secs_ago: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StopSnapshot {
    pub reason: String,
    pub at: String,
}

/// Per-session liveness ledger. Cheap sync updates (one uncontended mutex
/// lock) so the chat sink can stamp it from streaming callbacks.
pub struct SessionLiveness {
    state: Mutex<LivenessState>,
}

impl SessionLiveness {
    fn new() -> Self {
        Self {
            state: Mutex::new(LivenessState::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LivenessState> {
        // A poisoned liveness ledger must never take the turn down with it —
        // the data is diagnostic, monotone, and self-healing on next update.
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A turn was admitted for this session. Resets per-turn counters.
    /// `fingerprint` is [`content_fingerprint`] of the turn's user content.
    pub fn begin_turn(&self, model: Option<&str>, fingerprint: u64) {
        let mut s = self.lock();
        s.running = true;
        s.turn_started = Some(Instant::now());
        s.turn_started_at = Some(now_rfc3339());
        s.model = model.map(str::to_string);
        s.phase = Phase::Preparing;
        s.last_event = Some(Instant::now());
        s.step_index = None;
        s.step_kind = None;
        s.step_label = None;
        s.tool_in_flight = None;
        s.turn_side_effects = 0;
        s.turn_stream_chars = 0;
        s.beats = 0;
        s.turn_fingerprint = fingerprint;
    }

    /// The turn's model became known (the runner config is built after the
    /// prep phase, so `begin_turn` could not carry it).
    pub fn set_model(&self, model: &str) {
        self.lock().model = Some(model.to_string());
    }

    /// Preparation finished; the planner call is in flight.
    pub fn on_planning(&self) {
        let mut s = self.lock();
        s.phase = Phase::Planning;
        s.last_event = Some(Instant::now());
    }

    /// A harness step started (fires for quiet conversation-shaped items too).
    pub fn on_step(&self, step_index: usize, kind: &str, label: &str) {
        let mut s = self.lock();
        s.phase = Phase::StepPending;
        s.last_event = Some(Instant::now());
        s.step_index = Some(step_index);
        s.step_kind = Some(kind.to_string());
        s.step_label = Some(label.to_string());
        s.tool_in_flight = None;
    }

    /// Assistant text streamed.
    pub fn on_stream_delta(&self, chars: usize) {
        let mut s = self.lock();
        s.phase = Phase::Streaming;
        s.last_event = Some(Instant::now());
        s.turn_stream_chars += chars as u64;
    }

    /// Thinking/reasoning streamed.
    pub fn on_thinking(&self, chars: usize) {
        let mut s = self.lock();
        s.phase = Phase::Thinking;
        s.last_event = Some(Instant::now());
        s.turn_stream_chars += chars as u64;
    }

    pub fn on_tool_start(&self, name: &str) {
        let mut s = self.lock();
        s.phase = Phase::Tool;
        s.last_event = Some(Instant::now());
        let mark = ToolMark::now(name);
        s.tool_in_flight = Some(mark.clone());
        s.last_tool = Some(mark);
    }

    pub fn on_tool_end(&self, name: &str, success: bool) {
        let mut s = self.lock();
        s.last_event = Some(Instant::now());
        s.tool_in_flight = None;
        // Back to "request in flight" until the next delta says otherwise.
        s.phase = Phase::StepPending;
        if success && nanna_agent::is_work_evidence_tool(name) {
            s.last_side_effect = Some(ToolMark::now(name));
            s.turn_side_effects += 1;
        }
    }

    /// The turn finished. `stop_kind` is the snake-case stop reason.
    ///
    /// Returns `Some(n)` when this exit is the n-th consecutive REPEAT of an
    /// `all_tasks_done` exit for the SAME user content with zero side effects
    /// in between — the caller must state the escalation in the transcript
    /// rather than completing silently. Any side effect, a different stop
    /// reason, or different user content resets the streak (two different
    /// questions both answered read-only are two honest completions; only the
    /// same request re-completed against an unchanged world escalates).
    #[must_use = "a Some(n) repeat verdict must be surfaced in the transcript, not dropped"]
    pub fn finish_turn(&self, stop_kind: &str) -> Option<u32> {
        let mut s = self.lock();
        s.running = false;
        s.phase = Phase::Idle;
        s.tool_in_flight = None;

        let repeat = if stop_kind == "all_tasks_done" && s.turn_side_effects == 0 {
            match &s.last_stop {
                Some(prev)
                    if prev.kind == "all_tasks_done"
                        && prev.content_fingerprint == s.turn_fingerprint =>
                {
                    s.unchanged_done_repeats += 1;
                    Some(s.unchanged_done_repeats)
                }
                _ => {
                    s.unchanged_done_repeats = 0;
                    None
                }
            }
        } else {
            s.unchanged_done_repeats = 0;
            None
        };

        s.last_stop = Some(StopMark {
            kind: stop_kind.to_string(),
            at: now_rfc3339(),
            content_fingerprint: s.turn_fingerprint,
        });
        repeat
    }

    /// Count a beat and return the snapshot it should carry, or `None` when
    /// no turn is in flight (the beat task uses this as its quiet exit).
    pub fn beat(&self) -> Option<LivenessSnapshot> {
        {
            let mut s = self.lock();
            if !s.running {
                return None;
            }
            s.beats += 1;
        }
        Some(self.snapshot())
    }

    pub fn snapshot(&self) -> LivenessSnapshot {
        let s = self.lock();
        let elapsed_s = s
            .turn_started
            .filter(|_| s.running)
            .map_or(0, |t| t.elapsed().as_secs());
        let quiet_s = s
            .last_event
            .filter(|_| s.running)
            .map(|t| t.elapsed().as_secs());

        let awaiting = if !s.running {
            match &s.last_stop {
                Some(stop) => format!("idle — last turn ended {} at {}", stop.kind, stop.at),
                None => "idle — no turn this daemon lifetime".to_string(),
            }
        } else if let Some(tool) = &s.tool_in_flight {
            format!(
                "tool {} running for {}s",
                tool.name,
                tool.mono.elapsed().as_secs()
            )
        } else {
            let quiet = quiet_s.unwrap_or(0);
            let model = s.model.as_deref().unwrap_or("<unbound model>");
            match s.phase {
                Phase::Preparing => format!(
                    "preparing the turn (recall, workspace context): {quiet}s"
                ),
                Phase::Planning => format!("planning the turn ({quiet}s, model {model})"),
                Phase::StepPending => format!(
                    "LLM request in flight ({model}): {quiet}s, no output yet this step"
                ),
                Phase::Streaming | Phase::Thinking => format!(
                    "model output ({model}): last token {quiet}s ago, ~{} chars this turn",
                    s.turn_stream_chars
                ),
                Phase::Tool | Phase::Idle => format!("{quiet}s since last event"),
            }
        };

        LivenessSnapshot {
            running: s.running,
            phase: s.phase,
            awaiting,
            turn_started_at: s.turn_started_at.clone(),
            elapsed_s,
            quiet_s,
            model: s.model.clone(),
            step_index: s.step_index,
            step_kind: s.step_kind.clone(),
            step_label: s.step_label.clone(),
            last_tool: s.last_tool.as_ref().map(tool_mark_snapshot),
            last_side_effect: s.last_side_effect.as_ref().map(tool_mark_snapshot),
            turn_side_effects: s.turn_side_effects,
            turn_stream_chars: s.turn_stream_chars,
            beats: s.beats,
            last_stop: s.last_stop.as_ref().map(|stop| StopSnapshot {
                reason: stop.kind.clone(),
                at: stop.at.clone(),
            }),
            unchanged_done_repeats: s.unchanged_done_repeats,
        }
    }
}

fn tool_mark_snapshot(mark: &ToolMark) -> ToolMarkSnapshot {
    ToolMarkSnapshot {
        name: mark.name.clone(),
        at: mark.at.clone(),
        secs_ago: mark.mono.elapsed().as_secs(),
    }
}

/// All sessions' liveness ledgers. Entries persist for the daemon's lifetime
/// so cross-turn state (last stop, the repeat-completion streak) survives
/// between turns.
#[derive(Default)]
pub struct LivenessRegistry {
    sessions: RwLock<HashMap<String, Arc<SessionLiveness>>>,
}

impl LivenessRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Get-or-create the ledger for a session.
    pub fn handle(&self, session_id: &str) -> Arc<SessionLiveness> {
        if let Some(found) = self
            .sessions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session_id)
        {
            return found.clone();
        }
        self.sessions
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SessionLiveness::new()))
            .clone()
    }

    /// The ledger for a session, if any turn has ever touched it.
    #[must_use]
    pub fn get(&self, session_id: &str) -> Option<Arc<SessionLiveness>> {
        self.sessions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(session_id)
            .cloned()
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_interval_is_derived_and_sane() {
        let interval = beat_interval_secs();
        // Derivation, not a constant: a beat must fit BEATS_PER_SILENCE_WINDOW
        // times into the tightest legal-silence budget.
        let budget = nanna_llm::STREAM_READ_TIMEOUT_SECS
            .min(nanna_agent::harness::ACCEPTANCE_TIMEOUT_SECS_DEFAULT);
        assert_eq!(interval, budget / BEATS_PER_SILENCE_WINDOW);
        assert!(interval >= 1);
        assert!(
            interval * BEATS_PER_SILENCE_WINDOW <= budget,
            "beats must fit inside the silence budget"
        );
    }

    #[test]
    fn idle_session_snapshot_says_idle() {
        let live = SessionLiveness::new();
        let snap = live.snapshot();
        assert!(!snap.running);
        assert_eq!(snap.phase, Phase::Idle);
        assert!(snap.awaiting.contains("idle"));
        assert!(snap.quiet_s.is_none());
    }

    #[test]
    fn turn_lifecycle_updates_phase_and_tools() {
        let live = SessionLiveness::new();
        live.begin_turn(None, content_fingerprint("fix the bug"));
        assert!(live.snapshot().running);
        assert_eq!(live.snapshot().phase, Phase::Preparing);
        assert!(live.snapshot().awaiting.contains("preparing"));

        live.set_model("ollama/qwen3.5:9b");
        live.on_planning();
        assert_eq!(live.snapshot().phase, Phase::Planning);
        assert!(live.snapshot().awaiting.contains("ollama/qwen3.5:9b"));

        live.on_step(0, "working", "Fix the bug");
        assert_eq!(live.snapshot().phase, Phase::StepPending);
        assert!(live.snapshot().awaiting.contains("no output yet"));

        live.on_stream_delta(42);
        assert_eq!(live.snapshot().phase, Phase::Streaming);
        assert!(live.snapshot().awaiting.contains("ollama/qwen3.5:9b"));

        live.on_tool_start("read_file");
        let snap = live.snapshot();
        assert_eq!(snap.phase, Phase::Tool);
        assert!(snap.awaiting.contains("tool read_file running"));

        live.on_tool_end("read_file", true);
        let snap = live.snapshot();
        // read_file is not side-effecting — the ledger must not count it.
        assert_eq!(snap.turn_side_effects, 0);
        assert!(snap.last_side_effect.is_none());
        assert_eq!(snap.last_tool.as_ref().map(|t| t.name.as_str()), Some("read_file"));

        live.on_tool_start("write_file");
        live.on_tool_end("write_file", true);
        let snap = live.snapshot();
        assert_eq!(snap.turn_side_effects, 1);
        assert_eq!(
            snap.last_side_effect.as_ref().map(|t| t.name.as_str()),
            Some("write_file")
        );

        // A FAILED side-effecting call is not evidence the world changed.
        live.on_tool_start("exec");
        live.on_tool_end("exec", false);
        assert_eq!(live.snapshot().turn_side_effects, 1);

        assert_eq!(live.finish_turn("all_tasks_done"), None);
        let snap = live.snapshot();
        assert!(!snap.running);
        assert_eq!(snap.last_stop.as_ref().map(|s| s.reason.as_str()), Some("all_tasks_done"));
    }

    #[test]
    fn repeated_all_tasks_done_without_side_effects_escalates() {
        // The lfm shape: the same mission re-sent, "done" over and over with
        // nothing on disk.
        let live = SessionLiveness::new();
        let mission = content_fingerprint("Build minidb, work test_01..test_42 in order");

        live.begin_turn(None, mission);
        live.on_tool_start("write_file");
        live.on_tool_end("write_file", true);
        assert_eq!(live.finish_turn("all_tasks_done"), None, "first exit, with work");

        live.begin_turn(None, mission);
        assert_eq!(
            live.finish_turn("all_tasks_done"),
            Some(1),
            "same request re-completed with no new side effects is repeat #1"
        );

        live.begin_turn(None, mission);
        assert_eq!(live.finish_turn("all_tasks_done"), Some(2));
        assert_eq!(live.snapshot().unchanged_done_repeats, 2);

        // A side effect breaks the streak: the world changed.
        live.begin_turn(None, mission);
        live.on_tool_start("exec");
        live.on_tool_end("exec", true);
        assert_eq!(live.finish_turn("all_tasks_done"), None);
        assert_eq!(live.snapshot().unchanged_done_repeats, 0);
    }

    #[test]
    fn different_questions_do_not_escalate() {
        // Two ordinary read-only Q&A turns must stay silent completions —
        // the escalation is for a REPEATED request, not for answering twice.
        let live = SessionLiveness::new();
        live.begin_turn(None, content_fingerprint("what is in src/main.rs?"));
        assert_eq!(live.finish_turn("all_tasks_done"), None);
        live.begin_turn(None, content_fingerprint("and in src/lib.rs?"));
        assert_eq!(live.finish_turn("all_tasks_done"), None);
        assert_eq!(live.snapshot().unchanged_done_repeats, 0);
    }

    #[test]
    fn fingerprint_ignores_surrounding_whitespace() {
        assert_eq!(content_fingerprint("do it"), content_fingerprint("  do it \n"));
        assert_ne!(content_fingerprint("do it"), content_fingerprint("do it again"));
    }

    #[test]
    fn different_stop_reason_resets_the_streak() {
        let live = SessionLiveness::new();
        let same = content_fingerprint("run the mission");
        live.begin_turn(None, same);
        assert_eq!(live.finish_turn("all_tasks_done"), None);
        live.begin_turn(None, same);
        assert_eq!(live.finish_turn("all_tasks_done"), Some(1));

        live.begin_turn(None, same);
        assert_eq!(live.finish_turn("cancelled"), None);
        assert_eq!(live.snapshot().unchanged_done_repeats, 0);

        // After a cancel, a fresh all_tasks_done is a FIRST exit again.
        live.begin_turn(None, same);
        assert_eq!(live.finish_turn("all_tasks_done"), None);
    }

    #[test]
    fn beat_counts_only_while_running() {
        let live = SessionLiveness::new();
        assert!(live.beat().is_none(), "no beat for an idle session");

        live.begin_turn(None, 0);
        let snap = live.beat().expect("beat while running");
        assert_eq!(snap.beats, 1);
        assert!(live.beat().is_some());
        assert_eq!(live.snapshot().beats, 2);

        let _ = live.finish_turn("cancelled");
        assert!(live.beat().is_none(), "beats stop at turn end");
    }

    #[test]
    fn registry_persists_entries_across_turns() {
        let registry = LivenessRegistry::new();
        assert!(registry.get("s1").is_none());
        let same = content_fingerprint("the mission");
        let a = registry.handle("s1");
        a.begin_turn(None, same);
        let _ = a.finish_turn("all_tasks_done");
        // Same Arc on re-fetch: cross-turn state (the streak) survives.
        let b = registry.handle("s1");
        b.begin_turn(None, same);
        assert_eq!(b.finish_turn("all_tasks_done"), Some(1));
    }
}
