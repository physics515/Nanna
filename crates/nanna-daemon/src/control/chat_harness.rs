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
    closed_task_ids, demote_in_progress, seed_continuation, seed_plan,
};
use nanna_agent::harness::{Interjector, LongHorizonConfig};
use nanna_storage::Storage;
use nanna_agent::planner::{PLAN_DESCRIPTION_MAX_BYTES, PLAN_GOAL_MAX_BYTES};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Continuation rounds one mission turn may run before it is forcibly ended.
///
/// Bound justification: the dry-round counter is the real exit; this only
/// brakes a planner that keeps inventing genuinely new titles forever, and 60
/// rounds of real new work is a session the user should be steering anyway.
const CONTINUATION_ROUNDS_MAX: usize = 60;

/// Consecutive rounds that plan no new work before the mission is done.
///
/// Bound justification: one dry round can be a planner hiccup; two in a row
/// means re-planning from the current store state finds nothing left — the
/// goal is as done as this planner can make it.
const CONTINUATION_DRY_ROUNDS: usize = 2;

/// Rounds a run may end in an ERROR stop and still be retried.
///
/// `AllTasksDone` used to be the only continuable stop, which meant one
/// transient Turso error in `TursoTaskSource::next`, or three consecutive
/// 502s tripping the runner's error cap, silently ended an overnight run. The
/// mission's termination criterion should be "no more work can be planned",
/// not "the last step happened to exit by the happy path".
const CONTINUATION_ERROR_ROUNDS: usize = 3;

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
        workspace_context: Option<String>,
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
            workspace_context,
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
            // The planner sees the same bounded reference: acting on ROADMAP
            // items nobody asked for is precisely a planning failure.
            workspace_context: step_runner.workspace_context.clone(),
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

            // Deterministic fail-fast: the planner and every harness step
            // resolve the provider the same way, so a model no provider
            // serves has already decided the whole turn. Without this check
            // the turn still "runs": the planner falls back to a single-task
            // plan, both step attempts fail identically, poison containment
            // cancels the item, and the run exits AllTasksDone with zero
            // steps and nothing streamed — the user sees their prompt struck
            // through as a cancelled task and no reply at all (observed live
            // 2026-07-31, priority set to bare "claude-fable-5" with only
            // OpenRouter configured). Say why instead, and seed no task that
            // is born dead.
            let model = step_runner.agent_config.model.clone();
            if step_runner.router.client_for_model(&model).is_none() {
                tracing::warn!(
                    %model,
                    "chat turn cannot run: no provider serves the configured model"
                );
                final_sink.delta(&format!(
                    "_could not run: no provider is configured for model '{model}'. \
                     Add the provider's credential in Settings — it registers \
                     live, no restart needed — or pick a model from an \
                     available provider in Settings → Models._"
                ));
            } else {
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

                // The turn-start boundary for continuation dedup: tasks
                // already closed NOW belong to history, and only titles
                // closed AFTER this snapshot count as work this turn did
                // (see `seed_continuation`). On a store error the baseline
                // degrades to empty, which OVER-filters — continuation rounds
                // then dedup against all of history and the mission ends
                // early rather than treadmilling.
                let closed_before_turn = closed_task_ids(&storage, &scope, scope_id.as_deref())
                    .await
                    .unwrap_or_default();

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
                            report.tool_calls += extra.tool_calls;
                            report.items_completed += extra.items_completed;
                            report.items_abandoned += extra.items_abandoned;
                            report.interjected_items += admitted + extra.interjected_items;
                            if extra.last_runner_error.is_some() {
                                report.last_runner_error = extra.last_runner_error;
                            }
                            // The drain segment ran last, so its stop describes
                            // where the turn actually ended up — same rule as
                            // `fold_reports`.
                            report.stop = extra.stop;
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
                            // Dedup against titles this turn already CLOSED: the
                            // finished copy cannot be reused (only OPEN titles
                            // dedupe at add), so a planner that re-emits the work
                            // it just completed would seed a fresh clone every
                            // round and the dry detector would never trip —
                            // observed live 2026-08-02 (session 7ccc455a), one
                            // question re-planned eleven times to ROUNDS_MAX. An
                            // all-duplicate plan seeds nothing and counts dry.
                            let seeded = seed_continuation(
                                &storage,
                                &scope,
                                scope_id.as_deref(),
                                &next_plan,
                                &closed_before_turn,
                            )
                            .await
                            .unwrap_or_default();
                            // Open work is counted, never peeked with `next()` —
                            // that CLAIMS an item, and a probe must not consume
                            // the work it is probing for.
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
                            report.items_abandoned += more.items_abandoned;
                            report.interjected_items += more.interjected_items;
                            if more.last_runner_error.is_some() {
                                report.last_runner_error = more.last_runner_error;
                            }
                            report.stop = more.stop;
                        }

                        // A run that failed must SAY it failed. Poison
                        // containment can drain the whole plan through
                        // abandonment and exit `AllTasksDone` having streamed
                        // nothing — which rendered as no reply at all, the
                        // user's prompt just struck through as a cancelled
                        // task (observed live 2026-07-31). Announce WHAT
                        // stopped the run and WHY, in the transcript the user
                        // is actually looking at.
                        if let Some(notice) = failure_notice(&report) {
                            tracing::warn!(
                                stop = ?report.stop,
                                last_runner_error = report.last_runner_error.as_deref(),
                                "chat run failed — surfacing the reason in the transcript"
                            );
                            final_sink.delta(&notice);
                        }

                        // Run mechanics are shown only when there was a real run:
                        // a single-step reply stays a plain reply.
                        let multi_step = report.steps_taken > 1
                            || report.items_completed > 1
                            || report.interjected_items > 0;
                        if multi_step {
                            final_sink.delta(&format!(
                                "\n\n_{} step{} · {} item{} completed{}{}_",
                                report.steps_taken,
                                if report.steps_taken == 1 { "" } else { "s" },
                                report.items_completed,
                                if report.items_completed == 1 { "" } else { "s" },
                                // Abandoned work is part of the run's honest
                                // arithmetic — "3 items completed" from a
                                // 5-item plan must not read as done.
                                if report.items_abandoned > 0 {
                                    format!(" · {} abandoned", report.items_abandoned)
                                } else {
                                    String::new()
                                },
                                if report.interjected_items > 0 {
                                    format!(" · {} interjected", report.interjected_items)
                                } else {
                                    String::new()
                                },
                            ));
                        }
                    }
                }
            }

            // A cancelled run's in-flight items must not auto-resume:
            // `next()` sorts `in_progress` first, so a leftover from a
            // stopped turn would silently outrank the user's next message
            // (observed live 2026-08-02: a cancelled turn's continuation task
            // ran before the fresh question). Demote them to pending — still
            // open, still shown to the planner as unfinished work per the
            // owner directive above, just no longer at the head of the queue.
            if run_handle.cancel.is_cancelled() {
                match demote_in_progress(&storage, &scope, scope_id.as_deref(), "chat").await {
                    Ok(0) => {}
                    Ok(demoted) => tracing::info!(
                        demoted,
                        "cancelled turn — in-flight items returned to pending"
                    ),
                    Err(message) => {
                        tracing::warn!(%message, "could not demote the cancelled turn's items");
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

/// The user-visible line for a run that ended in failure, or `None` when the
/// ending needs no announcement (success, or a deliberate stop like Cancelled
/// or an exhausted budget — those were asked for).
///
/// Two failure shapes are announced, per "summaries must announce
/// themselves" — WHAT stopped and WHY, stated where the user is looking:
/// - an error stop (`RunnerErrors` / `SourceError`);
/// - `AllTasksDone` that completed nothing while abandoning something — the
///   poison-containment path, where a deterministic runner fault drains the
///   plan through abandonment and exits by the happy path.
///
/// Partial runs (some items completed, some abandoned) are NOT an error
/// banner; the run-stats line carries the abandoned count instead.
fn failure_notice(report: &nanna_agent::harness::LongHorizonReport) -> Option<String> {
    use nanna_agent::harness::StopReason;
    let why = match &report.stop {
        StopReason::RunnerErrors { message } => format!("the model kept failing: {message}"),
        StopReason::SourceError { message } => format!("the task store failed: {message}"),
        StopReason::AllTasksDone
            if report.items_completed == 0 && report.items_abandoned > 0 =>
        {
            report.last_runner_error.clone().unwrap_or_else(|| {
                "every planned task was abandoned without completing".to_string()
            })
        }
        _ => return None,
    };
    // Separate from streamed step text when there was any; a run that never
    // took a step starts its (only) message cleanly.
    let sep = if report.steps_taken > 0 { "\n\n" } else { "" };
    let verb = if report.steps_taken == 0 {
        "could not run"
    } else {
        "could not finish"
    };
    Some(format!("{sep}_{verb}: {why}_"))
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

/// The claim marker `nanna_agent::harness::step_claims_completion` verdicts
/// on. The model emits it, so matching is case-insensitive.
const CLAIM_MARKER: &str = "TASK COMPLETE";

/// Loop-recovery steering markers the agent loop may inject into a turn.
/// Steering is harness-to-model, never conversation: everything from the
/// marker to the end of its line is harness-fabricated, so the line is cut at
/// the marker. Emitted by our own code, so matching is exact.
const NUDGE_MARKERS: &[&str] = &["[THINKING SPIRAL DETECTED]"];

/// Case-insensitive `strip_prefix` (ASCII markers only). `get` guards the
/// UTF-8 boundary a byte-length slice could split.
fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// Case-insensitive `strip_suffix` (ASCII markers only).
fn strip_suffix_ci<'a>(s: &'a str, suffix: &str) -> Option<&'a str> {
    let split = s.len().checked_sub(suffix.len())?;
    let tail = s.get(split..)?;
    tail.eq_ignore_ascii_case(suffix).then(|| &s[..split])
}

/// Per-line strip: `None` drops the line, `Some` keeps (possibly peeled)
/// text. Recursion terminates because every recursive call shrinks the line
/// by at least one marker length.
fn strip_marker_line(line: &str) -> Option<&str> {
    for marker in NUDGE_MARKERS {
        if let Some(pos) = line.find(marker) {
            let head = line[..pos].trim_end();
            return if head.is_empty() { None } else { strip_marker_line(head) };
        }
    }
    let trimmed = line.trim();
    if trimmed.eq_ignore_ascii_case(CLAIM_MARKER) {
        return None;
    }
    // Stream glue: the claim marker fused with a neighboring line when the
    // newline between them was lost (observed live 2026-08-02: "TASK
    // COMPLETE[THINKING SPIRAL DETECTED]..."). Glue never introduces spaces,
    // so a whitespace boundary reads as genuine prose and survives.
    if let Some(rest) = strip_prefix_ci(trimmed, CLAIM_MARKER) {
        if !rest.starts_with(char::is_whitespace) {
            return strip_marker_line(rest);
        }
    }
    if let Some(rest) = strip_suffix_ci(trimmed, CLAIM_MARKER) {
        if !rest.ends_with(char::is_whitespace) {
            return strip_marker_line(rest);
        }
    }
    Some(line)
}

/// Remove harness plumbing from user-visible text: the `TASK COMPLETE`
/// claim marker the harness verdicts on (a line matching
/// `nanna_agent::harness::step_claims_completion`'s predicate — trimmed,
/// case-insensitive, on its own line, plus the glued prefix/suffix shapes
/// stream fusion produces) and loop-steering nudge markers. Space-separated
/// inline mentions are left alone.
pub(super) fn strip_harness_markers(text: &str) -> String {
    let mut out: Vec<&str> = text.lines().filter_map(strip_marker_line).collect();
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
    use nanna_agent::harness::{LongHorizonReport, StopReason};

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

    fn report(
        stop: StopReason,
        steps: usize,
        completed: usize,
        abandoned: usize,
        last_err: Option<&str>,
    ) -> LongHorizonReport {
        LongHorizonReport {
            stop,
            steps_taken: steps,
            tool_calls: 0,
            items_completed: completed,
            items_completed_unverified: 0,
            items_abandoned: abandoned,
            last_runner_error: last_err.map(String::from),
            replans: 0,
            false_success_claims: 0,
            input_tokens: 0,
            output_tokens: 0,
            wall_clock_secs: 0,
            tokens_per_completed_item: None,
            interjected_items: 0,
        }
    }

    #[test]
    fn failure_notice_surfaces_a_fully_abandoned_plan() {
        // The live 2026-07-31 shape: provider fault → poison containment →
        // AllTasksDone with zero steps. The notice must carry the error.
        let r = report(
            StopReason::AllTasksDone,
            0,
            0,
            1,
            Some("No provider available for model 'claude-fable-5'"),
        );
        let notice = failure_notice(&r).expect("a drained-by-abandonment run must announce itself");
        assert!(notice.contains("could not run"), "{notice}");
        assert!(
            notice.contains("No provider available for model 'claude-fable-5'"),
            "{notice}"
        );
        assert!(
            !notice.starts_with('\n'),
            "nothing streamed, so no separator: {notice:?}"
        );
    }

    #[test]
    fn failure_notice_falls_back_when_no_runner_error_was_recorded() {
        // Fruitless-step abandonment records no runner error; the notice must
        // still say the plan was abandoned rather than staying silent.
        let notice = failure_notice(&report(StopReason::AllTasksDone, 0, 0, 2, None))
            .expect("abandonment without an error still announces itself");
        assert!(notice.contains("abandoned"), "{notice}");
    }

    #[test]
    fn failure_notice_reports_error_stops_and_separates_from_streamed_text() {
        let r = report(
            StopReason::RunnerErrors {
                message: "step error: 502".to_string(),
            },
            4,
            1,
            0,
            Some("step error: 502"),
        );
        let notice = failure_notice(&r).expect("an error stop is a failure");
        assert!(
            notice.starts_with("\n\n"),
            "steps streamed text — the notice needs a separator: {notice:?}"
        );
        assert!(notice.contains("could not finish"), "{notice}");
        assert!(notice.contains("502"), "{notice}");

        let r = report(
            StopReason::SourceError {
                message: "storage exploded".to_string(),
            },
            0,
            0,
            0,
            None,
        );
        let notice = failure_notice(&r).expect("a source error is a failure");
        assert!(notice.contains("task store"), "{notice}");
    }

    #[test]
    fn failure_notice_stays_quiet_on_success_and_deliberate_stops() {
        // Clean success.
        assert!(failure_notice(&report(StopReason::AllTasksDone, 3, 2, 0, None)).is_none());
        // The user pressed Stop — do not dress it up as a fault.
        assert!(failure_notice(&report(StopReason::Cancelled, 0, 0, 1, Some("x"))).is_none());
        // Budget stops were asked for.
        assert!(failure_notice(&report(StopReason::WallClockExhausted, 9, 3, 0, None)).is_none());
        // Partial completion: the run-stats line carries the abandoned count;
        // an error banner would overstate the failure.
        assert!(failure_notice(&report(StopReason::AllTasksDone, 5, 2, 1, Some("x"))).is_none());
    }

    /// REGRESSION (live 2026-08-02, session 7ccc455a): one conversational
    /// question ("difference between a mutex and a semaphore?") became an
    /// eleven-round treadmill — every continuation round re-planned the same
    /// goal, the previous copy was CLOSED so seeding created a fresh clone
    /// (tasks #2033..#2044), and the dry detector never tripped until
    /// `CONTINUATION_ROUNDS_MAX`. This drives the loop's exact round
    /// arithmetic against a real store with a planner that re-emits the same
    /// goal every round: with closed-title dedup, two consecutive
    /// all-duplicate rounds end the mission and the goal is worked ONCE.
    #[tokio::test]
    async fn two_all_duplicate_continuation_rounds_end_the_mission() {
        use crate::tasks::{closed_task_ids, seed_continuation, seed_plan};
        use nanna_agent::planner::Plan;

        let storage = Arc::new(Storage::in_memory().await.expect("storage"));
        let scope = "session";
        let scope_id = Some("s1");
        let goal = "Explain the difference between a mutex and a semaphore";

        // Turn start: snapshot the closed baseline, seed the initial plan,
        // and let the "run" complete its single task.
        let closed_before_turn = closed_task_ids(&storage, scope, scope_id)
            .await
            .expect("baseline");
        let ids = seed_plan(&storage, scope, scope_id, &Plan::single(goal), false)
            .await
            .expect("initial seed");
        for id in &ids {
            storage
                .tasks()
                .complete(*id, Some("test"), None)
                .await
                .expect("complete");
        }

        // The continuation loop's round accounting, with the planner
        // behaviour observed live: the same single-task plan every round.
        let mut dry_rounds = 0usize;
        let mut continuations = 0usize;
        while dry_rounds < CONTINUATION_DRY_ROUNDS && continuations < CONTINUATION_ROUNDS_MAX {
            continuations += 1;
            let seeded =
                seed_continuation(&storage, scope, scope_id, &Plan::single(goal), &closed_before_turn)
                    .await
                    .unwrap_or_default();
            if seeded.is_empty() {
                dry_rounds += 1;
                continue;
            }
            dry_rounds = 0;
            for id in &seeded {
                storage
                    .tasks()
                    .complete(*id, Some("test"), None)
                    .await
                    .expect("complete");
            }
        }

        assert_eq!(
            continuations, CONTINUATION_DRY_ROUNDS,
            "the mission ends after two dry rounds, not at ROUNDS_MAX"
        );
        let all = storage
            .tasks()
            .list(scope, scope_id, true)
            .await
            .expect("list");
        assert_eq!(
            all.len(),
            1,
            "the goal was worked once, never treadmilled: {:?}",
            all.iter().map(|t| &t.title).collect::<Vec<_>>()
        );
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
    fn strip_harness_markers_cuts_lines_at_nudge_markers() {
        // The whole nudge line is fabricated steering — dropped outright.
        assert_eq!(
            strip_harness_markers(
                "[THINKING SPIRAL DETECTED] I was overthinking. Let me act instead of deliberate."
            ),
            ""
        );
        // Stream glue onto real prose: the fabricated tail is cut, prose stays.
        assert_eq!(
            strip_harness_markers("checking the cache[THINKING SPIRAL DETECTED] I was overthinking."),
            "checking the cache"
        );
    }

    #[test]
    fn strip_harness_markers_peels_glued_claim_markers() {
        // The live 2026-08-02 shape: claim marker fused to a nudge that the
        // lost newline glued onto the same line.
        let text = "ran both checks concurrently.\n\nTASK COMPLETE[THINKING SPIRAL DETECTED] \
                    I was overthinking. Let me act instead of deliberate.";
        assert_eq!(strip_harness_markers(text), "ran both checks concurrently.");
        // Suffix glue: prose that lost its newline before the claim line.
        assert_eq!(
            strip_harness_markers("all checks green.TASK COMPLETE"),
            "all checks green."
        );
        // Doubled marker collapses to nothing.
        assert_eq!(strip_harness_markers("TASK COMPLETEtask complete"), "");
    }

    #[test]
    fn strip_harness_markers_keeps_space_separated_mentions() {
        // Glue never introduces spaces, so a whitespace boundary is prose —
        // even at the start or end of a line.
        let text = "TASK COMPLETE is the marker I emit\nwe are almost TASK COMPLETE";
        assert_eq!(strip_harness_markers(text), text);
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
