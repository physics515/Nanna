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
//! the chat path:
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
//!    Stop button works (`cancel` flips the shared flag the harness polls at
//!    step boundaries), and the full run timeline is persisted with the final
//!    message instead of evaporating with the stream.
//! 4. **Interject.** A message sent while a run is live does not queue behind
//!    it: it is admitted at the next step boundary and jumps the plan, so the
//!    user is answered at the first available opportunity rather than in
//!    however many hours the run takes.

use super::{ControlPlane, Value, json};
use crate::tasks::{
    AgentPlanner, AgentStepRunner, ChatSink, PendingMessages, SessionInterjector, TursoTaskSource,
    seed_plan,
};
use nanna_agent::harness::LongHorizonConfig;
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
    /// Returns `Ok(Some(message_id))` when a run was started — the run itself
    /// proceeds in a spawned task and the caller should ACK immediately.
    /// Returns `Ok(None)` when a run is already live for the session: the
    /// message was admitted to that run instead.
    pub(super) async fn run_chat_turn(
        &self,
        session_id: &str,
        content: &str,
        system_prompt: String,
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
        };

        let step_runner = AgentStepRunner {
            router: router.clone(),
            tools: tools.clone(),
            agent_config: agent.agent_config().await,
            system_prompt,
            workspace_root: workspace_root.clone(),
            stats: Some(self.model_stats.clone()),
            chat_sink: Some(sink),
        };
        // The planner shares the step runner's provider handling but must not
        // stream its JSON into the transcript — planning is not work to show.
        let planner_runner = AgentStepRunner {
            chat_sink: None,
            router: step_runner.router.clone(),
            tools: step_runner.tools.clone(),
            agent_config: step_runner.agent_config.clone(),
            system_prompt: step_runner.system_prompt.clone(),
            workspace_root: step_runner.workspace_root.clone(),
            stats: step_runner.stats.clone(),
        };
        let planner = Arc::new(AgentPlanner::new(Arc::new(planner_runner)));

        let sessions = self.sessions.clone();
        let session_id_owned = session_id.to_string();
        let content_owned = content.to_string();
        let message_id_for_run = message_id.clone();

        tokio::spawn(async move {
            let scope = "session".to_string();
            let scope_id = Some(session_id_owned.clone());

            let plan = planner.plan(&content_owned, None).await;
            tracing::info!(
                session_id = %session_id_owned,
                tasks = plan.tasks.len(),
                origin = ?plan.origin,
                "planned a chat turn"
            );

            let summary = match seed_plan(&storage, &scope, scope_id.as_deref(), &plan, false).await
            {
                Err(message) => {
                    tracing::warn!(%message, "could not seed the chat plan");
                    format!("_could not start the run: {message}_")
                }
                Ok(_ids) => {
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
                    };
                    let workdir = workspace_root.unwrap_or_else(|| PathBuf::from("."));
                    let config = LongHorizonConfig {
                        actor: "chat".to_string(),
                        ..LongHorizonConfig::default()
                    };

                    let report = nanna_agent::harness::LongHorizonRunner::new(config)
                        .run_with_interjector(
                            &content_owned,
                            &source,
                            &step_runner,
                            &workdir,
                            Some(run_handle.cancellation_flag.clone()),
                            Some(&interjector),
                        )
                        .await;

                    format!(
                        "_{} step{} · {} item{} completed{}_",
                        report.steps_taken,
                        if report.steps_taken == 1 { "" } else { "s" },
                        report.items_completed,
                        if report.items_completed == 1 { "" } else { "s" },
                        if report.interjected_items > 0 {
                            format!(" · {} interjected", report.interjected_items)
                        } else {
                            String::new()
                        },
                    )
                }
            };

            // Persist the WHOLE run — the timeline journal carries every
            // streamed step, so history survives navigation and restart
            // instead of collapsing to the summary line.
            let timeline = run_handle
                .timeline
                .lock()
                .map(|journal| journal.clone())
                .unwrap_or_default();
            sessions
                .add_full_message(
                    &session_id_owned,
                    crate::session::MessageRole::Assistant,
                    &summary,
                    Vec::new(),
                    None,
                    timeline,
                    None,
                )
                .await;

            let _ = event_tx.send(crate::protocol::Event::MessageEnd {
                session_id: session_id_owned.clone(),
                message_id: message_id_for_run,
                content: summary,
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
