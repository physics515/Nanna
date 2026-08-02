//! Daemon Server - Main daemon orchestrator
//!
//! Combines IPC server, control plane, sessions, persistence, and all subsystems.

use crate::agent_service::{AgentService, AgentServiceConfig};
use crate::channels::{ChannelManager, ChannelsConfig};
use crate::control::ControlPlane;
use crate::embedding_router::{EmbeddingProviderInfo, EmbeddingRouter};
use crate::health::{DEFAULT_HEALTH_PORT, HealthServer, HealthState, PidFile};
use crate::ipc::{IpcServer, IpcServerConfig};
use crate::llm_router::LlmRouter;
use crate::memory_persistence::TursoMemoryPersistence;
use crate::persistence::PersistenceManager;
use crate::protocol::Response;
use crate::session::SessionManager;
use crate::webhook::{DEFAULT_WEBHOOK_PORT, WebhookConfig, WebhookServer};
use async_trait::async_trait;
use nanna_agent::ThinkingMode;
use nanna_channels::{
    ChannelId, IncomingMessage, MessageContent, MessageRouter as ChannelMessageRouter,
    Sender as ChannelSender, TelegramChannel,
};
use nanna_config::credentials::{self, SecureStore};
use nanna_memory::MemoryService;
use nanna_scripting::ServiceFn;
use nanna_tools::{AgentSpawner, ParentChannel, SpawnResult, ToolPolicy, ToolRegistry};
use std::collections::HashMap;
use std::path::PathBuf;

/// Heartbeat check-in prompt for the daemon scheduler.
///
/// Deliberately does **not** command the model to `Read HEARTBEAT.md` — that
/// drove a `read_file` tool call which, with no active workspace, resolved to
/// `~/HEARTBEAT.md` and hard-errored (`os error 2`) every heartbeat. P17 has
/// since retired the bespoke `HEARTBEAT.md` entirely (recurrence lives in
/// scheduled-task config), so the prompt now frames the heartbeat as running
/// due scheduled work — never reading instruction files off disk.
const DAEMON_HEARTBEAT_PROMPT: &str = "Heartbeat check-in. Run any due scheduled tasks. Do not read files from disk looking for instructions, and do not infer or repeat old tasks from prior chats. Review your current state, and if nothing needs attention, reply HEARTBEAT_OK.";

/// Concrete implementation of AgentSpawner that lives in the daemon
/// where it can create Agent instances with isolated context.
struct AgentSpawnerImpl {
    router: Arc<crate::llm_router::LlmRouter>,
    tools: Arc<ToolRegistry>,
    agent_config: nanna_agent::AgentConfig,
    system_prompt: String,
    workspace_root: Option<PathBuf>,
    workspace_context: Option<String>,
    /// Shared model stats tracker (sub-agents contribute to the same stats)
    stats: Option<nanna_agent::ModelStatsTracker>,
    /// Model fallback chain for sub-agents, in priority order — resolved via
    /// [`nanna_config::LlmConfig::effective_sub_agent_models`], so an empty
    /// sub-agent list already means "the main chat list". Never empty.
    sub_agent_models: Vec<String>,
}

#[async_trait]
impl AgentSpawner for AgentSpawnerImpl {
    async fn spawn(
        &self,
        prompt: &str,
        description: &str,
        max_iterations: Option<usize>,
    ) -> Result<SpawnResult, String> {
        use nanna_agent::{Agent, AgentContext, RunOptions};

        info!(description = description, max_iterations = ?max_iterations, "Spawning sub-agent");

        // Fallback chain: first working model wins. A candidate whose
        // provider is missing is skipped; a candidate whose run fails hands
        // the prompt to the next (a fresh agent — sub-agent runs are
        // idempotent by contract, the parent only consumes the final text).
        let candidates = if self.sub_agent_models.is_empty() {
            vec![self.agent_config.model.clone()]
        } else {
            self.sub_agent_models.clone()
        };

        let mut last_error = String::new();
        for (attempt, model_spec) in candidates.iter().enumerate() {
            let Some(llm_client) = self.router.client_for_model(model_spec) else {
                last_error = format!(
                    "No provider available for model '{}'. Available providers: {:?}",
                    model_spec,
                    self.router.available_providers()
                );
                warn!(model = %model_spec, "Sub-agent model has no provider — trying next");
                continue;
            };

            // Fresh isolated context per attempt: a failed candidate must not
            // leak partial state into the next one.
            let mut context = AgentContext::new(uuid::Uuid::new_v4().to_string())
                .with_system_prompt(&self.system_prompt);
            if let Some(ref ws_root) = self.workspace_root {
                context.workspace_root = Some(ws_root.clone());
            }
            if let Some(ref ws_ctx) = self.workspace_context {
                context.workspace_context = Some(ws_ctx.clone());
            }

            // Configure agent — sub-agents are full agents, no artificial iteration cap
            let mut config = self.agent_config.clone();
            config.max_iterations = max_iterations;
            let model_display = model_spec.clone(); // Full name (with provider prefix) for reporting
            // Strip provider prefix from model name for the actual API call
            config.model = LlmRouter::strip_model_prefix(model_spec);

            if attempt > 0 {
                info!(model = %model_spec, attempt, "Sub-agent model attempt");
            }

            let mut agent =
                Agent::new(config, llm_client, self.tools.clone()).with_context(context);

            // Share model stats tracker with sub-agents
            if let Some(ref tracker) = self.stats {
                agent = agent.with_stats(tracker.clone());
            }

            let options = RunOptions {
                max_iterations,
                is_sub_agent: true,
                all_tools_active: true,
                ..Default::default()
            };

            // Run without timeout — agent stops when done or cancelled
            match agent.run(prompt, options).await {
                Ok(result) => {
                    info!(
                        description = description,
                        model = %model_display,
                        iterations = result.iterations,
                        tool_calls = result.tool_calls.len(),
                        input_tokens = result.input_tokens,
                        output_tokens = result.output_tokens,
                        "Sub-agent completed"
                    );
                    return Ok(SpawnResult {
                        text: result.text,
                        iterations: result.iterations,
                        tool_calls: result.tool_calls.len(),
                        input_tokens: result.input_tokens,
                        output_tokens: result.output_tokens,
                        model: model_display,
                    });
                }
                Err(e) => {
                    last_error = format!("Sub-agent error on '{model_display}': {e}");
                    warn!(
                        model = %model_display,
                        error = %e,
                        remaining = candidates.len() - attempt - 1,
                        "Sub-agent model failed — falling back"
                    );
                }
            }
        }

        Err(format!(
            "All {} sub-agent model(s) failed ({:?}). Last error: {}",
            candidates.len(),
            candidates,
            last_error
        ))
    }
}

/// Concrete implementation of ParentChannel that lives in the daemon.
/// Allows sub-agents to ask their parent questions.
///
/// Instead of blocking on mailbox polling, this makes a lightweight LLM call
/// with the parent session's conversation context to answer the sub-agent's
/// question directly. This avoids deadlocks (parent is blocked on the task
/// tool while the sub-agent waits for a reply).
struct ParentChannelImpl {
    sessions: Arc<SessionManager>,
    event_tx: Option<tokio::sync::broadcast::Sender<crate::protocol::Event>>,
    router: Arc<crate::llm_router::LlmRouter>,
    /// Model to use for answering sub-agent questions (e.g. cheap/fast model)
    model: String,
}

#[async_trait]
impl ParentChannel for ParentChannelImpl {
    async fn ask_parent(
        &self,
        sub_session_id: &str,
        question: &str,
        _timeout_secs: u64,
    ) -> Result<String, String> {
        // Look up the sub-session to find its parent and task context
        let sub_info = self
            .sessions
            .get_sub_session(sub_session_id)
            .await
            .ok_or_else(|| {
                format!(
                    "Sub-session '{}' not found — ask_parent is only available to sub-agents",
                    sub_session_id
                )
            })?;

        let parent_id = sub_info
            .parent_id
            .clone()
            .ok_or_else(|| "This sub-agent has no parent session".to_string())?;

        let label = sub_info.label.clone();
        let task = sub_info.task.clone();

        // Emit event for GUI visibility
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(crate::protocol::Event::SubSessionQuestion {
                session_id: sub_session_id.to_string(),
                parent_id: Some(parent_id.clone()),
                label: label.clone(),
                question: question.to_string(),
            });
        }

        tracing::info!(
            sub_session = sub_session_id,
            parent = %parent_id,
            label = ?label,
            "Sub-agent asking parent: {}",
            question.chars().take(100).collect::<String>()
        );

        // Load parent session's recent conversation for context
        let parent_session = self
            .sessions
            .get(&parent_id)
            .await
            .ok_or_else(|| format!("Parent session '{}' not found", parent_id))?;

        let recent_messages: Vec<String> = parent_session
            .messages
            .iter()
            .rev()
            .take(20) // Last 20 messages for context
            .rev()
            .map(|m| {
                format!(
                    "[{}]: {}",
                    m.role.as_db_str(),
                    m.content.chars().take(500).collect::<String>()
                )
            })
            .collect();

        let context = recent_messages.join("\n");

        // Build a focused prompt to answer the sub-agent's question
        let prompt = format!(
            "You are answering a question from a sub-agent that was delegated a task.\n\n\
             ## Sub-agent task\n{}\n\n\
             ## Recent conversation context (parent session)\n{}\n\n\
             ## Sub-agent's question\n{}\n\n\
             Answer concisely and directly. Provide only the information the sub-agent needs \
             to continue its work. If you don't have enough context to answer, say so clearly.",
            task,
            if context.is_empty() {
                "(no prior conversation)".to_string()
            } else {
                context
            },
            question
        );

        // Make a lightweight LLM call to answer the question using parent context
        let model = &self.model;
        let llm_client = self
            .router
            .client_for_model(model)
            .ok_or_else(|| format!("No provider for model '{}'", model))?;

        let stripped_model = crate::llm_router::LlmRouter::strip_model_prefix(model);
        let request = nanna_llm::CompletionRequest {
            model: stripped_model,
            messages: vec![
                nanna_llm::Message::system(
                    "You are a helpful assistant answering questions from sub-agents. Be concise and precise.",
                ),
                nanna_llm::Message::user(&prompt),
            ],
            max_tokens: Some(2048),
            ..Default::default()
        };

        let answer = llm_client
            .complete(&request)
            .await
            .map_err(|e| format!("LLM call failed: {}", e))?;

        tracing::info!(
            sub_session = sub_session_id,
            answer_len = answer.len(),
            "Parent answered sub-agent question"
        );

        Ok(answer)
    }
}

use std::sync::Arc;
use std::time::Duration;

/// Build service closures for script tools.
///
/// These closures allow JS/TS tools to call back into Rust subsystems via `Nanna.service(name, params)`.
/// Shared session history that can be updated before each agent run.
/// This allows the `session.history` service to return messages for the current session.
pub type SharedSessionHistory = Arc<tokio::sync::RwLock<Vec<crate::session::SessionMessage>>>;

/// Resolve a memory handle — including one whose memory has since been
/// consolidated away by dreaming.
///
/// Context stubs quote a handle, so "the full result is in memory, use the
/// handle" is a promise the store must keep. But dreaming REPLACES clusters:
/// it writes one consolidated memory and forgets the originals. A handle
/// captured before that would dangle, and the model would be told a result it
/// was explicitly promised no longer exists — the worst possible answer,
/// because it reads as data loss when the content is actually still there in
/// generalised form.
///
/// Consolidation already records `consolidated_from` (the ids it absorbed),
/// so the trail exists; this follows it. Resolution order:
/// 1. the entry's own id,
/// 2. the `source_id` tag shared by the chunks of one tool result,
/// 3. **forwarding**: any memory that lists this handle in `consolidated_from`,
/// 4. an unambiguous id prefix (stubs carry a short id).
///
/// Forwarding is transitive by construction: a consolidation of a
/// consolidation carries the intermediate id, and the walk repeats. The hop
/// limit only stops a cycle that a corrupt store could otherwise turn into a
/// hang.
/// The whole text behind a handle: every chunk sharing its `source_id`, in order.
///
/// A large tool result is split into N rows that share one `source_id`, but
/// `resolve_memory_handle` returns a single row — so paging ran off the end of
/// chunk 1 and reported `truncated: false`, while the stub sitting in the
/// model's context promised the result "was stored whole in memory as {N}
/// chunk(s)" and that recall "returns the full text". A model reading a fifth
/// of a 42-case test run and being told nothing was missing will report on what
/// it saw. Reassemble, rather than keep a promise the retrieval path was not
/// keeping.
async fn assemble_handle_content(
    memory: &Arc<MemoryService>,
    entry: &nanna_memory::MemoryListEntry,
) -> String {
    let Some(source_id) = entry.metadata.get("source_id") else {
        return entry.content.clone();
    };
    let mut chunks: Vec<(usize, String)> = memory
        .list_all()
        .await
        .into_iter()
        .filter(|e| e.metadata.get("source_id").is_some_and(|s| s == source_id))
        .map(|e| {
            let idx = e
                .metadata
                .get("chunk")
                .and_then(|c| c.split('/').next()?.parse::<usize>().ok())
                .unwrap_or(1);
            (idx, e.content)
        })
        .collect();
    if chunks.len() <= 1 {
        return entry.content.clone();
    }
    chunks.sort_by_key(|(idx, _)| *idx);
    chunks
        .into_iter()
        .map(|(_, content)| content)
        .collect::<Vec<_>>()
        .join("\n")
}

async fn resolve_memory_handle(
    memory: &Arc<MemoryService>,
    handle: &str,
) -> Result<nanna_memory::MemoryListEntry, String> {
    const MAX_FORWARD_HOPS: usize = 8;

    let all = memory.list_all().await;
    let direct = |needle: &str| -> Option<nanna_memory::MemoryListEntry> {
        all.iter()
            .find(|e| e.id == needle)
            .or_else(|| {
                all.iter()
                    .find(|e| e.metadata.get("source_id").is_some_and(|s| s == needle))
            })
            .cloned()
    };

    if let Some(found) = direct(handle) {
        return Ok(found);
    }

    // Follow the consolidation trail: who absorbed this id?
    let mut needle = handle.to_string();
    for _ in 0..MAX_FORWARD_HOPS {
        let successor = all.iter().find(|e| {
            e.metadata
                .get("consolidated_from")
                .is_some_and(|sources| sources.split(',').any(|s| s.trim() == needle))
                || e.metadata
                    .get("sources")
                    .is_some_and(|sources| sources.split(',').any(|s| s.trim() == needle))
        });
        match successor {
            Some(entry) => {
                if let Some(found) = direct(&entry.id) {
                    return Ok(found);
                }
                needle = entry.id.clone();
            }
            None => break,
        }
    }

    all.iter()
        .find(|e| e.id.starts_with(handle))
        .cloned()
        .ok_or_else(|| {
            format!(
                "no memory matches handle '{handle}'. It was not found directly, and nothing \
                 in the store lists it as a source, so it was not consolidated into another \
                 memory either — it may predate this store or have been deleted outright."
            )
        })
}

fn build_script_services(
    memory: &Option<Arc<MemoryService>>,
    spawner: Option<Arc<dyn AgentSpawner + Send + Sync>>,
    session_history: SharedSessionHistory,
    workspace_id: Arc<tokio::sync::RwLock<Option<String>>>,
    storage: Option<Arc<nanna_storage::Storage>>,
    summarizer: Option<(Arc<crate::llm_router::LlmRouter>, Vec<String>)>,
) -> HashMap<String, ServiceFn> {
    use serde_json::{Value, json};

    let mut services: HashMap<String, ServiceFn> = HashMap::new();

    // Task store services (P15): the todo skill's backend. Only available
    // with storage — the skill falls back to its JSON file otherwise.
    if let Some(storage) = storage {
        services.extend(crate::tasks::build_task_services(
            storage,
            workspace_id.clone(),
        ));
    }

    // Memory services
    if let Some(mem) = memory {
        let mem_store = mem.clone();
        let ws_store = workspace_id.clone();
        services.insert(
            "memory.store".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_store.clone();
                let ws = ws_store.clone();
                Box::pin(async move {
                    let content = params
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tags: HashMap<String, String> = params
                        .get("tags")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let importance = params
                        .get("importance")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0) as f32;
                    let workspace = ws.read().await.clone();
                    match mem
                        .remember_scoped(&content, tags, importance, workspace)
                        .await
                    {
                        Ok((id, _)) => Ok(json!({"id": id})),
                        Err(e) => Err(e.to_string()),
                    }
                })
            }),
        );

        let mem_search = mem.clone();
        let ws_search = workspace_id.clone();
        services.insert(
            "memory.search".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_search.clone();
                let ws = ws_search.clone();
                Box::pin(async move {
                    let query = params
                        .get("query")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                    let workspace = ws.read().await;
                    match mem.recall_scoped(&query, workspace.as_deref()).await {
                        Ok(results) => {
                            let items: Vec<Value> = results
                                .into_iter()
                                .take(limit)
                                .map(
                                    |r| json!({"id": r.id, "content": r.content, "score": r.score}),
                                )
                                .collect();
                            Ok(Value::Array(items))
                        }
                        Err(e) => {
                            // If embedding is not configured, return empty results
                            // instead of an error so the agent can continue gracefully
                            let msg = e.to_string();
                            if msg.contains("embedding") || msg.contains("No embedding function") {
                                tracing::debug!("Memory search skipped: {}", msg);
                                Ok(Value::Array(vec![]))
                            } else {
                                Err(msg)
                            }
                        }
                    }
                })
            }),
        );

        // Alias: some tool scripts may call memory.embed instead of memory.store
        let mem_embed = mem.clone();
        let ws_embed = workspace_id.clone();
        services.insert(
            "memory.embed".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_embed.clone();
                let ws = ws_embed.clone();
                Box::pin(async move {
                    let content = params
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tags: HashMap<String, String> = params
                        .get("tags")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    let importance = params
                        .get("importance")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(1.0) as f32;
                    let workspace = ws.read().await.clone();
                    match mem
                        .remember_scoped(&content, tags, importance, workspace)
                        .await
                    {
                        Ok((id, _)) => Ok(json!({"id": id})),
                        Err(e) => Err(e.to_string()),
                    }
                })
            }),
        );

        let mem_delete = mem.clone();
        services.insert(
            "memory.delete".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_delete.clone();
                Box::pin(async move {
                    let id = params
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    match mem.forget(&id).await {
                        Ok(()) => Ok(json!({"deleted": true})),
                        Err(e) => Err(e.to_string()),
                    }
                })
            }),
        );

        let mem_list = mem.clone();
        services.insert(
            "memory.list".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_list.clone();
                Box::pin(async move {
                    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                    let all = mem.list_all().await;
                    let items: Vec<Value> = all
                        .into_iter()
                        .take(limit)
                        .map(|e| json!({"id": e.id, "content": e.content, "weight": e.weight}))
                        .collect();
                    Ok(Value::Array(items))
                })
            }),
        );
    }

    // memory.get — read ONE memory by id, with a byte range.
    //
    // The store had only "search by similarity" and "list everything", which
    // is why a tool result kept in memory could not be pointed at from
    // context: a stub naming an id had no way to dereference it. This is the
    // first piece of a file-like surface (read a range, later append/replace)
    // so "the full result lives in memory, a stub lives in context" actually
    // has a retrieval path.
    if let Some(mem) = memory {
        let mem_get = mem.clone();
        services.insert(
            "memory.get".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_get.clone();
                Box::pin(async move {
                    let id = params
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| "id is required".to_string())?
                        .to_string();
                    let offset =
                        params.get("offset").and_then(serde_json::Value::as_u64).unwrap_or(0) as usize;
                    // Default cap keeps a huge tool result from re-flooding
                    // the context the stub existed to protect.
                    let limit = params
                        .get("limit")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(4_000) as usize;

                    let entry = resolve_memory_handle(&mem, &id).await?;
                    let content = assemble_handle_content(&mem, &entry).await;

                    let total = content.len();
                    let start = offset.min(total);
                    let end = start.saturating_add(limit).min(total);
                    // Never split a UTF-8 char.
                    let mut s = start;
                    while s < total && !content.is_char_boundary(s) { s += 1; }
                    let mut e = end;
                    while e < total && !content.is_char_boundary(e) { e += 1; }

                    // If the handle forwarded, SAY SO. Silently returning a
                    // consolidated narration where raw output was asked for
                    // is how a model concludes its data was corrupted; the
                    // note explains that dreaming folded the original in and
                    // that nothing was lost, only generalised.
                    let forwarded = entry.id != id
                        && entry.metadata.get("source_id").is_none_or(|s| s != &id);
                    let mut out = json!({
                        "id": entry.id,
                        "content": &entry.content[s..e],
                        "offset": s,
                        "returned": e - s,
                        "total": total,
                        "truncated": e < total,
                    });
                    if forwarded {
                        out["forwarded_from"] = json!(id);
                        out["note"] = json!(format!(
                            "'{id}' was consolidated during dreaming; this is the memory that \
                             absorbed it ({}). The original text was generalised into this one, \
                             not deleted.",
                            entry.id
                        ));
                    }
                    Ok(out)
                })
            }),
        );
    }

    // memory.append / memory.replace — the write half of the file-like
    // surface. Without them a memory can only be created or forgotten, so a
    // record that has become WRONG (an action narrated as a fact, e.g.
    // "creating minidb.sh at D:\…" nine hours after that file stopped
    // existing) can only be duplicated or destroyed, never corrected. Append
    // gives a running record per subject instead of N disconnected islands.
    if let Some(mem) = memory {
        let mem_append = mem.clone();
        services.insert(
            "memory.append".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_append.clone();
                Box::pin(async move {
                    let handle = params
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| "id is required".to_string())?
                        .to_string();
                    let addition = params
                        .get("content")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| "content is required".to_string())?
                        .to_string();
                    let entry = resolve_memory_handle(&mem, &handle).await?;
                    let combined = format!("{}\n{addition}", entry.content);
                    mem.update_content(&entry.id, &combined)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(json!({ "id": entry.id, "total": combined.len() }))
                })
            }),
        );

        let mem_replace = mem.clone();
        services.insert(
            "memory.replace".to_string(),
            Arc::new(move |params: Value| {
                let mem = mem_replace.clone();
                Box::pin(async move {
                    let handle = params
                        .get("id")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| "id is required".to_string())?
                        .to_string();
                    let old = params
                        .get("old")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| "old is required".to_string())?
                        .to_string();
                    let new = params
                        .get("new")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let entry = resolve_memory_handle(&mem, &handle).await?;
                    let hits = entry.content.matches(&old).count();
                    if hits == 0 {
                        // Same contract as edit_file: refuse rather than
                        // guess, and say what is actually there.
                        let preview: String = entry.content.chars().take(160).collect();
                        return Err(format!(
                            "'{old}' does not appear in memory {}. Nothing was changed. It \
                             begins: {preview}",
                            entry.id
                        ));
                    }
                    let updated = entry.content.replace(&old, &new);
                    mem.update_content(&entry.id, &updated)
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(json!({
                        "id": entry.id,
                        "replaced": hits,
                        "total": updated.len(),
                    }))
                })
            }),
        );
    }

    // memory.summarize — concatenate texts and summarize them with the
    // summarization model chain. The `day_dream` tool is the model-facing
    // half: dreaming already does this on a schedule, and this lets the model
    // ask for it deliberately when it notices related fragments piling up.
    if let Some((router, models)) = summarizer {
        services.insert(
            "memory.summarize".to_string(),
            Arc::new(move |params: Value| {
                let router = router.clone();
                let models = models.clone();
                Box::pin(async move {
                    let texts: Vec<String> = params
                        .get("texts")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(std::string::ToString::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    if texts.is_empty() {
                        return Err("texts must be a non-empty array".to_string());
                    }
                    let joined = texts.join("

---

");
                    let summarize =
                        crate::dream_summarizer::summarize_with_failover(router, models);
                    let summary = summarize(joined).await?;
                    Ok(json!({ "summary": summary }))
                })
            }),
        );
    }

    // Agent spawner service
    if let Some(spawner) = spawner {
        services.insert(
            "agent.spawn".to_string(),
            Arc::new(move |params: Value| {
                let spawner = spawner.clone();
                Box::pin(async move {
                    let prompt = params
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let description = params
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("sub-task")
                        .to_string();
                    let max_iterations = params
                        .get("max_iterations")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as usize);
                    match spawner.spawn(&prompt, &description, max_iterations).await {
                        Ok(result) => Ok(json!({
                            "text": result.text,
                            "iterations": result.iterations,
                            "tool_calls": result.tool_calls,
                            "model": result.model,
                        })),
                        Err(e) => Err(e),
                    }
                })
            }),
        );
    }

    // Embedded Python interpreter (no system Python required)
    {
        use nanna_scripting::python::PythonEngine;
        let python_engine = Arc::new(PythonEngine::new());
        services.insert(
            "python.exec".to_string(),
            Arc::new(move |params: Value| {
                let engine = python_engine.clone();
                Box::pin(async move {
                    let code = params
                        .get("code")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let timeout = params.get("timeout").and_then(|v| v.as_u64()).unwrap_or(30);
                    let workdir = params
                        .get("workdir")
                        .and_then(|v| v.as_str())
                        .map(String::from);

                    match engine.execute(&code, workdir.as_deref(), timeout).await {
                        Ok(result) => Ok(json!({
                            "stdout": result.stdout,
                            "stderr": result.stderr,
                            "success": result.success,
                            "error": result.error,
                            "duration_ms": result.duration_ms,
                        })),
                        Err(e) => Err(e.to_string()),
                    }
                })
            }),
        );
    }

    // Session history service — returns recent messages from the current session.
    // The SharedSessionHistory is populated before each agent run.
    {
        let history = session_history;
        services.insert(
            "session.history".to_string(),
            Arc::new(move |params: Value| {
                let history = history.clone();
                Box::pin(async move {
                    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
                    let history = history.read().await;
                    let start = if history.len() > limit {
                        history.len() - limit
                    } else {
                        0
                    };
                    let messages: Vec<Value> = history[start..]
                        .iter()
                        .map(|msg| {
                            json!({
                                "role": format!("{:?}", msg.role).to_lowercase(),
                                "content": msg.content,
                                "timestamp": msg.timestamp.to_rfc3339(),
                            })
                        })
                        .collect();
                    Ok(json!(messages))
                })
            }),
        );
    }

    services
}
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Configuration for the daemon server
#[derive(Debug, Clone)]
pub struct DaemonConfig {
    /// IPC server configuration
    pub ipc: IpcServerConfig,
    /// Data directory
    pub data_dir: PathBuf,
    /// Log level
    pub log_level: String,
    /// Auto-save interval in seconds
    pub auto_save_interval_secs: u64,
    /// LLM configuration
    pub llm: LlmConfig,
    /// Agent configuration
    pub agent: AgentServiceConfig,
    /// Enable memory service (requires embedding provider)
    pub enable_memory: bool,
    /// Enable HTTP health server
    pub enable_health_server: bool,
    /// Health server port (default: 5148)
    pub health_port: u16,
    /// Enable PID file (prevents multiple instances)
    pub enable_pid_file: bool,
    /// Enable webhook server for inbound messages
    pub enable_webhook_server: bool,
    /// Webhook server port (default: 3000)
    pub webhook_port: u16,
    /// Webhook configuration
    pub webhook: WebhookConfig,
    /// Use TypeScript skill implementations instead of Rust builtins
    pub use_script_tools: bool,
    /// Directory containing tool scripts (resolved from env/config/default)
    pub tools_dir: Option<PathBuf>,
    /// Tool names the agent may call. `None` (or a list containing `"*"`) means
    /// "no allowlist — every tool is permitted". Mirrors `[tools] enabled`.
    pub tool_allowlist: Option<Vec<String>>,
    /// Tool names the agent may never call. Takes precedence over the allowlist.
    /// Mirrors `[tools] disabled` — this is the setting that makes a disabled
    /// tool actually stop executing (previously the list was parsed but ignored).
    pub tool_denylist: Vec<String>,
    /// Channel configurations (Telegram, Discord, Slack, etc.)
    pub channels: Option<nanna_config::ChannelsConfig>,
    /// Max fraction of memories the scheduled dream cycle may merge away in one
    /// run (mirrors `[memory] max_compression_ratio`). Threaded so automatic
    /// consolidation honors the same user setting the IPC-triggered path does.
    pub memory_max_compression_ratio: f32,
    /// Floor the scheduled dream cycle leaves after consolidating (mirrors
    /// `[memory] min_remaining_memories`).
    pub memory_min_remaining_memories: usize,
    /// Seconds of idle (no chat activity) before the scheduled dream cycle may
    /// run (mirrors `[memory] dream_idle_threshold_secs`). Gated via the shared
    /// [`ActivityClock`] + `nanna_memory::dream_trigger`.
    pub dream_idle_threshold_secs: u64,
    /// Live memory count that forces a dream cycle regardless of idle time
    /// (mirrors `[memory] dream_memory_pressure_count`; `0` disables).
    pub dream_memory_pressure_count: usize,
}

/// LLM provider configuration (multi-provider)
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// Primary provider (anthropic, openai, ollama) - used for single-provider mode
    pub provider: String,
    /// Anthropic API key
    pub anthropic_api_key: Option<String>,
    /// Anthropic OAuth access token (alternative to API key)
    pub anthropic_oauth_token: Option<String>,
    /// Whether to use OAuth token instead of API key for Anthropic
    pub anthropic_use_oauth: bool,
    /// OpenAI API key
    pub openai_api_key: Option<String>,
    /// OpenRouter API key
    pub openrouter_api_key: Option<String>,
    /// GitHub token (for GitHub Models)
    pub github_token: Option<String>,
    /// Ollama host
    pub ollama_host: String,
    /// Ollama API key (optional — for remote/authenticated instances)
    pub ollama_api_key: Option<String>,
    /// Legacy: API key field (for backwards compatibility)
    pub api_key: Option<String>,
}

/// A credential from the environment, falling back to the secure store.
///
/// The store is where the GUI puts keys the user types in, so a key that is
/// only ever read from the environment is a key the user cannot set. Anthropic
/// and OpenAI already got their store fallback further down this file; the
/// others never did, and OpenRouter's absence was load-bearing — the dream
/// summarizer is configured to OpenRouter models by default, so every
/// consolidation failed with `Missing API key for provider: OpenRouter` while
/// the key sat in the store the whole time. Dreaming had never once run.
fn credential(env_var: &str, store_key: &str) -> Option<String> {
    if let Ok(value) = std::env::var(env_var) {
        if !value.trim().is_empty() {
            return Some(value);
        }
    }
    SecureStore::new()
        .get(store_key)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

impl LlmConfig {
    /// Build the daemon's LLM credential view from the user config file.
    ///
    /// The single source of truth for this field mapping — used at boot
    /// ([`DaemonBuilder::from_nanna_config`]) and on every control-plane config
    /// mutation (`control::config`), so the provider set the router rebuilds
    /// from is always derived exactly the way boot derived it.
    #[must_use]
    pub fn from_nanna(config: &nanna_config::Config) -> Self {
        Self {
            provider: config.llm.provider.clone(),
            anthropic_api_key: config.llm.api_key.clone(),
            anthropic_oauth_token: config.llm.anthropic_oauth_token.clone(),
            anthropic_use_oauth: config.llm.anthropic_use_oauth,
            openai_api_key: config.llm.openai_api_key.clone(),
            openrouter_api_key: config.llm.openrouter_api_key.clone(),
            github_token: config.llm.github_token.clone(),
            // Ollama host is stored in memory config
            ollama_host: config.memory.ollama_host.clone(),
            ollama_api_key: config.llm.ollama_api_key.clone(),
            api_key: config.llm.api_key.clone(),
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: "anthropic".to_string(),
            anthropic_api_key: credential("ANTHROPIC_API_KEY", credentials::keys::ANTHROPIC_API_KEY),
            anthropic_oauth_token: None,
            anthropic_use_oauth: false,
            openai_api_key: credential("OPENAI_API_KEY", credentials::keys::OPENAI_API_KEY),
            openrouter_api_key: credential(
                "OPENROUTER_API_KEY",
                credentials::keys::OPENROUTER_API_KEY,
            ),
            github_token: credential("GITHUB_TOKEN", credentials::keys::GITHUB_TOKEN),
            ollama_host: "http://localhost:11434".to_string(),
            ollama_api_key: std::env::var("OLLAMA_API_KEY").ok(),
            api_key: None, // Legacy
        }
    }
}

impl Default for DaemonConfig {
    fn default() -> Self {
        let data_dir = nanna_config::project_dirs()
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("./data"));

        Self {
            ipc: IpcServerConfig::default(),
            data_dir,
            log_level: "info".to_string(),
            auto_save_interval_secs: 60,
            llm: LlmConfig::default(),
            agent: AgentServiceConfig::default(),
            enable_memory: true, // Enabled by default (requires embedding provider)
            enable_health_server: true,
            health_port: DEFAULT_HEALTH_PORT,
            enable_pid_file: true,
            enable_webhook_server: false, // Disabled by default (needs configuration)
            webhook_port: DEFAULT_WEBHOOK_PORT,
            webhook: WebhookConfig::default(),
            use_script_tools: true,
            tools_dir: None,
            tool_allowlist: None,
            tool_denylist: Vec::new(),
            channels: None,
            // Mirror ConsolidationConfig::default() (== nanna-config defaults).
            memory_max_compression_ratio: 0.50,
            memory_min_remaining_memories: 20,
            // Mirror DreamingConfig::default() (== nanna-config defaults).
            dream_idle_threshold_secs: 300,
            dream_memory_pressure_count: 5000,
        }
    }
}

/// Build the scheduled dream-cycle's [`ConsolidationConfig`] from the user's
/// memory settings, keeping automatic consolidation in lock-step with the
/// IPC-triggered path (see `control.rs`). The per-cluster content budget is
/// sized to the summarizer model's context window so the consolidation prompt
/// always fits it. Pure so it is unit-testable.
fn scheduled_consolidation_config(
    max_compression_ratio: f32,
    min_remaining_memories: usize,
    summarizer_context_window_tokens: usize,
) -> nanna_memory::ConsolidationConfig {
    nanna_memory::ConsolidationConfig {
        max_compression_ratio,
        min_remaining_memories,
        ..nanna_memory::ConsolidationConfig::default()
    }
    .with_summarizer_context_window(summarizer_context_window_tokens)
}


/// The main daemon server
pub struct DaemonServer {
    config: DaemonConfig,
    embedding: EmbeddingConfig,
    memory_path: Option<PathBuf>,
    _brave_api_key: Option<String>,
    sessions: Arc<SessionManager>,
    _control: Arc<ControlPlane>,
    ipc: Arc<IpcServer>,
    persistence: Arc<PersistenceManager>,
    shutdown_tx: broadcast::Sender<()>,
    /// PID file (prevents multiple instances)
    pid_file: Option<PidFile>,
    /// Log buffer for capturing daemon logs
    log_buffer: Option<crate::log_buffer::LogBuffer>,
    /// Shared storage for model stats persistence
    storage: Option<Arc<nanna_storage::Storage>>,
    /// Set when storage init quarantined + rebuilt a corrupt database file;
    /// surfaced on /status and broadcast as `Event::MemoryStoreRebuilt`.
    memory_recovery: Option<Arc<nanna_storage::RecoveryReport>>,
}

impl DaemonServer {
    /// Discover the embedding dimension by probing **the same router the live
    /// embed path uses**.
    ///
    /// Probing through the router (rather than a bespoke client built from
    /// `embedding.provider`) matters for two reasons:
    ///
    /// 1. The router resolves cloud keys from **config *or* env**, while a
    ///    bespoke client read only the env — so a key set in `config.toml`
    ///    probed as "missing" even though every real embed succeeded.
    /// 2. The router carries the Ollama fallback, so a probe survives an
    ///    unreachable/unkeyed cloud provider exactly like a real embed does.
    ///
    /// A failure here is **not** fatal — see the call site.
    async fn probe_embedding_dimension(router: &EmbeddingRouter) -> Result<usize, String> {
        let (embedding, _provider_changed) = router.embed_one("dimension probe").await?;
        if embedding.is_empty() {
            return Err("embedding provider returned an empty vector".to_string());
        }
        debug_assert!(!embedding.is_empty(), "probe returned a non-empty vector");
        Ok(embedding.len())
    }

    /// Create a new daemon server
    pub fn new(
        config: DaemonConfig,
        embedding: EmbeddingConfig,
        memory_path: Option<PathBuf>,
        brave_api_key: Option<String>,
    ) -> Self {
        let sessions = Arc::new(SessionManager::new());
        let control = Arc::new(ControlPlane::new(sessions.clone()));
        let ipc = Arc::new(IpcServer::new(config.ipc.clone()));
        let persistence = Arc::new(PersistenceManager::new(&config.data_dir));
        let (shutdown_tx, _) = broadcast::channel(1);

        // Create PID file if enabled
        let pid_file = if config.enable_pid_file {
            Some(PidFile::new(&config.data_dir))
        } else {
            None
        };

        Self {
            config,
            embedding,
            memory_path,
            _brave_api_key: brave_api_key,
            sessions,
            _control: control,
            ipc,
            persistence,
            shutdown_tx,
            pid_file,
            log_buffer: None,
            storage: None,
            memory_recovery: None,
        }
    }

    /// Recovery report from a startup quarantine + rebuild, if one happened.
    pub fn memory_recovery(&self) -> Option<Arc<nanna_storage::RecoveryReport>> {
        self.memory_recovery.clone()
    }

    /// Set the storage backend for model stats persistence and session persistence.
    pub fn set_storage(&mut self, storage: Arc<nanna_storage::Storage>) {
        // Replace the SessionManager with one that has storage
        let new_sessions = Arc::new(SessionManager::with_storage(storage.clone()));
        self.sessions = new_sessions.clone();
        // Update control plane reference
        self._control = Arc::new(ControlPlane::new(self.sessions.clone()));
        self.storage = Some(storage);
    }

    /// Get the shutdown sender (for signaling shutdown)
    pub fn shutdown_handle(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }

    /// Get the IPC server address
    pub fn ipc_address(&self) -> String {
        self.ipc.address()
    }

    /// Run the daemon server
    pub async fn run(&mut self) -> Result<(), crate::DaemonError> {
        info!("Starting Nanna daemon...");
        info!("Data directory: {:?}", self.config.data_dir);

        // Adopt a kill-on-close Job Object BEFORE anything can spawn a child:
        // every exec/acceptance process inherits membership, so the OS reaps
        // the whole tree when this process dies for any reason — clean stop,
        // `taskkill /F`, or a crash. Closes the leak where daemon restarts
        // orphaned in-flight `powershell.exe`/`bash.exe` children (89 counted
        // on 2026-08-01). Survivable on failure: log and run uncontained.
        #[cfg(windows)]
        if crate::job::adopt_kill_on_close_job() {
            info!("Job Object adopted — child processes cannot outlive the daemon");
        } else {
            warn!("Job Object adoption failed — exec children may outlive an unclean daemon exit");
        }

        // Ensure data directory exists
        std::fs::create_dir_all(&self.config.data_dir)?;

        // Acquire PID file to prevent multiple instances
        if let Some(ref pid_file) = self.pid_file {
            match pid_file.acquire() {
                Ok(()) => {
                    info!("PID file acquired at {:?}", pid_file.path());
                }
                Err(crate::health::PidFileError::AlreadyRunning(pid)) => {
                    error!("Another daemon instance is already running (PID: {})", pid);
                    return Err(crate::DaemonError::AlreadyRunning);
                }
                Err(e) => {
                    warn!("Failed to acquire PID file: {}. Continuing anyway.", e);
                }
            }
        }

        // Load sessions from Turso database
        {
            let loaded = self.sessions.load_from_db().await;
            info!("Loaded {} sessions from database", loaded);
        }

        // If no sessions loaded from DB, check for legacy sessions.json migration
        if self.sessions.count().await == 0 {
            if let Some((sessions, default_id)) = self.persistence.load_legacy_sessions().await {
                if !sessions.is_empty() {
                    info!(
                        "Migrating {} sessions from legacy sessions.json to database",
                        sessions.len()
                    );
                    for session in sessions {
                        self.sessions.restore(session).await;
                    }
                    if let Some(id) = default_id {
                        self.sessions.set_default(&id).await;
                    }
                    // Mark as migrated
                    self.persistence.mark_sessions_migrated().await;
                }
            }
        }

        // Create default session if none exist
        if self.sessions.count().await == 0 {
            let default_session = self.sessions.create(Some("Main".to_string())).await;
            info!("Created default session: {}", default_session.id);
        }

        // Initialize services
        let (tools, memory, agent, router, tools_dir, workspace_id_for_services, model_stats) =
            self.init_services().await?;

        // Recover any orphaned checkpoints from the database.
        if let Some(ref storage) = self.storage {
            match storage.list_checkpoints().await {
                Ok(checkpoint_ids) => {
                    for session_id in checkpoint_ids {
                        // Load checkpoint data from DB and parse it. The
                        // checkpoint is deleted only after a SUCCESSFUL
                        // recovery (or when it holds nothing recoverable) —
                        // deleting before/regardless of the parse made any
                        // recovery failure a permanent data loss.
                        let mut recovered = false;
                        if let Ok(Some(data)) = storage.load_checkpoint(&session_id).await {
                            if let Some(partial) = agent.recover_checkpoint_from_data(&data) {
                                let reasoning = partial.reasoning.clone();
                                self.sessions
                                    .add_full_message(
                                        &session_id,
                                        crate::session::MessageRole::Assistant,
                                        &partial.content,
                                        partial.tool_calls,
                                        reasoning,
                                        partial.timeline,
                                        partial.usage,
                                    )
                                    .await;
                                info!("Recovered crashed run for session {}", session_id);
                                recovered = true;
                            } else if serde_json::from_str::<serde_json::Value>(&data).is_ok() {
                                // Parsed fine but held nothing recoverable —
                                // an empty checkpoint is safe to clean up.
                                recovered = true;
                            } else {
                                warn!(
                                    "Checkpoint for session {} did not parse — keeping it for manual inspection",
                                    session_id
                                );
                            }
                        }
                        if recovered {
                            if let Err(e) = storage.delete_checkpoint(&session_id).await {
                                warn!(
                                    "Failed to delete checkpoint for session {}: {}",
                                    session_id, e
                                );
                            }
                        }
                    }
                }
                Err(e) => warn!("Failed to list checkpoints: {}", e),
            }

            // Also migrate any legacy checkpoint JSON files
            let checkpoint_dir = self.config.data_dir.join("checkpoints");
            if checkpoint_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(&checkpoint_dir) {
                    for entry in entries.flatten() {
                        let filename = entry.file_name();
                        let name = filename.to_string_lossy();
                        if name.starts_with("checkpoint-") && name.ends_with(".json") {
                            let session_id = name
                                .strip_prefix("checkpoint-")
                                .and_then(|s| s.strip_suffix(".json"))
                                .unwrap_or("");
                            if !session_id.is_empty() {
                                if let Some(partial) = agent.recover_checkpoint(session_id) {
                                    let reasoning = partial.reasoning.clone();
                                    self.sessions
                                        .add_full_message(
                                            session_id,
                                            crate::session::MessageRole::Assistant,
                                            &partial.content,
                                            partial.tool_calls,
                                            reasoning,
                                            partial.timeline,
                                            partial.usage,
                                        )
                                        .await;
                                    info!(
                                        "Recovered crashed run from legacy checkpoint for session {}",
                                        session_id
                                    );
                                }
                                // Remove the legacy file
                                let _ = std::fs::remove_file(entry.path());
                            }
                        }
                    }
                }
            }
        }

        // Shared activity clock: stamped by the control plane on every chat
        // request, read by the scheduled dream cycle to gate on idleness. Made
        // here so it is in scope for both the scheduler executor (below) and the
        // control plane (built later); cloning the Arc shares the same clock.
        let activity_clock = Arc::new(nanna_memory::ActivityClock::new());

        // The single dreaming orchestrator (P13 unification). Built once here and
        // shared by BOTH consolidation paths — the scheduled cycle below and the
        // IPC `MemoryAction::Consolidate` handler — so they run the same
        // multi-phase body over the same live store and accumulate pending
        // feedback in one place. It reads the very `activity_clock` the control
        // plane stamps, so its idle gate cannot drift from the daemon's own
        // notion of "in use".
        let dreaming: Option<Arc<nanna_memory::DreamingService>> = memory.as_ref().map(|memory| {
            let dreaming_config = nanna_memory::DreamingConfig {
                idle_threshold_secs: self.config.dream_idle_threshold_secs,
                memory_pressure_count: self.config.dream_memory_pressure_count,
                ..nanna_memory::DreamingConfig::default()
            };
            Arc::new(
                nanna_memory::DreamingService::with_shared_memory(
                    dreaming_config,
                    Arc::clone(memory),
                )
                .with_activity_clock(Arc::clone(&activity_clock)),
            )
        });

        // Dreaming must observe the very store the agent writes to, never a
        // private copy — that identity is the whole point of the shared seam.
        debug_assert_eq!(
            dreaming.is_some(),
            memory.is_some(),
            "dreaming service must exist exactly when the memory store does"
        );

        // Scheduler: with daemon-first startup the daemon owns nanna.db, so it
        // is the cron runner (the GUI scheduler only runs in embedded mode).
        // Loads persisted jobs and runs heartbeat + memory consolidation,
        // mirroring the GUI's embedded schedule.
        let scheduler = {
            let scheduler_config = nanna_core::SchedulerConfig {
                heartbeat_interval: std::time::Duration::from_secs(1800),
                heartbeat_enabled: true,
                heartbeat_prompt: DAEMON_HEARTBEAT_PROMPT.to_string(),
                max_concurrent: 4,
                check_interval: std::time::Duration::from_secs(30),
                default_timezone: "UTC".to_string(),
            };
            let mut scheduler = nanna_core::Scheduler::new(scheduler_config);
            if let Some(ref storage) = self.storage {
                scheduler = scheduler.with_storage(storage.clone());
                match scheduler.load_jobs().await {
                    Ok(count) => info!("Loaded {count} cron jobs from storage"),
                    Err(e) => warn!("Failed to load cron jobs: {e}"),
                }
            } else {
                info!("Scheduler running without persistence (no storage backend)");
            }

            let deduped = scheduler.deduplicate_by_name("memory_consolidation").await;
            if deduped > 0 {
                info!("Removed {deduped} duplicate consolidation tasks");
            }
            if !scheduler.has_task_named("memory_consolidation").await {
                scheduler
                    .add_task(nanna_core::consolidation_task(Some(
                        std::time::Duration::from_secs(3600),
                    )))
                    .await;
                info!("Scheduled memory consolidation task (every 1 hour)");
            }

            // Task recurrence sweep (P15): the scheduler is the one recurrence
            // engine — recurring todo items are reopened here, not by a second
            // clock inside the task store.
            if self.storage.is_some() {
                let deduped = scheduler.deduplicate_by_name("task_recurrence_sweep").await;
                if deduped > 0 {
                    info!("Removed {deduped} duplicate recurrence sweep tasks");
                }
                if !scheduler.has_task_named("task_recurrence_sweep").await {
                    scheduler
                        .add_task(nanna_core::recurring_task(
                            "task_recurrence_sweep",
                            std::time::Duration::from_secs(300),
                            "Reopen recurring tasks whose next occurrence has arrived.",
                        ))
                        .await;
                    info!("Scheduled task recurrence sweep (every 5 minutes)");
                }
            }

            let agent_for_tasks = agent.clone();
            let dreaming_for_tasks = dreaming.clone();
            let router_for_tasks = router.clone();
            let storage_for_tasks = self.storage.clone();
            // Capture the user's memory-compression settings for the scheduled
            // dream cycle (Copy scalars, moved into the executor closure).
            let consolidation_max_ratio = self.config.memory_max_compression_ratio;
            let consolidation_min_remaining = self.config.memory_min_remaining_memories;
            // Idle threshold is captured for the skip log only — the gate
            // *decision* now lives in the DreamingService (built above with
            // both thresholds), so there is no second copy of the policy here.
            let dream_idle_threshold_secs = self.config.dream_idle_threshold_secs;
            let activity_for_tasks = activity_clock.clone();
            // The user's full summarization priority list, not just its head:
            // the scheduled dream cycle now fails over exactly like the IPC one,
            // so a single unavailable model no longer kills the nightly cycle.
            let summarization_models = crate::dream_summarizer::summarization_models(
                &self.config.agent.summarization_priority,
                std::slice::from_ref(&self.config.agent.model),
            );
            let executor: nanna_core::TaskExecutor = Arc::new(move |task| {
                let agent = agent_for_tasks.clone();
                let dreaming = dreaming_for_tasks.clone();
                let router = router_for_tasks.clone();
                let storage = storage_for_tasks.clone();
                let activity = activity_for_tasks.clone();
                let summarization_models = summarization_models.clone();
                Box::pin(async move {
                    let start = std::time::Instant::now();
                    let started_at = chrono::Utc::now();
                    let (success, output, error) = match task.name.as_str() {
                        "memory_consolidation" => {
                            if let Some(ref dreaming) = dreaming {
                                // The idle gate AND the full dream cycle (feedback
                                // flush -> FSRS testing-effect flush -> consolidate)
                                // both live in the one `DreamingService`. The daemon
                                // supplies only what it owns: its lock-free activity
                                // clock and a consolidation budget sized to *its*
                                // summarizer model. Dreaming competes with the live
                                // agent for that model and rewrites the store, so it
                                // still only runs during a lull (or under pressure).
                                let idle = activity.idle();
                                // Size the budget to the SMALLEST window across
                                // the failover list: one prompt is built and
                                // then offered to each candidate in turn, so a
                                // budget fitted to the first model would
                                // overflow a smaller fallback.
                                let window_tokens =
                                    crate::dream_summarizer::summarizer_context_window_tokens(
                                        &router,
                                        &summarization_models,
                                    )
                                    .await;
                                let consolidation_config = scheduled_consolidation_config(
                                    consolidation_max_ratio,
                                    consolidation_min_remaining,
                                    window_tokens,
                                );
                                let summarize = crate::dream_summarizer::summarize_with_failover(
                                    router.clone(),
                                    summarization_models.clone(),
                                );
                                match dreaming
                                    .dream_if_triggered(idle, &consolidation_config, summarize)
                                    .await
                                {
                                    Ok(None) => {
                                        let memory_count = dreaming.memory().count().await;
                                        debug!(
                                            "Skipping scheduled consolidation: system active \
                                             (idle {idle:?} < {dream_idle_threshold_secs}s, \
                                             {memory_count} memories)"
                                        );
                                        (
                                            true,
                                            Some(format!(
                                                "Skipped (active; idle {}s, {memory_count} memories)",
                                                idle.as_secs()
                                            )),
                                            None,
                                        )
                                    }
                                    Ok(Some((trigger, stats))) => {
                                        info!(
                                            "Scheduled dream ({trigger:?}): {} processed, \
                                             {} merged, {} deduped, {} promoted, {} demoted",
                                            stats.consolidation.memories_processed,
                                            stats.consolidation.memories_merged,
                                            stats.consolidation.memories_deduped,
                                            stats.auto_promoted,
                                            stats.auto_demoted,
                                        );
                                        (
                                            true,
                                            Some(format!(
                                                "Processed {} memories",
                                                stats.consolidation.memories_processed
                                            )),
                                            None,
                                        )
                                    }
                                    Err(e) => {
                                        error!("Scheduled consolidation failed: {e}");
                                        (false, None, Some(e.to_string()))
                                    }
                                }
                            } else {
                                (
                                    true,
                                    Some("Skipped (memory service unavailable)".to_string()),
                                    None,
                                )
                            }
                        }
                        "task_recurrence_sweep" => {
                            if let Some(ref storage) = storage {
                                let reopened = crate::tasks::sweep_recurrences(storage).await;
                                if reopened > 0 {
                                    info!("Recurrence sweep reopened {reopened} tasks");
                                }
                                (
                                    true,
                                    Some(format!("Reopened {reopened} recurring tasks")),
                                    None,
                                )
                            } else {
                                (true, Some("Skipped (no storage)".to_string()), None)
                            }
                        }
                        _ if task.payload.is_empty() => {
                            debug!("Skipping task with empty payload: {}", task.name);
                            (true, Some("Skipped (empty payload)".to_string()), None)
                        }
                        _ => {
                            // Heartbeat and cron jobs run as full agent prompts
                            // (tools, memory, model fallback) in a task-scoped
                            // session that is not persisted to the session store.
                            let session_id = format!("scheduled-{}", task.id);
                            // Idle gate: never start an autonomous prompt on top
                            // of a live run. A local model server serves one
                            // generation at a time, so a heartbeat firing into a
                            // streaming chat gets the slot time-shared and the
                            // chat's generation CANCELLED — surfacing as a bogus
                            // "provider incident" the harness then heals against
                            // (observed live 2026-07-26). Skipping loses nothing:
                            // a heartbeat exists to work during a lull, and the
                            // next tick picks up whatever was due.
                            if agent.any_run_active().await {
                                debug!(
                                    "Skipping scheduled task '{}': a run is already in flight",
                                    task.name
                                );
                                (true, Some("Skipped (a run is in flight)".to_string()), None)
                            } else {
                            // An autonomous agent run (heartbeat / cron / task
                            // prompt) is the daemon actively using the model, so
                            // it counts as activity too — defer the dream cycle
                            // while it runs. Heartbeats are infrequent (30 min)
                            // vs the 5-min idle threshold, and memory pressure
                            // still overrides, so dreaming is not starved.
                            activity.record();
                            match agent.chat(&session_id, &task.payload, None, &[]).await {
                                Ok(result) => {
                                    let heartbeat_ok = task.name == "heartbeat"
                                        && result.content.trim().contains("HEARTBEAT_OK");
                                    if heartbeat_ok {
                                        debug!("Heartbeat: OK (nothing to do)");
                                    } else {
                                        info!(
                                            "Scheduled task '{}' completed: {}",
                                            task.name,
                                            result.content.chars().take(200).collect::<String>()
                                        );
                                    }
                                    if task.target_channel.is_some() {
                                        warn!(
                                            "Task '{}' targets a channel; channel routing from the \
                                             daemon scheduler is not implemented yet",
                                            task.name
                                        );
                                    }
                                    (true, Some(result.content), None)
                                }
                                Err(e) => {
                                    error!("Scheduled task '{}' failed: {}", task.name, e.message);
                                    (false, None, Some(e.message))
                                }
                            }
                            }
                        }
                    };
                    nanna_core::TaskResult {
                        task_id: task.id.clone(),
                        task_name: task.name.clone(),
                        success,
                        output,
                        error,
                        duration_ms: start.elapsed().as_millis() as u64,
                        started_at,
                        finished_at: chrono::Utc::now(),
                    }
                })
            });
            scheduler = scheduler.with_executor(executor);
            scheduler.start();
            info!("Daemon scheduler started (heartbeat + cron runner)");
            Arc::new(tokio::sync::RwLock::new(scheduler))
        };

        // Create control plane with all services (including router for consolidation)
        let mut control = ControlPlane::with_all_services(
            self.sessions.clone(),
            agent,
            memory.clone(),
            Some(tools),
            Some(router),
        )
        .with_tools_dir(tools_dir)
        .with_event_tx(self.ipc.event_sender())
        .with_workspace_id(workspace_id_for_services)
        .with_scheduler(scheduler)
        .with_task_runs(Arc::new(crate::tasks::TaskRunManager::new()))
        .with_memory_recovery(self.memory_recovery.clone())
        .with_shutdown(self.shutdown_tx.clone());
        if let Some(ref buf) = self.log_buffer {
            control = control.with_log_buffer(buf.clone());
        }
        // Make the tracker the agent + sub-agents record into (from
        // init_services) the canonical one the control plane owns. Must happen
        // BEFORE with_storage, which loads persisted stats via
        // import_from_storage — those now land in the shared tracker too.
        control.model_stats = model_stats;
        // Load persisted model stats from storage
        if let Some(ref storage) = self.storage {
            control = control.with_storage(storage.clone()).await;
        }

        // Load persisted workspaces from database
        if let Some(ref storage) = self.storage {
            match storage.workspaces().list().await {
                Ok(records) if !records.is_empty() => {
                    let mut registry = control.workspaces().write().await;
                    let mut active_id = None;
                    for record in &records {
                        let path = PathBuf::from(&record.path);
                        if path.exists() {
                            let mut ws = nanna_core::Workspace::new(&path);
                            ws.id = record.id.clone();
                            if let Err(e) = ws.load_context().await {
                                warn!(
                                    "Failed to load workspace context for {}: {}",
                                    record.path, e
                                );
                            }
                            registry.register(ws);
                            if record.active {
                                active_id = Some(record.id.clone());
                            }
                        } else {
                            warn!("Persisted workspace path no longer exists: {}", record.path);
                        }
                    }
                    if let Some(id) = active_id {
                        registry.set_active(&id);
                        // Seed the tool working directory from the persisted active
                        // workspace so tools resolve against it from boot — not just
                        // after an interactive SetActive or the first workspace-scoped
                        // chat. Without this, a fresh daemon with a persisted active
                        // workspace left `default_workdir` at None until the user
                        // re-selected it, so tools fell back to the home dir instead of
                        // running "in the workspace you're in".
                        let active_path = registry.get(&id).map(|ws| ws.path.clone());
                        drop(registry);
                        if let (Some(tools), Some(path)) = (control.tools(), active_path) {
                            tools.set_default_workdir(Some(path.clone())).await;
                            info!(
                                "Seeded tool working directory from active workspace: {:?}",
                                path
                            );
                        }
                    } else {
                        drop(registry);
                    }
                    info!("Restored {} workspaces from database", records.len());
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Failed to load workspaces from database: {}", e);
                }
            }
        }

        // Wire model stats tracker into the router for health-aware routing.
        // The control plane owns the canonical tracker; the router reads it.
        if let Some(ref router) = control.router() {
            router.set_stats(control.model_stats.clone()).await;
            info!("Stats-informed routing enabled on LLM router");
        }

        // Shared channel status manager — attached before the Arc wrap so
        // ChannelAction::Status and ChannelManager listeners see the same state.
        let channel_status_manager = Arc::new(nanna_channels::StatusManager::new());
        control.set_status_manager(Arc::clone(&channel_status_manager));

        // Share the activity clock so chat requests stamp the same clock the
        // scheduled dream cycle reads for its idle gate.
        control.set_activity_clock(Arc::clone(&activity_clock));

        // Share the ONE dreaming orchestrator, so an IPC-triggered consolidation
        // runs the same multi-phase cycle the scheduler does (P13 unification).
        if let Some(ref dreaming) = dreaming {
            control.set_dreaming(Arc::clone(dreaming));
        }

        let control = Arc::new(control);

        // Take the request receiver from IPC server
        let mut request_rx =
            self.ipc.take_request_receiver().await.ok_or_else(|| {
                crate::DaemonError::Ipc("Request receiver already taken".to_string())
            })?;

        let mut shutdown_rx = self.shutdown_tx.subscribe();

        // Spawn IPC server
        let ipc_server = self.ipc.clone();
        let ipc_shutdown = self.shutdown_tx.clone();
        let ipc_handle = tokio::spawn(async move {
            if let Err(e) = ipc_server.run().await {
                // An IPC-less daemon is unreachable (no control plane) but
                // would keep running heartbeats and burning the LLM budget —
                // observed live when a second instance lost the port race.
                // Take the daemon down instead of zombie-ing.
                error!(
                    "IPC server error: {} — shutting down (a daemon without IPC is unreachable)",
                    e
                );
                if ipc_shutdown.send(()).is_err() {
                    // No shutdown listener yet — exit hard rather than linger.
                    std::process::exit(1);
                }
            }
        });

        // Announce a startup quarantine + rebuild to subscribed clients. Boot
        // usually precedes any subscriber, so /status (health server + control
        // plane) carries the same facts for late-connecting clients.
        if let Some(ref report) = self.memory_recovery {
            self.ipc
                .broadcast_event(crate::protocol::Event::MemoryStoreRebuilt {
                    recovered: report.memories_recovered,
                    expected: report.memories_expected,
                    quarantine_path: report.quarantine_path.to_string_lossy().to_string(),
                })
                .await;
        }

        // Spawn health HTTP server if enabled
        let _health_state = if self.config.enable_health_server {
            // Seed durable-memory-store health (load already ran in init_services),
            // so a corrupt/degraded store shows on /status, not just a boot log.
            let (mem_degraded, mem_corrupt) = if let Some(ref m) = memory {
                let h = m.store_health().await;
                (h.degraded, h.corrupt_rows)
            } else {
                (false, 0)
            };
            let mut state = HealthState::new(
                memory.is_some(),
                true, // agent is available
            )
            .with_memory_health(mem_degraded, mem_corrupt);
            if let Some(ref report) = self.memory_recovery {
                state = state
                    .with_memory_rebuild(report.memories_recovered, report.memories_expected);
            }
            let health_state = Arc::new(state);

            // Update session count
            let sessions_for_health = self.sessions.clone();
            let health_state_clone = health_state.clone();
            tokio::spawn(async move {
                loop {
                    let count = sessions_for_health.count().await;
                    health_state_clone.set_session_count(count).await;
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            });

            // Serve the SAME state the session-count loop above updates
            // (via `from_shared`), so `/status` reflects live counts instead of
            // a throwaway copy stuck at zero. The server logs its own
            // "listening" line from `run()` once the bind succeeds, so we don't
            // pre-log here (that duplicate line falsely implied a bind before
            // one had happened).
            let health_server = HealthServer::from_shared(
                health_state.clone(),
                &self.config.ipc.host,
                self.config.health_port,
            );
            health_server.spawn();

            Some(health_state)
        } else {
            None
        };

        // Start ChannelManager if any channels are configured.
        // This handles listener-based inbound (polling) and routes responses back out.
        let channel_manager = if let Some(ref channels_config) = self.config.channels {
            // Build a daemon-local ChannelsConfig from the nanna_config::ChannelsConfig.
            // We re-map from nanna_config types to the daemon-local types.
            let daemon_channels = build_daemon_channels_config(channels_config);

            let mut manager = ChannelManager::with_status_manager(
                Arc::clone(&control),
                Arc::clone(&channel_status_manager),
            );
            manager.configure(&daemon_channels).await;

            // Also register outbound channels for webhook-sourced providers that have
            // bot tokens in the channel config (Telegram, Discord, Slack).
            // The listener-based configure() already does this; this is a no-op guard.

            match manager.start().await {
                Ok(()) => {
                    info!("Channel manager started");
                    Some(Arc::new(manager))
                }
                Err(e) => {
                    error!("Failed to start channel manager: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Spawn webhook HTTP server if enabled
        if self.config.enable_webhook_server {
            let mut webhook_config = self.config.webhook.clone();
            webhook_config.host = self.config.ipc.host.clone();
            webhook_config.port = self.config.webhook_port;

            // Keep a copy of the config for outbound channel registration below
            let webhook_config_copy = webhook_config.clone();

            let (webhook_server, mut webhook_rx) = WebhookServer::new(webhook_config);

            // Spawn the webhook server
            tokio::spawn(async move {
                if let Err(e) = webhook_server.run().await {
                    error!("Webhook server error: {}", e);
                }
            });

            // Build a shared router for the webhook event processor.
            // If a ChannelManager is running, share its router so outbound channels
            // (bot tokens) are already registered.  Otherwise create a standalone
            // router that may only cover providers registered via webhook config.
            let webhook_router = if let Some(ref mgr) = channel_manager {
                mgr.router()
            } else {
                // No channel manager — create a standalone router.
                // Outbound channels can be registered here from webhook config if
                // bot tokens are provided.
                let standalone_router =
                    Arc::new(tokio::sync::RwLock::new(ChannelMessageRouter::new()));

                // Register outbound channels from webhook config credentials
                {
                    let mut router = standalone_router.write().await;
                    if let Some(ref token) = webhook_config_copy.telegram_token {
                        router.register("telegram", Box::new(TelegramChannel::new(token)));
                        info!("Registered Telegram outbound channel from webhook config");
                    }
                    if webhook_config_copy.discord_public_key.is_some() {
                        // discord_public_key is for verification; bot token for sending
                        // is not separately stored in WebhookConfig currently.
                        // Log a warning — users should configure channels.discord instead.
                        debug!(
                            "Discord public key found in webhook config; for outbound replies configure channels.discord with a bot_token"
                        );
                    }
                }

                standalone_router
            };

            // Spawn webhook event processor — routes events through the same pipeline
            // as channel listener messages.
            let control_for_webhooks = Arc::clone(&control);
            tokio::spawn(async move {
                while let Some(event) = webhook_rx.recv().await {
                    debug!("Webhook event from {}: {:?}", event.source, event.message);

                    if let Some(ref msg) = event.message {
                        // Convert WebhookMessage → IncomingMessage
                        let incoming = IncomingMessage {
                            id: msg
                                .message_id
                                .clone()
                                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                            channel: ChannelId::new(&event.source, &msg.chat_id),
                            sender: ChannelSender {
                                id: msg.sender_id.clone(),
                                name: msg.sender_name.clone(),
                                username: None,
                            },
                            content: MessageContent::Text {
                                text: msg.content.clone(),
                            },
                            timestamp: event.timestamp,
                            reply_to: None,
                        };

                        // Process through the same pipeline as channel listeners
                        let router_guard = webhook_router.read().await;
                        ChannelManager::process_message(
                            incoming,
                            &control_for_webhooks,
                            &router_guard,
                        )
                        .await;
                    }
                }
            });

            info!(
                "Webhook server listening on http://{}:{}",
                self.config.ipc.host, self.config.webhook_port
            );
        }

        // Sessions are now persisted via Turso write-through on every mutation.
        // No more periodic JSON auto-save — each create/message/delete/rename writes to DB immediately.

        // Spawn model + tool stats auto-save task (every 5 minutes)
        let stats_control = control.clone();
        let mut stats_shutdown = self.shutdown_tx.subscribe();
        let stats_save_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        stats_control.save_model_stats().await;
                        stats_control.save_tool_stats().await;
                    }
                    _ = stats_shutdown.recv() => {
                        // Final save on shutdown
                        stats_control.save_model_stats().await;
                        stats_control.save_tool_stats().await;
                        info!("Model + tool stats final save completed");
                        break;
                    }
                }
            }
        });

        // Spawn sub-agent check-in task: periodically check running sub-agents
        // and drain parent mailboxes so questions don't go stale.
        // When a sub-agent uses ask_parent, the ParentChannelImpl handles it directly
        // via an LLM call. This task handles any orphaned mailbox messages and provides
        // visibility into long-running sub-agents.
        {
            let sessions = self.sessions.clone();
            let ipc_events = self.ipc.event_sender();
            let mut checkin_shutdown = self.shutdown_tx.subscribe();
            tokio::spawn(async move {
                // Check every 30 seconds
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                            let running = sessions.list_sub_sessions(None).await;
                            let running: Vec<_> = running.into_iter()
                                .filter(|s| matches!(s.state, crate::session::SubSessionState::Running | crate::session::SubSessionState::Spawning))
                                .collect();

                            if running.is_empty() {
                                continue;
                            }

                            // Check each parent's mailbox for pending questions
                            for sub in &running {
                                if let Some(ref parent_id) = sub.parent_id {
                                    let messages = sessions.drain_mailbox(parent_id).await;
                                    for msg in messages {
                                        // Re-emit as events for any listening clients (GUI)
                                        let _ = ipc_events.send(crate::protocol::Event::SubSessionQuestion {
                                            session_id: sub.session_id.clone(),
                                            parent_id: Some(parent_id.clone()),
                                            label: sub.label.clone(),
                                            question: msg.content,
                                        });
                                    }
                                }
                            }
                        }
                        _ = checkin_shutdown.recv() => break,
                    }
                }
            });
        }

        info!(
            "Daemon ready. IPC server listening on ws://{}",
            self.ipc.address()
        );

        // Main event loop
        //
        // Every request is dispatched as a tokio task so the loop is purely
        // a router — it never blocks.  The multi-threaded runtime (default
        // for `Runtime::new()`) schedules tasks across worker threads, so
        // concurrent requests (e.g. session creation while an agent is
        // running) execute in parallel.
        loop {
            tokio::select! {
                Some((client_id, request)) = request_rx.recv() => {
                    let control = control.clone();
                    let ipc = self.ipc.clone();

                    tokio::spawn(async move {
                        let request_id = request.id.clone();
                        let result = control.handle(&client_id, request.action).await;
                        let response = Response::success(request_id, result);
                        if let Err(e) = ipc.send_response(&client_id, response).await {
                            warn!("Failed to send response to client {}: {}", client_id, e);
                        }
                    });
                }

                _ = shutdown_rx.recv() => {
                    info!("Shutdown signal received");
                    break;
                }
            }
        }

        // Cleanup
        info!("Shutting down daemon...");
        self.ipc.shutdown();

        // Keep channel_manager alive until the end of run() so the spawned
        // listener task doesn't get prematurely shut down via shutdown_tx drop.
        // Explicit drop here makes the intent clear.
        drop(channel_manager);

        // Wait for stats auto-save task to complete final save
        let _ = tokio::time::timeout(Duration::from_secs(5), stats_save_handle).await;

        ipc_handle.abort();

        // Release PID file
        if let Some(ref pid_file) = self.pid_file {
            pid_file.release();
        }

        info!("Daemon stopped");
        Ok(())
    }

    /// Initialize all services
    async fn init_services(
        &self,
    ) -> Result<
        (
            Arc<ToolRegistry>,
            Option<Arc<MemoryService>>,
            Arc<AgentService>,
            Arc<LlmRouter>,
            Option<PathBuf>,                          // tools_dir
            Arc<tokio::sync::RwLock<Option<String>>>, // workspace_id for script services
            nanna_agent::ModelStatsTracker,           // shared model-stats tracker
        ),
        crate::DaemonError,
    > {
        // Create LLM router with all available providers. The same resolution
        // + construction runs again on every control-plane config mutation
        // (see `control::config`), so a provider the user authenticates after
        // boot registers without a daemon restart — registration used to be
        // boot-only, leaving the GUI and daemon split-brained about which
        // providers exist.
        let router = LlmRouter::new();
        let creds = crate::llm_router::ProviderCredentials::resolve(&self.config.llm).await;
        router.rebuild(&creds);

        let available = router.available_providers();
        if available.is_empty() {
            return Err(crate::DaemonError::Config(
                "No LLM providers configured. Please set up at least one provider (Anthropic, OpenAI, OpenRouter, or Ollama).".to_string()
            ));
        }
        info!(
            "LLM router initialized with {} providers: {:?}",
            available.len(),
            available
        );
        let router = Arc::new(router);

        // Create empty tool registry — all tools loaded from disk
        let tools = Arc::new(ToolRegistry::new());

        // Resolve tools directory (env var > config > dev fallback > {data_dir}/tools/)
        let tools_dir = if self.config.use_script_tools {
            let config_dir = self.config.tools_dir.as_deref();
            let resolved = nanna_tools::skills::defaults::resolve_tools_dir(config_dir)
                .unwrap_or_else(|| self.config.data_dir.join("tools"));

            // Bootstrap default skills into the tools directory if needed.
            // In debug builds this is a no-op (tools load from source tree).
            // In release builds, embedded skills are extracted on first run.
            let bootstrapped = nanna_tools::skills::defaults::bootstrap_default_skills(&resolved);
            if bootstrapped > 0 {
                info!(
                    "Bootstrapped {} default skills into {:?}",
                    bootstrapped, resolved
                );
            }

            if resolved.is_dir() {
                nanna_tools::skills::defaults::ensure_permissions(&resolved);
                info!("Tools directory: {:?}", resolved);
            } else {
                warn!("Tools directory does not exist: {:?}", resolved);
            }
            Some(resolved)
        } else {
            None
        };

        // Initialize memory service with embeddings if enabled
        let memory: Option<Arc<MemoryService>> = if self.config.enable_memory {
            // Build primary embedding client
            let primary_client = match self.embedding.provider.as_str() {
                "openai" => {
                    let api_key = std::env::var("OPENAI_API_KEY").ok();
                    api_key.map(|key| {
                        info!("Primary embeddings: OpenAI {}", self.embedding.model);
                        (
                            EmbeddingProviderInfo {
                                name: "openai".into(),
                                model: self.embedding.model.clone(),
                            },
                            Arc::new(
                                nanna_llm::EmbeddingClient::openai(&key)
                                    .with_model(&self.embedding.model),
                            ),
                        )
                    })
                }
                "openrouter" => {
                    let api_key = self
                        .config
                        .llm
                        .openrouter_api_key
                        .clone()
                        .or_else(|| std::env::var("OPENROUTER_API_KEY").ok());
                    api_key.map(|key| {
                        info!("Primary embeddings: OpenRouter {}", self.embedding.model);
                        (
                            EmbeddingProviderInfo {
                                name: "openrouter".into(),
                                model: self.embedding.model.clone(),
                            },
                            Arc::new(
                                nanna_llm::EmbeddingClient::openai(&key)
                                    .with_model(&self.embedding.model)
                                    .with_base_url("https://openrouter.ai/api"),
                            ),
                        )
                    })
                }
                "ollama" | _ => {
                    info!(
                        "Primary embeddings: Ollama {} at {}",
                        self.embedding.model, self.embedding.ollama_host
                    );
                    Some((
                        EmbeddingProviderInfo {
                            name: "ollama".into(),
                            model: self.embedding.model.clone(),
                        },
                        Arc::new(
                            nanna_llm::EmbeddingClient::ollama(&self.embedding.ollama_host)
                                .with_model(&self.embedding.model),
                        ),
                    ))
                }
            };

            match primary_client {
                Some((primary_info, primary)) => {
                    // Build the embedding router with fallback providers
                    let mut embed_router = EmbeddingRouter::new(primary_info.clone(), primary);

                    // Add fallback providers from available credentials
                    if primary_info.name != "openai" {
                        if let Some(api_key) = std::env::var("OPENAI_API_KEY").ok() {
                            let fallback_model = "text-embedding-3-small".to_string();
                            info!("Adding OpenAI embedding fallback: {}", fallback_model);
                            embed_router = embed_router.with_fallback(
                                EmbeddingProviderInfo {
                                    name: "openai".into(),
                                    model: fallback_model.clone(),
                                },
                                Arc::new(
                                    nanna_llm::EmbeddingClient::openai(&api_key)
                                        .with_model(&fallback_model),
                                ),
                            );
                        }
                    }
                    if primary_info.name != "openrouter" {
                        if let Some(api_key) = self
                            .config
                            .llm
                            .openrouter_api_key
                            .clone()
                            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
                        {
                            let fallback_model = "openai/text-embedding-3-small".to_string();
                            info!("Adding OpenRouter embedding fallback: {}", fallback_model);
                            embed_router = embed_router.with_fallback(
                                EmbeddingProviderInfo {
                                    name: "openrouter".into(),
                                    model: fallback_model.clone(),
                                },
                                Arc::new(
                                    nanna_llm::EmbeddingClient::openai(&api_key)
                                        .with_model(&fallback_model)
                                        .with_base_url("https://openrouter.ai/api"),
                                ),
                            );
                        }
                    }
                    if primary_info.name != "ollama" {
                        let fallback_model = "nomic-embed-text".to_string();
                        info!(
                            "Adding Ollama embedding fallback: {} at {}",
                            fallback_model, self.embedding.ollama_host
                        );
                        embed_router = embed_router.with_fallback(
                            EmbeddingProviderInfo {
                                name: "ollama".into(),
                                model: fallback_model.clone(),
                            },
                            Arc::new(
                                nanna_llm::EmbeddingClient::ollama(&self.embedding.ollama_host)
                                    .with_model(&fallback_model),
                            ),
                        );
                    }

                    info!(
                        "Embedding router: {} providers configured",
                        embed_router.provider_count()
                    );
                    let embed_router = Arc::new(embed_router);

                    // Create embedding function that routes through the EmbeddingRouter.
                    // Tracks the router generation to detect provider switches.
                    let router_for_fn = embed_router.clone();
                    let last_generation =
                        Arc::new(std::sync::atomic::AtomicU64::new(embed_router.generation()));
                    // Placeholder for memory service — set after construction via lazy init
                    let memory_for_reembed: Arc<tokio::sync::OnceCell<Arc<MemoryService>>> =
                        Arc::new(tokio::sync::OnceCell::new());
                    let mem_cell_for_fn = memory_for_reembed.clone();

                    let embed_fn: nanna_memory::EmbedFn = Arc::new(move |text: &str| {
                        let router = router_for_fn.clone();
                        let text = text.to_string();
                        let gen_tracker = last_generation.clone();
                        let mem_cell = mem_cell_for_fn.clone();
                        Box::pin(async move {
                            let (embedding, provider_changed) = router.embed_one(&text).await?;

                            // If the provider changed, check if we need to re-embed
                            if provider_changed {
                                let new_gen = router.generation();
                                let old_gen =
                                    gen_tracker.swap(new_gen, std::sync::atomic::Ordering::Relaxed);
                                if new_gen != old_gen {
                                    // Provider switched — realign the store BEFORE
                                    // this write is allowed to land.
                                    //
                                    // This used to `tokio::spawn` the probe and
                                    // return immediately, so the write that
                                    // detected the switch — and every write
                                    // racing it — went into a store still full
                                    // of the previous provider's vectors. Two
                                    // widths then coexisted, which is not a
                                    // degraded search but a broken one: the
                                    // comparison either asserts or, at equal
                                    // widths from different models, silently
                                    // returns nonsense.
                                    //
                                    // A width change means everything must be
                                    // re-embedded, and the only safe moment is
                                    // before the next write. It costs a stall.
                                    // That is what it costs.
                                    //
                                    // The `swap` above is the stampede guard:
                                    // it is atomic, so exactly one caller
                                    // observes the change and does the work.
                                    if let Some(mem) = mem_cell.get() {
                                        let model = router.active_provider().await.to_string();
                                        tracing::info!(
                                            "Embedding provider changed (gen {} → {}) — rebinding \
                                             the store to '{}'",
                                            old_gen,
                                            new_gen,
                                            model
                                        );

                                        // Rebinding is a hash lookup per entry:
                                        // no network, no re-embed, and a switch
                                        // BACK to a model used earlier is free
                                        // because its bucket was retained.
                                        let (_, missing) = mem.rebind_embeddings(&model).await;

                                        // Whatever this model has never embedded
                                        // gets filled in lazily, in bounded
                                        // passes, while the run continues. It
                                        // must not be done inline: the store can
                                        // hold thousands of entries and the
                                        // provider we just failed over to may be
                                        // the rate-limited one.
                                        if missing > 0 {
                                            let mem = mem.clone();
                                            tokio::spawn(async move {
                                                const BATCH: usize = 64;
                                                loop {
                                                    match mem
                                                        .backfill_embeddings(&model, BATCH)
                                                        .await
                                                    {
                                                        Ok(0) => break,
                                                        Ok(_) => {}
                                                        Err(e) => {
                                                            tracing::warn!(
                                                                "Backfill for '{model}' halted: {e}"
                                                            );
                                                            break;
                                                        }
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
                            }

                            Ok(embedding)
                        })
                    });

                    // Seed the embedding dimension by probing the router.
                    //
                    // A probe failure must NOT stop the daemon: Nanna is
                    // offline-capable by default, so an unreachable or unkeyed
                    // embedding provider degrades memory — it does not refuse
                    // to boot. The seed only has to be a valid positive
                    // dimension: real vectors always come from the provider,
                    // and the background `probe_and_align_dimension` below
                    // corrects the store (re-embedding any mismatched entries)
                    // as soon as a provider answers. Probing here is purely an
                    // optimization — when it succeeds the store is right
                    // immediately and nothing is ever re-embedded.
                    let seed_dimension = nanna_memory::MemoryServiceConfig::default().dimension;
                    let dimension = match Self::probe_embedding_dimension(&embed_router).await {
                        Ok(dim) => {
                            info!(
                                "Memory service using probed dimension {} for model {}",
                                dim, self.embedding.model
                            );
                            dim
                        }
                        Err(e) => {
                            warn!(
                                "Could not probe the embedding dimension ({e}). Starting anyway with a \
                                 provisional dimension of {seed_dimension}; memory will re-align \
                                 automatically once an embedding provider is reachable. To enable \
                                 embeddings, run a local Ollama with `ollama pull {}` or set an \
                                 OpenAI/OpenRouter key.",
                                self.embedding.model
                            );
                            seed_dimension
                        }
                    };
                    assert!(dimension > 0, "embedding dimension must be positive");
                    let config = nanna_memory::MemoryServiceConfig {
                        dimension,
                        ..Default::default()
                    };

                    // Wire up Turso persistence if storage is available.
                    // The persistence adapter is constructed here and attached to the
                    // MemoryService so all writes are automatically mirrored to Turso.
                    let memory_service = if let Some(ref storage) = self.storage {
                        let repo = storage.memories();
                        let db = Arc::new(TursoMemoryPersistence::new(repo));
                        nanna_memory::MemoryService::new(config)
                            .with_embed_fn(embed_fn)
                            .with_persistence(db)
                    } else {
                        warn!(
                            "No storage backend available — memory will NOT be persisted to Turso"
                        );
                        nanna_memory::MemoryService::new(config).with_embed_fn(embed_fn)
                    };

                    // One-time migration: if memories.json exists and Turso is empty,
                    // load from JSON into in-memory cache then save each entry to Turso.
                    let json_path = self.memory_path.as_ref();
                    let should_migrate = if let (Some(path), Some(storage)) =
                        (json_path, &self.storage)
                    {
                        if path.exists() {
                            match storage.memories().count().await {
                                Ok(0) => true,
                                Ok(n) => {
                                    info!(
                                        "Turso already has {} memories — skipping JSON migration",
                                        n
                                    );
                                    false
                                }
                                Err(e) => {
                                    warn!("Could not check Turso memory count: {}", e);
                                    false
                                }
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if should_migrate {
                        let path = json_path.unwrap();
                        info!(
                            "Migrating memories from {:?} to Turso (one-time migration)",
                            path
                        );
                        match memory_service.load(path).await {
                            Ok(()) => {
                                let count = memory_service.count().await;
                                info!("Loaded {} memories from JSON, flushing to Turso...", count);
                                // Flush all entries to Turso
                                match memory_service.flush_to_db().await {
                                    Ok(n) => info!("Flushed {} memories to Turso", n),
                                    Err(e) => warn!("Failed to flush memories to Turso: {}", e),
                                }
                                // Rename the JSON file so we don't re-migrate next time
                                let migrated_path = path.with_extension("json.migrated");
                                if let Err(e) = tokio::fs::rename(path, &migrated_path).await {
                                    warn!("Could not rename migrated JSON file: {}", e);
                                } else {
                                    info!(
                                        "Renamed {:?} → {:?} (migration complete)",
                                        path, migrated_path
                                    );
                                }
                            }
                            Err(e) => {
                                warn!(
                                    "JSON migration failed: {}. Will attempt to load from Turso.",
                                    e
                                );
                            }
                        }
                    }

                    // Load from Turso into the in-memory cache (normal startup path).
                    // Skipped if we just migrated (the entries are already in-memory from the JSON load above).
                    if !should_migrate {
                        match memory_service.load_from_db().await {
                            Ok(count) => {
                                info!("Loaded {} memories from Turso", count);
                            }
                            Err(nanna_memory::MemoryError::Persistence(ref e))
                                if e.contains("No persistence backend") =>
                            {
                                // No storage configured — silently skip
                            }
                            Err(e) => {
                                warn!("Failed to load memories from Turso: {}", e);
                            }
                        }
                    }

                    info!("Memory service initialized with Turso persistence and embedding router");
                    let memory_arc = Arc::new(memory_service);

                    // Probe the actual embedding dimension from the model IN THE
                    // BACKGROUND. The probe's first embed call can take ~a minute
                    // when the local embedding model is cold (Ollama loads it on
                    // demand), and it used to block startup past the GUI's
                    // daemon-ready timeout — forcing an embedded fallback while an
                    // orphaned daemon kept running. `probe_and_align_dimension`
                    // takes `&self` on the Arc'd service specifically so it can run
                    // at runtime; a mismatched dimension is corrected (and entries
                    // re-embedded) as soon as the probe completes.
                    {
                        let memory_for_probe = memory_arc.clone();
                        let model_name = self.embedding.model.clone();
                        // Bind the store to the model that is about to write to
                        // it, BEFORE any probe or write. Without this the first
                        // entries of a session get bucketed under `None` — they
                        // would be re-embedded on the next switch instead of
                        // being reusable, which is the whole point of buckets.
                        let bind_model = embed_router.active_provider().await.to_string();
                        let memory_for_bind = memory_arc.clone();
                        tokio::spawn(async move {
                            let (_, missing) = memory_for_bind.rebind_embeddings(&bind_model).await;
                            if missing > 0 {
                                const BATCH: usize = 64;
                                loop {
                                    match memory_for_bind.backfill_embeddings(&bind_model, BATCH).await {
                                        Ok(0) => break,
                                        Ok(_) => {}
                                        Err(e) => {
                                            warn!("Startup backfill for '{bind_model}' halted: {e}");
                                            break;
                                        }
                                    }
                                }
                            }
                        });
                        tokio::spawn(async move {
                            match memory_for_probe.probe_and_align_dimension().await {
                                Ok(actual_dim) => {
                                    if actual_dim == dimension {
                                        debug!(
                                            "Embedding dimension confirmed: {actual_dim} for model {model_name}"
                                        );
                                    } else {
                                        info!(
                                            "Embedding dimension corrected: {dimension} → {actual_dim} for model {model_name}"
                                        );
                                    }
                                }
                                Err(e) => {
                                    warn!(
                                        "Could not probe embedding dimension (model may be loading): {e}. \
                                         Using static dimension {dimension}."
                                    );
                                }
                            }
                        });
                    }

                    // Wire the memory service into the embed_fn's OnceCell
                    // so provider-switch re-embedding can find it
                    let _ = memory_for_reembed.set(memory_arc.clone());

                    Some(memory_arc)
                }
                None => {
                    warn!("Memory service disabled: no embedding provider available");
                    warn!(
                        "To enable memory: set OPENAI_API_KEY or use Ollama with embeddings model"
                    );
                    None
                }
            }
        } else {
            info!("Memory service disabled in config");
            None
        };

        // Shared session history for the recall_messages tool service
        let session_history: SharedSessionHistory = Arc::new(tokio::sync::RwLock::new(Vec::new()));

        // One shared model-stats tracker for the whole daemon: the main agent
        // (AgentService) and every sub-agent (AgentSpawnerImpl) record into it,
        // and the control plane makes it canonical (persists it + feeds the
        // router). Cloning shares state (Arc<RwLock<_>> inside).
        let model_stats = nanna_agent::ModelStatsTracker::new();

        // Build script services and load all tools from disk
        let workspace_id_for_services: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(None));
        {
            let spawner_arc: Option<Arc<dyn AgentSpawner + Send + Sync>> = if !router
                .available_providers()
                .is_empty()
            {
                // Build model routing for sub-agents from the priority list.
                // Cheapest model (last in priority) handles Simple tasks,
                // most capable (first) handles Complex. This gives sub-agents
                // intelligent per-iteration routing while the main agent always
                // uses the primary model.
                let sub_agent_routing = build_sub_agent_routing(&self.config.agent.model_priority);
                if !sub_agent_routing.is_empty() {
                    info!(
                        "Sub-agent routing: {:?}",
                        sub_agent_routing
                            .iter()
                            .map(|t| format!("{}:{:?}", t.model, t.tier))
                            .collect::<Vec<_>>()
                    );
                }

                Some(Arc::new(AgentSpawnerImpl {
                    router: router.clone(),
                    tools: tools.clone(),
                    agent_config: nanna_agent::AgentConfig {
                        model: self.config.agent.model.clone(),
                        max_tokens: self.config.agent.max_tokens,
                        temperature: self.config.agent.temperature,
                        max_iterations: None, // Unlimited — model stops when done
                        thinking_mode: self.config.agent.thinking_mode,
                        summarization_priority: self.config.agent.summarization_priority.clone(),
                        summarization_ollama_url: self
                            .config
                            .agent
                            .summarization_ollama_url
                            .clone(),
                        model_routing: sub_agent_routing,
                        routing_first_turn_primary: true,
                        openrouter_api_key: self.config.agent.openrouter_api_key.clone(),
                        openai_api_key: self.config.agent.openai_api_key.clone(),
                        ..Default::default()
                    },
                    system_prompt: nanna_agent::prompts::DEFAULT_SYSTEM_PROMPT.to_string(),
                    workspace_root: None,
                    workspace_context: None,
                    stats: Some(model_stats.clone()),
                    sub_agent_models: self.config.agent.sub_agent_models.clone(),
                }))
            } else {
                None
            };

            let summarizer_models = crate::dream_summarizer::summarization_models(
                &self.config.agent.summarization_priority,
                std::slice::from_ref(&self.config.agent.model),
            );
            let services = build_script_services(
                &memory,
                spawner_arc,
                session_history.clone(),
                workspace_id_for_services.clone(),
                self.storage.clone(),
                Some((router.clone(), summarizer_models)),
            );

            if let Some(ref dir) = tools_dir {
                if dir.is_dir() {
                    let loaded = tools.load_skills_with_services(dir, &services).await;
                    info!("Loaded {} tools from {:?}", loaded, dir);
                }
            }
        }

        // Register common aliases for Claude Code compatibility (after tools are loaded)
        tools.register_alias("read", "read_file").await;
        tools.register_alias("Read", "read_file").await;
        tools.register_alias("write", "write_file").await;
        tools.register_alias("Write", "write_file").await;
        tools.register_alias("edit", "edit_file").await;
        tools.register_alias("Edit", "edit_file").await;
        tools.register_alias("bash", "exec").await;
        tools.register_alias("Bash", "exec").await;
        tools.register_alias("glob", "list_dir").await;
        tools.register_alias("Glob", "list_dir").await;
        tools.register_alias("ls", "list_dir").await;

        {
            let tool_count = tools.definitions().await.len();
            info!("Tool registry: {} tools (including aliases)", tool_count);
        }

        // Register discover_tools (JS/TS skill with registry access)
        if let Some(ref dir) = tools_dir {
            if let Some(source) = nanna_tools::skills::defaults::load_discover_tools_source(dir) {
                let wrapper = nanna_tools::skills::ScriptedToolWrapper::from_source(
                    "discover_tools",
                    &source,
                )
                .expect("discover_tools skill must parse")
                .with_registry(Arc::downgrade(&tools));
                tools.register(wrapper).await;
                info!("Registered discover_tools skill from {:?}", dir);
            } else {
                warn!("discover_tools not found in tools directory");
            }
        }

        // Register ask_parent tool for sub-agent ↔ parent communication
        {
            let parent_channel: Arc<dyn ParentChannel + Send + Sync> =
                Arc::new(ParentChannelImpl {
                    sessions: self.sessions.clone(),
                    event_tx: Some(self.ipc.event_sender()),
                    router: router.clone(),
                    model: self.config.agent.model.clone(),
                });
            tools
                .register(nanna_tools::AskParentTool::new(
                    parent_channel,
                    tools.clone(),
                ))
                .await;
            info!("Registered ask_parent tool for sub-agent communication");
        }

        // Apply the tool allow/deny policy. Built from `[tools] enabled/disabled`
        // and enforced by the registry AFTER alias/fuzzy resolution, so a denied
        // tool cannot be reached via an alias (`Bash` → `exec`), a case variant,
        // or a fuzzy near-miss. Skipped entirely when unrestricted.
        let policy = build_tool_policy(
            self.config.tool_allowlist.as_deref(),
            &self.config.tool_denylist,
        );
        if !policy.is_unrestricted() {
            tools.set_policy(policy).await;
        }

        // Create agent service with multi-provider router
        let event_tx = self.ipc.event_sender();
        let mut agent_service = AgentService::with_data_dir(
            self.config.agent.clone(),
            router.clone(),
            tools.clone(),
            memory.clone(),
            event_tx,
            Some(self.config.data_dir.clone()),
        )
        .with_session_history(session_history)
        .with_stats(model_stats.clone());
        if let Some(ref storage) = self.storage {
            agent_service = agent_service.with_storage(storage.clone());
        }
        let agent = Arc::new(agent_service);

        info!("Agent service initialized");

        Ok((
            tools,
            memory,
            agent,
            router,
            tools_dir,
            workspace_id_for_services,
            model_stats,
        ))
    }
}

/// Build a [`ToolPolicy`] from the `[tools] enabled`/`disabled` config lists.
///
/// Semantics chosen to match the existing config surface:
/// - `enabled` acts as an allowlist. The conventional wildcard `"*"` (the
///   default) means "no allowlist in force" — every tool is permitted. An empty
///   list would deny everything, which nobody writes by accident because the
///   default is `["*"]`; we treat an empty allowlist as unrestricted too, so a
///   caller that forgets to populate it fails *open* on the allow side (the
///   denylist still applies) rather than silently muting every tool.
/// - `disabled` is the denylist and always wins (fail closed on conflict).
fn build_tool_policy(enabled: Option<&[String]>, disabled: &[String]) -> ToolPolicy {
    // Thin wrapper over the shared interpretation in `nanna-tools`, so the
    // daemon and `nanna mcp serve` cannot drift on what `[tools] enabled` /
    // `disabled` mean. The daemon's tests below pin the behaviour from this side.
    ToolPolicy::from_config_lists(enabled, disabled)
}

/// Embedding configuration for the daemon
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// Provider (ollama, openai, openrouter)
    pub provider: String,
    /// Model name
    pub model: String,
    /// Ollama host (if using Ollama)
    pub ollama_host: String,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: "ollama".to_string(),
            model: "nomic-embed-text".to_string(),
            ollama_host: "http://localhost:11434".to_string(),
        }
    }
}

/// Builder for DaemonServer
pub struct DaemonBuilder {
    config: DaemonConfig,
    embedding: EmbeddingConfig,
    memory_path: Option<PathBuf>,
    brave_api_key: Option<String>,
    log_buffer: Option<crate::log_buffer::LogBuffer>,
}

/// Copy the signature-verification secrets from the user's channel config into a
/// [`WebhookConfig`]. Only providers the user configured are set; each verifier
/// skips when its secret is `None`, so unset providers keep the previous value.
///
/// Telegram is intentionally absent: its `TelegramConfig` carries no webhook
/// secret (Telegram authenticates via the bot token in the URL), only the bot
/// token (registered for outbound sends).
fn apply_channel_webhook_secrets(
    webhook: &mut WebhookConfig,
    channels: &nanna_config::ChannelsConfig,
) {
    if let Some(ref discord) = channels.discord {
        webhook.discord_public_key = Some(discord.public_key.clone());
    }
    if let Some(ref slack) = channels.slack {
        webhook.slack_signing_secret = Some(slack.signing_secret.clone());
    }
    if let Some(ref whatsapp) = channels.whatsapp {
        webhook.whatsapp_verify_token = whatsapp.verify_token.clone();
        webhook.whatsapp_app_secret = whatsapp.app_secret.clone();
    }
    if let Some(ref telegram) = channels.telegram {
        webhook.telegram_token = Some(telegram.bot_token.clone());
    }
}

impl DaemonBuilder {
    pub fn new() -> Self {
        Self {
            config: DaemonConfig::default(),
            embedding: EmbeddingConfig::default(),
            memory_path: None,
            brave_api_key: None,
            log_buffer: None,
        }
    }

    /// Create builder from Nanna config file
    pub fn from_nanna_config() -> Result<Self, crate::DaemonError> {
        use nanna_config::Config;

        let config = match Config::load() {
            Ok(cfg) => {
                info!("Loaded Nanna config successfully");
                cfg.with_env_overrides()
            }
            Err(e) => {
                warn!(
                    "Failed to load Nanna config: {}, using defaults with env overrides",
                    e
                );
                Config::default().with_env_overrides()
            }
        };

        let mut builder = Self::new();

        // Set LLM configuration - copy all provider credentials (shared with
        // the control-plane reload path so both derive providers identically)
        builder.config.llm = LlmConfig::from_nanna(&config);

        // Set embedding configuration from Nanna memory config
        builder.embedding.provider = config.memory.embedding_provider.clone();
        builder.embedding.model = config.memory.embedding_model.clone();
        builder.embedding.ollama_host = config.memory.ollama_host.clone();

        // Thread the memory-compression settings so the scheduled dream cycle
        // honors them (previously only the IPC-triggered path did).
        builder.config.memory_max_compression_ratio = config.memory.max_compression_ratio;
        builder.config.memory_min_remaining_memories = config.memory.min_remaining_memories;
        // Idle gate for the scheduled dream cycle (defers dreaming to a lull).
        builder.config.dream_idle_threshold_secs = config.memory.dream_idle_threshold_secs;
        builder.config.dream_memory_pressure_count = config.memory.dream_memory_pressure_count;

        // Wire webhook signature-verification secrets from the user's channel
        // config. Without this the inbound webhook server always ran with the
        // all-None default, so every provider's verification silently no-op'd —
        // the daemon never received the secrets it checks against.
        apply_channel_webhook_secrets(&mut builder.config.webhook, &config.channels);

        // Set data directory from Nanna config (same location as GUI)
        match nanna_config::Config::default_data_dir() {
            Ok(data_dir) => {
                info!("Using Nanna data directory: {:?}", data_dir);
                builder.config.data_dir = data_dir.clone();
                builder.memory_path = Some(data_dir.join("memories.json"));
            }
            Err(e) => {
                warn!("Could not determine Nanna data dir: {}, using default", e);
            }
        }

        // Set agent configuration from loaded config
        // Use user-configured model priority list for fallback
        builder.config.agent.model_priority = config.llm.model_priority.clone();
        info!("Model priority list: {:?}", config.llm.model_priority);

        if let Some(model) = config.llm.model_priority.first() {
            builder.config.agent.model = model.to_string();
        } else {
            builder.config.agent.model = config.llm.model.clone();
        }

        // Set summarization configuration
        builder.config.agent.summarization_priority = config.llm.summarization_priority.clone();
        builder.config.agent.summarization_ollama_url = config.llm.ollama_url.clone();

        // Pass API keys to agent config so summarization can use OpenRouter/OpenAI
        builder.config.agent.openrouter_api_key = config.llm.openrouter_api_key.clone();
        builder.config.agent.openai_api_key = config.llm.openai_api_key.clone();

        // Set thinking mode from config
        if config.agent.thinking_enabled {
            builder.config.agent.thinking_mode = ThinkingMode::Medium;
        }

        // Agent-loop iteration policy: unbounded by default (long-horizon worker),
        // with late escalating soft nudges. All three are user-configurable.
        builder.config.agent.max_iterations = config.agent.max_iterations;
        builder.config.agent.nudge_after_iterations = config.agent.nudge_after_iterations;
        builder.config.agent.nudge_interval_iterations = config.agent.nudge_interval_iterations;

        // Set model routing configuration
        builder.config.agent.model_routing = config.llm.model_routing.clone();
        builder.config.agent.routing_first_turn_primary = config.llm.routing_first_turn_primary;
        builder.config.agent.sub_agent_model = config.llm.sub_agent_model.clone();
        // Resolved here (list > legacy single > main chat list) so every
        // consumer sees one authoritative, never-empty chain.
        builder.config.agent.sub_agent_models = config.llm.effective_sub_agent_models();
        if !config.llm.model_routing.is_empty() {
            info!("Model routing enabled: {:?}", config.llm.model_routing);
        }
        if !config.llm.sub_agent_models.is_empty() || config.llm.sub_agent_model.is_some() {
            info!(
                "Sub-agent models: {:?}",
                builder.config.agent.sub_agent_models
            );
        }

        // Set Brave API key for web search
        builder.brave_api_key = config.tools.brave_api_key.clone();

        // Set script tools flag and tools directory
        builder.config.use_script_tools = config.tools.use_script_tools;
        builder.config.tools_dir = config.tools.tools_dir.clone();

        // Tool allow/deny policy — `[tools] enabled` is the allowlist ("*" = all),
        // `[tools] disabled` is the denylist. This is the wiring that makes a
        // disabled tool actually stop executing (the lists were previously
        // parsed into config but never enforced).
        builder.config.tool_allowlist = Some(config.tools.enabled.clone());
        builder.config.tool_denylist = config.tools.disabled.clone();

        // Load channel configuration (Telegram, Discord, Slack, etc.)
        let has_channels = config.channels.telegram.is_some()
            || config.channels.discord.is_some()
            || config.channels.slack.is_some()
            || config.channels.signal.is_some()
            || config.channels.whatsapp.is_some();
        if has_channels {
            builder.config.channels = Some(config.channels.clone());
            info!("Channel configuration loaded");
        }

        // Log configured providers
        let mut providers = Vec::new();
        if builder.config.llm.anthropic_api_key.is_some()
            || builder.config.llm.anthropic_oauth_token.is_some()
        {
            providers.push("anthropic");
        }
        if builder.config.llm.openai_api_key.is_some() {
            providers.push("openai");
        }
        if builder.config.llm.openrouter_api_key.is_some() {
            providers.push("openrouter");
        }
        if builder.config.llm.github_token.is_some() {
            providers.push("github");
        }
        providers.push("ollama"); // Always available

        info!(
            "Daemon config loaded: model={}, embedding={}:{}, providers=[{}], brave_key={}",
            builder.config.agent.model,
            builder.embedding.provider,
            builder.embedding.model,
            providers.join(", "),
            if builder.brave_api_key.is_some() {
                "set"
            } else {
                "none"
            }
        );

        Ok(builder)
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.config.ipc.port = port;
        self
    }

    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.config.ipc.host = host.into();
        self
    }

    pub fn with_data_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.data_dir = path.into();
        self
    }

    pub fn with_log_level(mut self, level: impl Into<String>) -> Self {
        self.config.log_level = level.into();
        self
    }

    pub fn with_auto_save_interval(mut self, secs: u64) -> Self {
        self.config.auto_save_interval_secs = secs;
        self
    }

    pub fn with_llm_provider(mut self, provider: impl Into<String>) -> Self {
        self.config.llm.provider = provider.into();
        self
    }

    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.config.llm.api_key = Some(key.into());
        self
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.config.agent.model = model.into();
        self
    }

    pub fn with_memory(mut self, enable: bool) -> Self {
        self.config.enable_memory = enable;
        self
    }

    pub fn with_health_server(mut self, enable: bool) -> Self {
        self.config.enable_health_server = enable;
        self
    }

    pub fn with_health_port(mut self, port: u16) -> Self {
        self.config.health_port = port;
        self
    }

    pub fn with_pid_file(mut self, enable: bool) -> Self {
        self.config.enable_pid_file = enable;
        self
    }

    pub fn with_webhook_server(mut self, enable: bool) -> Self {
        self.config.enable_webhook_server = enable;
        self
    }

    pub fn with_webhook_port(mut self, port: u16) -> Self {
        self.config.webhook_port = port;
        self
    }

    pub fn with_webhook_config(mut self, config: WebhookConfig) -> Self {
        self.config.webhook = config;
        self
    }

    pub fn with_script_tools(mut self, enable: bool) -> Self {
        self.config.use_script_tools = enable;
        self
    }

    pub fn with_log_buffer(mut self, buffer: crate::log_buffer::LogBuffer) -> Self {
        self.log_buffer = Some(buffer);
        self
    }

    pub async fn build(self) -> DaemonServer {
        let mut server = DaemonServer::new(
            self.config,
            self.embedding,
            self.memory_path,
            self.brave_api_key,
        );
        server.log_buffer = self.log_buffer;

        // Initialize Turso storage. The recovering open verifies the memories
        // table is readable; on page-level corruption it quarantines the
        // damaged file, rebuilds a fresh store at the same path, and salvages
        // every reachable row — so the daemon boots with a working store
        // instead of a silently empty one.
        let db_path = server.config.data_dir.join("nanna.db");
        let storage_config = nanna_storage::StorageConfig {
            path: db_path.to_string_lossy().to_string(),
        };
        match nanna_storage::open_with_recovery(&storage_config).await {
            Ok((storage, recovery)) => {
                info!("Storage initialized at {:?}", db_path);
                if let Some(report) = recovery {
                    warn!(
                        "Memory store was REBUILT after corruption: {} memories recovered \
                         (corrupt copy: {})",
                        report.memories_recovered,
                        report.quarantine_path.display()
                    );
                    server.memory_recovery = Some(Arc::new(report));
                }
                server.set_storage(Arc::new(storage));
            }
            Err(e) => {
                warn!(
                    "Failed to initialize storage: {}. Model stats will not persist.",
                    e
                );
            }
        }

        server
    }
}

impl Default for DaemonBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Helper: convert nanna_config::ChannelsConfig → daemon-local ChannelsConfig
// =============================================================================

/// Map the high-level `nanna_config::ChannelsConfig` (with its richer field names)
/// to the daemon-local `ChannelsConfig` used by `ChannelManager::configure()`.
///
/// Fields that exist in `nanna_config` but not in the daemon-local type are
/// silently dropped — the local type only covers what `ChannelManager` actually
/// needs at runtime.
/// Build model routing tiers for sub-agents from the model priority list.
///
/// Given a priority list like ["claude-opus-4-6", "openrouter/free", "ollama/qwen3.5:9b"],
/// assigns tiers so the cheapest model (last) handles Simple tasks, the most capable
/// (first) handles Complex, and everything in between gets Medium.
///
/// With 1 model: no routing (always use that model).
/// With 2 models: first=Complex, second=Simple.
/// With 3+: first=Complex, last=Simple, middle=Medium.
fn build_sub_agent_routing(model_priority: &[String]) -> Vec<nanna_agent::ModelTier> {
    use nanna_agent::{ModelTier, TaskComplexity};

    if model_priority.len() <= 1 {
        return vec![]; // No routing with a single model
    }

    let mut tiers = Vec::new();

    // Reversed: cheapest first (routing picks the first model whose tier >= complexity)
    for (i, model) in model_priority.iter().enumerate().rev() {
        let tier = if i == 0 {
            TaskComplexity::Complex // Most capable
        } else if i == model_priority.len() - 1 {
            TaskComplexity::Simple // Cheapest
        } else {
            TaskComplexity::Medium
        };
        tiers.push(ModelTier {
            model: model.clone(),
            tier,
        });
    }

    tiers
}

fn build_daemon_channels_config(src: &nanna_config::ChannelsConfig) -> ChannelsConfig {
    use crate::channels::{
        DiscordConfig as DaemonDiscord, SlackConfig as DaemonSlack,
        TelegramConfig as DaemonTelegram,
    };

    ChannelsConfig {
        telegram: src.telegram.as_ref().map(|tg| DaemonTelegram {
            bot_token: tg.bot_token.clone(),
            // nanna_config::TelegramConfig uses webhook_url; treat presence of it as
            // "use webhooks" mode (listener-based polling is disabled when webhook URL set).
            allowed_chats: tg.allowed_users.clone().unwrap_or_default(),
            use_webhooks: tg.webhook_url.is_some(),
        }),
        discord: src.discord.as_ref().map(|dc| DaemonDiscord {
            bot_token: dc.bot_token.clone(),
            allowed_guilds: vec![], // nanna_config::DiscordConfig has no allowed_guilds yet
            intents: None,
        }),
        slack: src.slack.as_ref().and_then(|sl| {
            // Slack Socket Mode listener requires an app_token; fall back gracefully
            sl.app_token.as_ref().map(|app_token| DaemonSlack {
                app_token: app_token.clone(),
                bot_token: sl.bot_token.clone(),
                allowed_channels: vec![], // nanna_config::SlackConfig has no allowed_channels yet
            })
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_webhook_secrets_flow_into_webhook_config() {
        use nanna_config::{ChannelsConfig, DiscordConfig, SlackConfig, WhatsAppConfig};

        let channels = ChannelsConfig {
            discord: Some(DiscordConfig {
                bot_token: "bot".into(),
                application_id: "app".into(),
                public_key: "pubkey-hex".into(),
            }),
            slack: Some(SlackConfig {
                bot_token: "bot".into(),
                app_token: None,
                signing_secret: "slack-signing".into(),
            }),
            whatsapp: Some(WhatsAppConfig {
                connection_method: "cloud-api".into(),
                phone_number_id: None,
                access_token: None,
                verify_token: Some("wa-verify".into()),
                app_secret: Some("wa-app-secret".into()),
                session_name: None,
                allowed_contacts: None,
            }),
            telegram: None,
            signal: None,
        };

        let mut webhook = WebhookConfig::default();
        apply_channel_webhook_secrets(&mut webhook, &channels);

        assert_eq!(webhook.discord_public_key.as_deref(), Some("pubkey-hex"));
        assert_eq!(
            webhook.slack_signing_secret.as_deref(),
            Some("slack-signing")
        );
        assert_eq!(webhook.whatsapp_verify_token.as_deref(), Some("wa-verify"));
        assert_eq!(
            webhook.whatsapp_app_secret.as_deref(),
            Some("wa-app-secret")
        );
    }

    #[test]
    fn absent_channels_leave_webhook_secrets_unset() {
        let channels = nanna_config::ChannelsConfig::default();
        let mut webhook = WebhookConfig::default();
        apply_channel_webhook_secrets(&mut webhook, &channels);
        assert!(webhook.discord_public_key.is_none());
        assert!(webhook.slack_signing_secret.is_none());
        assert!(webhook.whatsapp_app_secret.is_none());
    }

    #[test]
    fn tool_policy_wildcard_enabled_is_unrestricted() {
        // The default `[tools] enabled = ["*"]` must not gate anything.
        let p = build_tool_policy(Some(&["*".to_string()]), &[]);
        assert!(p.is_unrestricted());
    }

    #[test]
    fn tool_policy_none_enabled_is_unrestricted() {
        assert!(build_tool_policy(None, &[]).is_unrestricted());
    }

    #[test]
    fn tool_policy_disabled_denies_even_with_wildcard() {
        // Regression: `disabled` was parsed but never enforced. With the wildcard
        // allowlist a disabled tool must still be refused.
        let p = build_tool_policy(Some(&["*".to_string()]), &["exec".to_string()]);
        assert!(!p.is_unrestricted());
        assert!(!p.permits("exec"));
        assert!(p.permits("read_file"));
    }

    #[test]
    fn tool_policy_explicit_allowlist_restricts() {
        let enabled = vec!["read_file".to_string(), "recall".to_string()];
        let p = build_tool_policy(Some(&enabled), &[]);
        assert!(p.permits("read_file"));
        assert!(!p.permits("exec"));
    }

    #[test]
    fn tool_policy_deny_wins_over_allow() {
        let enabled = vec!["exec".to_string(), "read_file".to_string()];
        let p = build_tool_policy(Some(&enabled), &["exec".to_string()]);
        assert!(!p.permits("exec"));
        assert!(p.permits("read_file"));
    }

    #[test]
    fn tool_policy_empty_allowlist_fails_open_on_allow_side() {
        // A forgotten/empty allowlist must not silently mute every tool; only the
        // denylist should bite.
        let p = build_tool_policy(Some(&[]), &["exec".to_string()]);
        assert!(p.permits("read_file"));
        assert!(!p.permits("exec"));
    }

    #[test]
    fn daemon_heartbeat_prompt_does_not_command_file_read() {
        // The daemon overrides the scheduler default with its own prompt; guard
        // it too so neither site reintroduces the erroring `Read HEARTBEAT.md`.
        let p = DAEMON_HEARTBEAT_PROMPT.to_lowercase();
        assert!(
            !p.contains("read heartbeat"),
            "must not command a file read: {p}"
        );
        assert!(
            !p.contains(".md"),
            "must not reference a bespoke .md file: {p}"
        );
        assert!(
            p.contains("heartbeat_ok"),
            "must keep the HEARTBEAT_OK sentinel: {p}"
        );
    }

    /// Serve exactly one HTTP request with a canned body, then exit.
    ///
    /// Bound on port 0 so the OS assigns a free port — the test never races a
    /// real Ollama or another test for a fixed port. Returns the base URL.
    async fn spawn_one_shot_embedding_server(body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind an ephemeral port");
        let addr = listener.local_addr().expect("read back the bound addr");
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                // Read whatever the client sends; we only need the socket
                // drained enough to reply.
                let mut buf = [0_u8; 4096];
                let _ = stream.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.flush().await;
            }
        });
        format!("http://{addr}")
    }

    fn ollama_router_at(base_url: &str) -> EmbeddingRouter {
        EmbeddingRouter::new(
            EmbeddingProviderInfo {
                name: "ollama".into(),
                model: "nomic-embed-text".into(),
            },
            Arc::new(nanna_llm::EmbeddingClient::ollama(base_url).with_model("nomic-embed-text")),
        )
    }

    /// The probe reports the dimension the provider actually returned — it does
    /// not consult any per-model dimension table.
    #[tokio::test]
    async fn probe_reports_the_dimension_the_provider_returns() {
        let base =
            spawn_one_shot_embedding_server(r#"{"embeddings":[[0.1,0.2,0.3,0.4,0.5]]}"#).await;
        let dim = DaemonServer::probe_embedding_dimension(&ollama_router_at(&base))
            .await
            .expect("probe succeeds against a responsive provider");
        assert_eq!(dim, 5, "dimension comes from the response vector's length");
    }

    /// A provider that answers with an empty vector is an error, not a
    /// zero-dimension memory store.
    #[tokio::test]
    async fn probe_rejects_an_empty_embedding_vector() {
        let base = spawn_one_shot_embedding_server(r#"{"embeddings":[[]]}"#).await;
        let err = DaemonServer::probe_embedding_dimension(&ollama_router_at(&base))
            .await
            .expect_err("an empty vector must not pass as a valid dimension");
        assert!(!err.is_empty(), "the failure carries a reason");
    }

    /// The regression this fixes: an unreachable/unkeyed provider makes the
    /// probe fail, and the caller must be able to carry on. The probe returns
    /// `Err` rather than panicking, so boot can degrade to a provisional
    /// dimension instead of aborting.
    #[tokio::test]
    async fn probe_fails_cleanly_when_no_provider_answers() {
        // Bind then immediately drop the listener, so the port is (almost
        // certainly) closed — a stand-in for "no embedding provider running".
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        drop(listener);

        let err =
            DaemonServer::probe_embedding_dimension(&ollama_router_at(&format!("http://{addr}")))
                .await
                .expect_err("probe must fail when nothing is listening");
        assert!(
            err.contains("embedding providers failed"),
            "the router reports exhausting its providers, got: {err}"
        );
    }

    /// The seed the daemon falls back to must itself be a usable dimension —
    /// this is the value memory runs on until a provider answers.
    #[test]
    fn provisional_seed_dimension_is_positive() {
        assert!(
            nanna_memory::MemoryServiceConfig::default().dimension > 0,
            "a zero seed would make every add() fail before the probe realigns"
        );
    }

    #[test]
    fn scheduled_consolidation_config_threads_user_memory_settings() {
        // The scheduled dream cycle must use the user's compression settings,
        // not ConsolidationConfig::default(), so automatic and IPC-triggered
        // consolidation behave identically.
        let cfg = scheduled_consolidation_config(0.25, 100, 8_192);
        assert!((cfg.max_compression_ratio - 0.25).abs() < f32::EPSILON);
        assert_eq!(cfg.min_remaining_memories, 100);
        // Untouched fields keep their defaults (e.g. the member-count cap).
        let default = nanna_memory::ConsolidationConfig::default();
        assert_eq!(cfg.max_cluster_memories, default.max_cluster_memories);
        assert!((cfg.cluster_threshold - default.cluster_threshold).abs() < f32::EPSILON);
    }

    #[test]
    fn scheduled_consolidation_config_sizes_content_budget_to_the_model() {
        // A large-context summarizer gets a proportionally larger per-cluster
        // content budget than a small one (so big models consolidate more per
        // pass) — the whole point of threading the model's context window.
        let small = scheduled_consolidation_config(0.5, 20, 8_192);
        let large = scheduled_consolidation_config(0.5, 20, 200_000);
        assert!(large.max_cluster_content_bytes > small.max_cluster_content_bytes);
        assert_eq!(
            large.max_cluster_content_bytes,
            nanna_memory::cluster_content_bytes_for_context(200_000)
        );
    }

    /// The P13 unification invariant: the orchestrator the daemon builds reads
    /// the **same** clock the control plane stamps, so a chat request moves the
    /// dream gate without any second bookkeeping call.
    #[tokio::test]
    async fn dreaming_service_gates_on_the_control_plane_clock() {
        let clock = Arc::new(nanna_memory::ActivityClock::new());
        let memory = Arc::new(nanna_memory::MemoryService::new(
            nanna_memory::MemoryServiceConfig::default(),
        ));
        // A 1-hour idle threshold: the gate is shut for as long as the clock
        // says the system was recently used.
        let dreaming = nanna_memory::DreamingService::with_shared_memory(
            nanna_memory::DreamingConfig {
                idle_threshold_secs: 3_600,
                memory_pressure_count: 0,
                ..nanna_memory::DreamingConfig::default()
            },
            memory,
        )
        .with_activity_clock(Arc::clone(&clock));

        // Stamping the clock the way the control plane does on a chat request
        // must be visible to the service…
        clock.record();
        assert!(
            dreaming.idle_duration() < std::time::Duration::from_secs(1),
            "the control plane's stamp must reset the service's idle timer"
        );
        // …and hold the gate shut.
        let ran = dreaming
            .dream_if_idle(|_p| async { Ok(String::new()) })
            .await
            .expect("gate must not error");
        assert!(ran.is_none(), "an actively-used daemon must not dream");
        // Both sides genuinely hold one clock, not two equal ones.
        assert!(Arc::ptr_eq(&dreaming.activity_clock(), &clock));
    }

    #[test]
    fn daemon_config_default_mirrors_consolidation_defaults() {
        let daemon = DaemonConfig::default();
        let cons = nanna_memory::ConsolidationConfig::default();
        assert!(
            (daemon.memory_max_compression_ratio - cons.max_compression_ratio).abs() < f32::EPSILON
        );
        assert_eq!(
            daemon.memory_min_remaining_memories,
            cons.min_remaining_memories
        );
    }

    #[test]
    fn daemon_config_default_mirrors_dreaming_idle_gate_defaults() {
        // The scheduled dream cycle's idle gate must default to the same policy
        // DreamingService uses, so wiring the gate changed no thresholds.
        let daemon = DaemonConfig::default();
        let dream = nanna_memory::DreamingConfig::default();
        assert_eq!(
            daemon.dream_idle_threshold_secs, dream.idle_threshold_secs,
            "daemon idle-gate default must mirror DreamingConfig"
        );
        assert_eq!(
            daemon.dream_memory_pressure_count, dream.memory_pressure_count,
            "daemon memory-pressure default must mirror DreamingConfig"
        );
    }
}
