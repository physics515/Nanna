//! Long-horizon chat (P19): every chat turn is a harness run.
//!
//! Before this, chat and the long-horizon harness were two execution paths.
//! Chat called [`crate::agent_service::AgentService::chat_with_options`] — one
//! growing context, no continuation machinery, and a follow-up message
//! serialized behind a `tokio::Mutex` until the turn finished. The harness had
//! the continuation machinery (re-anchored O(1) steps, acceptance verdicts,
//! progress-or-replan) but nothing to execute, because nothing turned a
//! request into a plan.
//!
//! [`nanna_agent::planner`] closes that gap, so this module makes the harness
//! THE chat path — there is no other:
//!
//! 1. **Plan.** The turn's message becomes a plan. Conversation and questions
//!    yield one task with no acceptance check, which the harness runs as
//!    exactly one step — the same cost as the old path.
//! 2. **Run — fire and forget.** The Send is ACKed immediately and the run is
//!    driven by a spawned task; a run can last hours, and holding the IPC
//!    request open that long was observed to outlive the GUI client's grace
//!    period ("Received response for unknown request"). Every step streams
//!    into the transcript through the existing `MessageDelta` / `ToolStart` /
//!    `ToolEnd` events.
//! 3. **Recoverable.** The run registers with
//!    [`crate::agent_service::AgentService::register_external_run`], so
//!    navigation away and back rebuilds the live view (`get_run_state`), the
//!    Stop button works (`cancel` wakes the shared token, aborting the
//!    in-flight LLM stream and tool calls, and skipping planning/seeding),
//!    and the full run timeline is persisted with the final message instead
//!    of evaporating with the stream.
//! 4. **Interject.** A message sent while a run is live does not queue behind
//!    it: it is admitted at the next step boundary and jumps the plan, so the
//!    user is answered at the first available opportunity rather than in
//!    however many hours the run takes.
//!
//! **It must feel like chat.** The harness is machinery, not UI: a
//! single-task turn renders as a plain reply — no step banner, no run-stats
//! line — and the `TASK COMPLETE` claim marker the harness verdicts on is
//! stripped before anything is persisted. Run mechanics surface only when
//! there is a genuine multi-item run to attribute work to.

use super::{ControlPlane, Value, json};
use crate::session::{MessageRole, SessionMessage, TimelineItem};
use crate::tasks::{
    AgentPlanner, AgentStepRunner, ChatSink, PendingMessages, SessionInterjector, TursoTaskSource,
    seed_plan,
};
use nanna_agent::harness::{Interjector, LongHorizonConfig};
use nanna_storage::Storage;
use nanna_agent::planner::{PLAN_DESCRIPTION_MAX_BYTES, PLAN_GOAL_MAX_BYTES};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Per-session interjection intake, shared between the chat handler (which
/// pushes) and the live run's [`SessionInterjector`] (which drains).
///
/// Keyed by session id. An entry outlives its run so a message that lands in
/// the gap between two turns is not dropped.
#[derive(Debug, Default)]
pub struct ChatRunRegistry {
    pending: RwLock<HashMap<String, Arc<PendingMessages>>>,
    active: RwLock<HashMap<String, ()>>,
}

impl ChatRunRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The intake queue for a session, created on first use.
    pub async fn pending_for(&self, session_id: &str) -> Arc<PendingMessages> {
        if let Some(queue) = self.pending.read().await.get(session_id) {
            return queue.clone();
        }
        let mut pending = self.pending.write().await;
        pending
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(PendingMessages::new()))
            .clone()
    }

    /// Whether a harness run is currently live for this session.
    pub async fn is_active(&self, session_id: &str) -> bool {
        self.active.read().await.contains_key(session_id)
    }

    /// Claim the run slot. Returns false when one is already live — the
    /// caller must then interject instead of starting a second run.
    pub async fn try_claim(&self, session_id: &str) -> bool {
        let mut active = self.active.write().await;
        if active.contains_key(session_id) {
            return false;
        }
        active.insert(session_id.to_string(), ());
        true
    }

    /// Release the slot. Must run on every exit path, including errors.
    pub async fn release(&self, session_id: &str) {
        self.active.write().await.remove(session_id);
    }
}

impl ControlPlane {
    /// Start one chat turn as a long-horizon harness run.
    ///
    /// `conversation` is the bounded rendering of the session so far (see
    /// [`conversation_context`]); it is handed to the planner so a follow-up
    /// turn ("double it") plans against what was actually said.
    ///
    /// Returns `Ok(Some(message_id))` when a run was started — the run itself
    /// proceeds in a spawned task and the caller should ACK immediately.
    /// Returns `Ok(None)` when a run is already live for the session: the
    /// message was admitted to that run instead.
    pub(super) async fn run_chat_turn(
        &self,
        session_id: &str,
        content: &str,
        system_prompt: String,
        conversation: Option<String>,
        workspace_root: Option<PathBuf>,
    ) -> Result<Option<String>, String> {
        let (Some(agent), Some(router), Some(tools), Some(storage), Some(event_tx)) = (
            self.agent.clone(),
            self.router.clone(),
            self.tools.clone(),
            self.storage.clone(),
            self.event_tx.clone(),
        ) else {
            return Err("long-horizon chat requires agent, router, tools and storage".to_string());
        };

        let registry = self.chat_runs.clone();
        let pending = registry.pending_for(session_id).await;

        // A live run owns this session: the message joins it at the next step
        // boundary rather than starting a competing run.
        if !registry.try_claim(session_id).await {
            let depth = pending.push(content.to_string()).await;
            tracing::info!(
                session_id,
                depth,
                "message admitted to the live run at the next step boundary"
            );
            return Ok(None);
        }

        let message_id = uuid::Uuid::new_v4().to_string();
        let _ = event_tx.send(crate::protocol::Event::MessageStart {
            session_id: session_id.to_string(),
            message_id: message_id.clone(),
        });

        // Register the run so navigation recovery, Stop, and timeline
        // persistence work exactly as for the in-service chat path.
        let run_handle = agent.register_external_run(session_id).await;

        let sink = ChatSink {
            session_id: session_id.to_string(),
            message_id: message_id.clone(),
            event_tx: event_tx.clone(),
            run: Some(run_handle.clone()),
            // Parity with the retired direct path: chat tool calls feed the
            // stats tracker and the Turso time-series.
            tool_stats: Some(self.tool_stats.clone()),
            storage: Some(storage.clone()),
            quiet_item: Arc::new(std::sync::Mutex::new(None)),
        };
        // The finalizer needs the sink after the step runner takes ownership;
        // ChatSink is a bundle of shared handles, so a clone IS the same sink.
        let final_sink = sink.clone();

        // The active workspace scopes stored memories, so a run's observations
        // belong to the workspace they happened in. `services_workspace_id` is
        // the same handle the tool services use, so tools and memory agree.
        let active_workspace_id = match &self.services_workspace_id {
            Some(ws) => ws.read().await.clone(),
            None => None,
        };

        let step_runner = AgentStepRunner {
            discovered_tools: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
            router: router.clone(),
            tools: tools.clone(),
            agent_config: agent.agent_config().await,
            system_prompt,
            workspace_root: workspace_root.clone(),
            stats: Some(self.model_stats.clone()),
            chat_sink: Some(sink),
            // Tool results go to memory, a stub goes to context.
            memory: self.memory.clone(),
            workspace_id: active_workspace_id,
        };
        // The planner shares the step runner's provider handling but must not
        // stream its JSON into the transcript — planning is not work to show.
        let planner_runner = AgentStepRunner {
            discovered_tools: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
            chat_sink: None,
            router: step_runner.router.clone(),
            tools: step_runner.tools.clone(),
            agent_config: step_runner.agent_config.clone(),
            system_prompt: step_runner.system_prompt.clone(),
            workspace_root: step_runner.workspace_root.clone(),
            stats: step_runner.stats.clone(),
            // Planning calls no tools, so it has nothing to remember.
            memory: None,
            workspace_id: None,
        };
        let planner = Arc::new(AgentPlanner::new(Arc::new(planner_runner)));

        // Opt-in assistant auto-remember, matching the user-message side of
        // the Send handler.
        let auto_remember = self.config.read().await.memory.auto_remember_messages;
        let memory = self.memory.clone();

        let sessions = self.sessions.clone();
        let session_id_owned = session_id.to_string();
        let content_owned = content.to_string();
        let message_id_for_run = message_id.clone();

        tokio::spawn(async move {
            let scope = "session".to_string();
            let scope_id = Some(session_id_owned.clone());

            // Unfinished work from an earlier turn is INFORMATION FOR THE
            // MODEL, not an instruction to the harness. Owner directive
            // (2026-07-25): *"the model should decide to resume or answer
            // another question by the user … i don't think we should assume
            // that the user wants to resume."* Previously the store decided
            // silently: leftover items sorted ahead of the new plan, so a
            // fresh question waited behind stale work nobody re-confirmed.
            // Now the planner is shown what is outstanding and chooses.
            let outstanding = open_work_context(&storage, &scope, scope_id.as_deref()).await;
            let context = match (conversation.as_deref(), outstanding.as_deref()) {
                (Some(convo), Some(work)) => Some(format!("{convo}\n\n{work}")),
                (Some(convo), None) => Some(convo.to_string()),
                (None, Some(work)) => Some(work.to_string()),
                (None, None) => None,
            };

            let plan = planner
                .plan(&content_owned, context.as_deref(), Some(&run_handle.cancel))
                .await;
            tracing::info!(
                session_id = %session_id_owned,
                tasks = plan.tasks.len(),
                origin = ?plan.origin,
                "planned a chat turn"
            );

            // Stop pressed while planning ran: seed nothing and start no
            // work. An empty seed makes the harness run below a no-op — its
            // first cancel check fires before any step — so the turn falls
            // straight through to the persist/release tail.
            let seeded = if run_handle.cancel.is_cancelled() {
                tracing::info!(
                    session_id = %session_id_owned,
                    "cancelled during planning — skipping the run"
                );
                Ok(Vec::new())
            } else {
                seed_plan(&storage, &scope, scope_id.as_deref(), &plan, false).await
            };
            match seeded {
                Err(message) => {
                    tracing::warn!(%message, "could not seed the chat plan");
                    final_sink.delta(&format!("_could not start the run: {message}_"));
                }
                Ok(ids) => {
                    // A one-task plan is a conversation-shaped turn: mark its
                    // item quiet so the transcript reads as a plain reply,
                    // with no step banner. Items added later (interjections,
                    // replans) get banners — by then there IS a run to show.
                    if let (1, Some(id)) = (ids.len(), ids.first()) {
                        if let Ok(mut quiet) = final_sink.quiet_item.lock() {
                            *quiet = Some(*id);
                        }
                    }

                    let source = TursoTaskSource::new(
                        storage.clone(),
                        scope.clone(),
                        scope_id.clone(),
                        "chat".to_string(),
                        Some(event_tx.clone()),
                    );
                    let interjector = SessionInterjector {
                        storage: storage.clone(),
                        scope: scope.clone(),
                        scope_id: scope_id.clone(),
                        pending: pending.clone(),
                        planner: planner.clone(),
                        actor: "chat".to_string(),
                        event_tx: Some(event_tx.clone()),
                        cancel: Some(run_handle.cancel.clone()),
                    };
                    let workdir = workspace_root.unwrap_or_else(|| PathBuf::from("."));
                    let config = LongHorizonConfig {
                        actor: "chat".to_string(),
                        ..LongHorizonConfig::default()
                    };

                    // Drain-before-release. The interjector is only polled
                    // INSIDE the harness loop, before `next()`. A message that
                    // arrives after the final poll therefore lands in
                    // `pending` with no loop left to notice it: the run exits,
                    // the claim is released, and nothing ever starts a run for
                    // it — the user's message silently disappears. Observed
                    // live: "the queue doesn't seem to ever make it to the
                    // model even after the model reaches a stopping point."
                    //
                    // So: after the harness returns, re-check the queue and
                    // run again for whatever arrived late. Bounded by
                    // POST_RUN_DRAIN_MAX rather than `while !empty` — a user
                    // typing steadily could otherwise keep one turn alive
                    // forever, and any message past the bound is still safe in
                    // `pending` for the next turn to claim.
                    const POST_RUN_DRAIN_MAX: usize = 4;
                    let mut report = nanna_agent::harness::LongHorizonRunner::new(config.clone())
                        .run_with_interjector(
                            &content_owned,
                            &source,
                            &step_runner,
                            &workdir,
                            Some(run_handle.cancel.clone()),
                            Some(&interjector),
                        )
                        .await;

                    for sweep in 0..POST_RUN_DRAIN_MAX {
                        if run_handle.cancel.is_cancelled() {
                            break; // the user pressed Stop; do not start more work
                        }
                        let admitted = match interjector.interject().await {
                            Ok(0) => break,
                            Ok(n) => n,
                            Err(message) => {
                                tracing::warn!(%message, "post-run drain could not seed");
                                break;
                            }
                        };
                        tracing::info!(
                            sweep,
                            admitted,
                            "message arrived after the last step boundary — running it now"
                        );
                        let extra = nanna_agent::harness::LongHorizonRunner::new(config.clone())
                            .run_with_interjector(
                                &content_owned,
                                &source,
                                &step_runner,
                                &workdir,
                                Some(run_handle.cancel.clone()),
                                Some(&interjector),
                            )
                            .await;
                        report.steps_taken += extra.steps_taken;
                        report.items_completed += extra.items_completed;
                        report.interjected_items += admitted + extra.interjected_items;
                    }

                    // Keep a MISSION alive while there is still work to plan.
                    //
                    // `AllTasksDone` means "the current plan drained", not
                    // "the goal is met". A 9B planner decomposes a 42-feature
                    // build into a handful of tasks, so the turn used to end
                    // minutes in with the goal barely started — observed live
                    // 2026-07-27: ended normally at 15 minutes, 2 of 42
                    // acceptance checks passing. The multi-hour runs this is
                    // meant to match came from a fully seeded ladder, which
                    // chat never gets.
                    //
                    // So: re-plan from the CURRENT state (the store is the
                    // truth, and the workspace is on disk for the model to
                    // inspect) and keep going. Termination is loop-until-dry
                    // rather than a step count: when a planning round adds no
                    // new open work twice running, the goal is as done as this
                    // planner can make it.
                    //
                    // Only missions continue — but "mission" is about WORK
                    // DONE, not plan size. Gating on `ids.len() > 1` was
                    // wrong: this planner routinely emits ONE task for a
                    // 42-feature build ("Build minidb CLI by iterating
                    // through all 42 test files"), so the gate stayed shut on
                    // exactly the runs it exists for — observed 2026-07-27, a
                    // one-task plan ran 67 tool calls over 12 minutes and then
                    // ended at 4/42 with continuation never firing.
                    //
                    // `steps_taken > 1` was still not the honest signal, and it
                    // failed the same way one rung down. Observed 2026-07-28: a
                    // 42-feature build produced a ONE-task plan whose FIRST step
                    // made 13 tool calls, wrote a file, ran a test, and then
                    // marked its single task done. `ids.len() > 1` was false and
                    // `steps_taken > 1` was false, so the continuation loop
                    // never ran a single round and a mission meant to last hours
                    // ended 90 seconds in, at 1/42.
                    //
                    // Counting steps and items cannot separate a mission from a
                    // greeting, because both can be one of each. What separates
                    // them is whether the run ACTED: a conversational turn
                    // answers from the model's head and calls no tools, while
                    // anything that touched the world may have left work behind.
                    // So the question asked here is "did this run do anything?",
                    // and if it did, the loop below is allowed to ask whether
                    // more remains. It is a cheap question — the dry-round
                    // counter closes it out after two empty rounds.
                    const CONTINUATION_ROUNDS_MAX: usize = 60;
                    const CONTINUATION_DRY_ROUNDS: usize = 2;
                    /// Rounds a run may end in an ERROR stop and still be retried.
                    ///
                    /// `AllTasksDone` used to be the only continuable stop, which
                    /// meant one transient Turso error in `TursoTaskSource::next`,
                    /// or three consecutive 502s tripping the runner's error cap,
                    /// silently ended an overnight run. The mission's termination
                    /// criterion should be "no more work can be planned", not
                    /// "the last step happened to exit by the happy path".
                    const CONTINUATION_ERROR_ROUNDS: usize = 3;

                    // Say how the run ended, always. Every exit produces a report
                    // and it was simply discarded, so recovering WHY a run stopped
                    // took a database query. It should take a grep.
                    tracing::info!(
                        stop = ?report.stop,
                        steps = report.steps_taken,
                        tool_calls = report.tool_calls,
                        items = report.items_completed,
                        false_success = report.false_success_claims,
                        "chat harness run finished"
                    );

                    let is_mission =
                        ids.len() > 1 || report.steps_taken > 1 || report.tool_calls > 0;
                    let mut error_rounds = 0usize;
                    let mut dry_rounds = 0usize;
                    let mut continuations = 0usize;
                    while is_mission
                        && {
                            use nanna_agent::harness::StopReason;
                            match report.stop {
                                StopReason::AllTasksDone => true,
                                // Transient: the store hiccuped or the model
                                // failed a few times in a row. Worth another
                                // round, but bounded so a hard fault cannot spin.
                                StopReason::SourceError { .. } | StopReason::RunnerErrors { .. } => {
                                    error_rounds += 1;
                                    if error_rounds <= CONTINUATION_ERROR_ROUNDS {
                                        tracing::warn!(
                                            stop = ?report.stop,
                                            round = error_rounds,
                                            "run ended on an error — retrying rather than                                              abandoning the mission"
                                        );
                                        true
                                    } else {
                                        tracing::error!(
                                            stop = ?report.stop,
                                            "run keeps failing — giving up after {} error rounds",
                                            CONTINUATION_ERROR_ROUNDS
                                        );
                                        false
                                    }
                                }
                                // Deliberate: the user stopped it, or the budget
                                // is genuinely spent. Do not paper over these.
                                _ => false,
                            }
                        }
                        && dry_rounds < CONTINUATION_DRY_ROUNDS
                        && continuations < CONTINUATION_ROUNDS_MAX
                        && !run_handle.cancel.is_cancelled()
                    {
                        continuations += 1;
                        let outstanding =
                            open_work_context(&storage, &scope, scope_id.as_deref()).await;
                        let ctx = match (conversation.as_deref(), outstanding.as_deref()) {
                            (Some(c), Some(w)) => Some(format!("{c}\n\n{w}")),
                            (Some(c), None) => Some(c.to_string()),
                            (None, Some(w)) => Some(w.to_string()),
                            (None, None) => None,
                        };
                        let next_plan = planner
                            .plan(&content_owned, ctx.as_deref(), Some(&run_handle.cancel))
                            .await;
                        // Stop pressed while the continuation round planned:
                        // the mission is over — seed nothing.
                        if run_handle.cancel.is_cancelled() {
                            break;
                        }
                        let seeded = seed_plan(
                            &storage,
                            &scope,
                            scope_id.as_deref(),
                            &next_plan,
                            false,
                        )
                        .await
                        .unwrap_or_default();
                        // seed_plan reuses an existing open task for a repeated
                        // title, so "nothing new" shows up as an empty plan or
                        // as a store with nothing open. Counted, never peeked
                        // with `next()` — that CLAIMS an item, and a probe must
                        // not consume the work it is probing for.
                        let has_work = storage
                            .tasks()
                            .counts(&scope, scope_id.as_deref())
                            .await
                            .is_ok_and(|(open, _closed)| open > 0);
                        if seeded.is_empty() || !has_work {
                            dry_rounds += 1;
                            tracing::info!(
                                continuations,
                                dry_rounds,
                                "mission continuation planned no new work"
                            );
                            continue;
                        }
                        dry_rounds = 0;
                        tracing::info!(
                            continuations,
                            new_tasks = seeded.len(),
                            "mission continues — the goal is not done yet"
                        );
                        let more =
                            nanna_agent::harness::LongHorizonRunner::new(config.clone())
                                .run_with_interjector(
                                    &content_owned,
                                    &source,
                                    &step_runner,
                                    &workdir,
                                    Some(run_handle.cancel.clone()),
                                    Some(&interjector),
                                )
                                .await;
                        report.steps_taken += more.steps_taken;
                        report.tool_calls += more.tool_calls;
                        report.items_completed += more.items_completed;
                        report.interjected_items += more.interjected_items;
                        report.stop = more.stop;
                    }

                    // Run mechanics are shown only when there was a real run:
                    // a single-step reply stays a plain reply.
                    let multi_step = report.steps_taken > 1
                        || report.items_completed > 1
                        || report.interjected_items > 0;
                    if multi_step {
                        final_sink.delta(&format!(
                            "\n\n_{} step{} · {} item{} completed{}_",
                            report.steps_taken,
                            if report.steps_taken == 1 { "" } else { "s" },
                            report.items_completed,
                            if report.items_completed == 1 { "" } else { "s" },
                            if report.interjected_items > 0 {
                                format!(" · {} interjected", report.interjected_items)
                            } else {
                                String::new()
                            },
                        ));
                    }
                }
            }

            // Persist the WHOLE run: the message content is everything that
            // streamed (so conversation history and exports read like chat),
            // and the timeline journal carries the interleaved record.
            // Harness plumbing (the TASK COMPLETE claim marker) is stripped
            // from both — it is a verdict signal, not conversation.
            let full_text = run_handle.accumulated_text.read().await.clone();
            let content = strip_harness_markers(&full_text);
            let timeline = sanitize_timeline(
                run_handle
                    .timeline
                    .lock()
                    .map(|journal| journal.clone())
                    .unwrap_or_default(),
            );
            sessions
                .add_full_message(
                    &session_id_owned,
                    MessageRole::Assistant,
                    &content,
                    Vec::new(),
                    None,
                    timeline,
                    None,
                )
                .await;

            if auto_remember {
                if let Some(memory) = memory {
                    if content.split_whitespace().count() >= 3 {
                        if let Err(e) = memory
                            .remember_with_importance(&content, HashMap::new(), 1.0)
                            .await
                        {
                            tracing::debug!("Failed to auto-remember assistant response: {e}");
                        }
                    }
                }
            }

            let _ = event_tx.send(crate::protocol::Event::MessageEnd {
                session_id: session_id_owned.clone(),
                message_id: message_id_for_run,
                content,
            });

            // Every exit path releases both registrations — a leaked entry
            // would make the session look busy forever.
            agent.unregister_external_run(&session_id_owned).await;
            registry.release(&session_id_owned).await;
        });

        Ok(Some(message_id))
    }
}

/// Shape the chat handler returns when a message joined a live run.
/// `content` is present (empty) because the GUI command layer requires the
/// field on every non-error response.
#[must_use]
pub fn interjected_response(session_id: &str, depth: usize) -> Value {
    json!({
        "status": "interjected",
        "session_id": session_id,
        "pending": depth,
        "content": "",
        "message": "admitted to the run in progress at the next step boundary",
    })
}

/// Render this scope's still-open work so the PLANNER can decide what to do
/// with it — resume it, fold the new request into it, or leave it parked and
/// answer the question that was actually asked.
///
/// Deliberately framed as a decision for the model rather than a directive:
/// the store must not silently resume stale work just because it sorts first.
/// `None` when nothing is outstanding, so an ordinary turn carries no extra
/// prompt weight.
///
/// Bounded like every other injected context: at most
/// [`OPEN_WORK_MAX`] items, titles clamped, so a scope with a runaway plan
/// cannot crowd out the request itself.
async fn open_work_context(
    storage: &Arc<Storage>,
    scope: &str,
    scope_id: Option<&str>,
) -> Option<String> {
    /// Enough to convey what is parked without displacing the request. A
    /// model that needs the full list can read it with the todo tool.
    const OPEN_WORK_MAX: usize = 10;

    let open = storage.tasks().list(scope, scope_id, false).await.ok()?;
    if open.is_empty() {
        return None;
    }
    let total = open.len();
    let mut out = String::from(
        "## Unfinished work from earlier in this session\n\
         These tasks are still open. Decide for yourself whether the user's new message means \
         to continue them, to change them, or to set them aside and answer something else — \
         do NOT assume a resume was requested:\n",
    );
    for task in open.iter().take(OPEN_WORK_MAX) {
        let title = if task.title.len() > 120 {
            let end = task.title.floor_char_boundary(120);
            format!("{}…", &task.title[..end])
        } else {
            task.title.clone()
        };
        out.push_str(&format!("- #{} [{}] {}\n", task.id, task.status, title));
    }
    if total > OPEN_WORK_MAX {
        out.push_str(&format!(
            "- …and {} more (use the todo tool to see them all)\n",
            total - OPEN_WORK_MAX
        ));
    }
    Some(out)
}

/// Render recent conversation as role-tagged lines for prompt injection.
///
/// The harness re-anchors every step from the task store, so unlike the
/// retired direct path (which passed history as a message array) the
/// conversation must ride in the system prompt — without it, "double it"
/// after "what is 2+2?" plans against nothing.
///
/// Bounds — both derived from the planner's own limits so every consumer of
/// this rendering sees the same window:
/// - total ≤ [`PLAN_GOAL_MAX_BYTES`] (8 KiB): the planner clamps its context
///   slot to this constant, and anything larger would displace the step's
///   working context;
/// - each message ≤ [`PLAN_DESCRIPTION_MAX_BYTES`] (2 KiB): one giant paste
///   must not occupy the whole window — this guarantees at least the last
///   four turns always fit.
///
/// Newest messages win; a dropped prefix and clamped messages announce
/// themselves so a partial view is never mistaken for the whole.
pub(super) fn conversation_context(messages: &[SessionMessage]) -> Option<String> {
    const OMISSION_NOTE: &str = "[earlier conversation omitted]";

    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    let mut truncated = false;

    for message in messages.iter().rev() {
        let speaker = match message.role {
            MessageRole::User => "User",
            MessageRole::Assistant => "Nanna",
            // System/tool records are plumbing, not conversation.
            MessageRole::System | MessageRole::Tool => continue,
        };
        let text = message.content.trim();
        if text.is_empty() {
            continue;
        }

        let clamped = if text.len() > PLAN_DESCRIPTION_MAX_BYTES {
            let end = text.floor_char_boundary(PLAN_DESCRIPTION_MAX_BYTES);
            format!("{}… [message truncated]", &text[..end])
        } else {
            text.to_string()
        };
        let line = format!("{speaker}: {clamped}");

        if used + line.len() + 1 > PLAN_GOAL_MAX_BYTES {
            truncated = true;
            break;
        }
        used += line.len() + 1;
        lines.push(line);
    }

    if lines.is_empty() {
        return None;
    }
    if truncated {
        lines.push(OMISSION_NOTE.to_string());
    }
    lines.reverse();
    Some(lines.join("\n"))
}

/// Remove harness plumbing from user-visible text: the `TASK COMPLETE`
/// claim marker the harness verdicts on (a line matching
/// `nanna_agent::harness::step_claims_completion`'s predicate — trimmed,
/// case-insensitive, on its own line). Inline mentions are left alone; only
/// whole marker lines are dropped.
pub(super) fn strip_harness_markers(text: &str) -> String {
    let mut out: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().eq_ignore_ascii_case("TASK COMPLETE"))
        .collect();
    // Marker lines at the end often leave a dangling blank line behind them.
    while out.last().is_some_and(|line| line.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Apply [`strip_harness_markers`] to the journal's text entries, dropping
/// entries the strip empties out. Tool, thinking and fault entries pass
/// through untouched — they are records, not prose.
pub(super) fn sanitize_timeline(items: Vec<TimelineItem>) -> Vec<TimelineItem> {
    items
        .into_iter()
        .filter_map(|item| match item {
            TimelineItem::Text { content, at } => {
                let stripped = strip_harness_markers(&content);
                if stripped.trim().is_empty() {
                    None
                } else {
                    Some(TimelineItem::Text {
                        content: stripped,
                        at,
                    })
                }
            }
            other => Some(other),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn msg(role: MessageRole, content: &str) -> SessionMessage {
        SessionMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role,
            content: content.to_string(),
            timestamp: Utc::now(),
            tool_calls: Vec::new(),
            attachments: Vec::new(),
            reasoning: None,
            timeline: Vec::new(),
            usage: None,
        }
    }

    #[test]
    fn conversation_context_renders_roles_in_order() {
        let history = vec![
            msg(MessageRole::User, "what is 2+2?"),
            msg(MessageRole::Assistant, "4"),
        ];
        let rendered = conversation_context(&history).expect("non-empty history renders");
        assert_eq!(rendered, "User: what is 2+2?\nNanna: 4");
    }

    #[test]
    fn conversation_context_skips_system_tool_and_empty_messages() {
        let history = vec![
            msg(MessageRole::System, "internal prompt"),
            msg(MessageRole::User, "   "),
            msg(MessageRole::Tool, "tool record"),
        ];
        assert!(conversation_context(&history).is_none());
        assert!(conversation_context(&[]).is_none());
    }

    #[test]
    fn conversation_context_clamps_one_giant_message() {
        let giant = "x".repeat(PLAN_GOAL_MAX_BYTES * 2);
        let history = vec![msg(MessageRole::User, &giant), msg(MessageRole::User, "next")];
        let rendered = conversation_context(&history).expect("renders");
        // The giant message is clamped to the per-message bound, so BOTH
        // messages fit — one paste must not occupy the whole window.
        assert!(rendered.contains("… [message truncated]"));
        assert!(rendered.ends_with("User: next"));
        assert!(rendered.len() <= PLAN_GOAL_MAX_BYTES + "[earlier conversation omitted]\n".len());
    }

    #[test]
    fn conversation_context_keeps_newest_and_announces_dropped_prefix() {
        // Enough medium messages to overflow the total budget.
        let filler = "y".repeat(PLAN_DESCRIPTION_MAX_BYTES / 2);
        let history: Vec<SessionMessage> = (0..20)
            .map(|i| msg(MessageRole::User, &format!("m{i} {filler}")))
            .collect();
        let rendered = conversation_context(&history).expect("renders");
        assert!(rendered.starts_with("[earlier conversation omitted]"));
        // Newest message always survives.
        assert!(rendered.contains("m19 "));
        // Oldest was dropped.
        assert!(!rendered.contains("m0 "));
    }

    #[test]
    fn strip_harness_markers_removes_only_whole_marker_lines() {
        let text = "did the work\ntask complete\nTASK COMPLETE\nalmost TASK COMPLETE inline\n";
        let stripped = strip_harness_markers(text);
        assert_eq!(stripped, "did the work\nalmost TASK COMPLETE inline");
    }

    #[test]
    fn strip_harness_markers_trims_dangling_trailing_blanks() {
        assert_eq!(strip_harness_markers("answer: 4\n\nTASK COMPLETE\n"), "answer: 4");
        assert_eq!(strip_harness_markers("TASK COMPLETE"), "");
    }

    #[test]
    fn sanitize_timeline_strips_text_and_drops_emptied_entries() {
        let at = Utc::now().to_rfc3339();
        let items = vec![
            TimelineItem::Text {
                content: "hello\nTASK COMPLETE".to_string(),
                at: at.clone(),
            },
            TimelineItem::Text {
                content: "\nTASK COMPLETE\n".to_string(),
                at: at.clone(),
            },
            TimelineItem::Tool {
                call_id: "c1".to_string(),
                name: "exec".to_string(),
                input: None,
                output: Some("ok".to_string()),
                success: Some(true),
                duration_ms: Some(3),
                tokens: None,
                total_tokens: None,
                at: at.clone(),
            },
        ];
        let sanitized = sanitize_timeline(items);
        assert_eq!(sanitized.len(), 2);
        assert!(matches!(
            &sanitized[0],
            TimelineItem::Text { content, .. } if content == "hello"
        ));
        assert!(matches!(&sanitized[1], TimelineItem::Tool { .. }));
    }
}
