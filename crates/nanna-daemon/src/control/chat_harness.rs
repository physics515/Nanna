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
//! 2. **Run.** The harness drives the plan, streaming every step into the
//!    session transcript through the existing `MessageDelta` / `ToolStart` /
//!    `ToolEnd` events, so the work shows as it happens.
//! 3. **Interject.** A message sent while a run is live does not queue behind
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

/// Outcome of one long-horizon chat turn.
pub struct ChatRunOutcome {
    pub message_id: String,
    pub content: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub steps_taken: usize,
    pub items_completed: usize,
    pub interjected_items: usize,
    pub stop: String,
}

impl ControlPlane {
    /// Run one chat turn as a long-horizon harness run.
    ///
    /// Returns `Ok(None)` when a run is already live for the session: the
    /// message was admitted to that run instead, and the caller should
    /// acknowledge rather than wait for a second reply.
    pub(super) async fn run_chat_turn(
        &self,
        session_id: &str,
        content: &str,
        system_prompt: String,
        workspace_root: Option<PathBuf>,
    ) -> Result<Option<ChatRunOutcome>, String> {
        let (Some(agent), Some(router), Some(tools), Some(storage), Some(event_tx)) = (
            self.agent.as_ref(),
            self.router.as_ref(),
            self.tools.as_ref(),
            self.storage.as_ref(),
            self.event_tx.clone(),
        ) else {
            return Err("long-horizon chat requires agent, router, tools and storage".to_string());
        };

        let registry = &self.chat_runs;
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

        // From here every exit must release the slot.
        let result = self
            .drive_chat_run(
                session_id,
                content,
                system_prompt,
                workspace_root,
                agent,
                router,
                tools,
                storage,
                &event_tx,
                &pending,
            )
            .await;
        registry.release(session_id).await;
        result.map(Some)
    }

    #[allow(clippy::too_many_arguments)]
    async fn drive_chat_run(
        &self,
        session_id: &str,
        content: &str,
        system_prompt: String,
        workspace_root: Option<PathBuf>,
        agent: &Arc<crate::agent_service::AgentService>,
        router: &Arc<crate::llm_router::LlmRouter>,
        tools: &Arc<nanna_tools::ToolRegistry>,
        storage: &Arc<nanna_storage::Storage>,
        event_tx: &tokio::sync::broadcast::Sender<crate::protocol::Event>,
        pending: &Arc<PendingMessages>,
    ) -> Result<ChatRunOutcome, String> {
        let message_id = uuid::Uuid::new_v4().to_string();
        let _ = event_tx.send(crate::protocol::Event::MessageStart {
            session_id: session_id.to_string(),
            message_id: message_id.clone(),
        });

        let sink = ChatSink {
            session_id: session_id.to_string(),
            message_id: message_id.clone(),
            event_tx: event_tx.clone(),
        };

        let step_runner = Arc::new(AgentStepRunner {
            router: router.clone(),
            tools: tools.clone(),
            agent_config: agent.agent_config().await,
            system_prompt,
            workspace_root: workspace_root.clone(),
            stats: Some(self.model_stats.clone()),
            chat_sink: Some(sink),
        });

        // The planner shares the step runner's provider handling but must not
        // stream its JSON into the transcript — planning is not work to show.
        let planner = Arc::new(AgentPlanner::new(Arc::new(AgentStepRunner {
            chat_sink: None,
            ..clone_runner(&step_runner)
        })));

        let plan = planner.plan(content, None).await;
        tracing::info!(
            session_id,
            tasks = plan.tasks.len(),
            origin = ?plan.origin,
            "planned a chat turn"
        );

        let scope = "session";
        let scope_id = Some(session_id.to_string());
        seed_plan(storage, scope, scope_id.as_deref(), &plan, false).await?;

        let source = TursoTaskSource::new(
            storage.clone(),
            scope.to_string(),
            scope_id.clone(),
            "chat".to_string(),
            Some(event_tx.clone()),
        );
        let interjector = SessionInterjector {
            storage: storage.clone(),
            scope: scope.to_string(),
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
                content,
                &source,
                step_runner.as_ref(),
                &workdir,
                None,
                Some(&interjector),
            )
            .await;

        // The transcript already carries the streamed work; the persisted
        // message is a summary of the run, not a second copy of its output.
        let content = format!(
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
        );

        let _ = event_tx.send(crate::protocol::Event::MessageEnd {
            session_id: session_id.to_string(),
            message_id: message_id.clone(),
            content: content.clone(),
        });

        Ok(ChatRunOutcome {
            message_id,
            content,
            input_tokens: report.input_tokens,
            output_tokens: report.output_tokens,
            steps_taken: report.steps_taken,
            items_completed: report.items_completed,
            interjected_items: report.interjected_items,
            stop: serde_json::to_value(&report.stop)
                .ok()
                .and_then(|v| v.get("reason").and_then(|r| r.as_str().map(String::from)))
                .unwrap_or_else(|| "unknown".to_string()),
        })
    }
}

/// Shallow copy of a step runner (every field is cheap to clone: `Arc`s,
/// a config, and a prompt string). Used to build the planner's non-streaming
/// twin without threading the constructor arguments twice.
fn clone_runner(runner: &AgentStepRunner) -> AgentStepRunner {
    AgentStepRunner {
        router: runner.router.clone(),
        tools: runner.tools.clone(),
        agent_config: runner.agent_config.clone(),
        system_prompt: runner.system_prompt.clone(),
        workspace_root: runner.workspace_root.clone(),
        stats: runner.stats.clone(),
        chat_sink: runner.chat_sink.clone(),
    }
}

/// Shape the chat handler returns when a message joined a live run.
#[must_use]
pub fn interjected_response(session_id: &str, depth: usize) -> Value {
    json!({
        "status": "interjected",
        "session_id": session_id,
        "pending": depth,
        "message": "admitted to the run in progress at the next step boundary",
    })
}
