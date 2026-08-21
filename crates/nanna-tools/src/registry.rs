//! Tool registry for managing available tools

use crate::skills::{discover_skills, load_skill};
use crate::{
    OutputTarget, Tool, ToolCall, ToolDefinition, ToolPolicy, ToolResponse, ToolResult,
    format_tool_output,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[cfg(feature = "scripting")]
use crate::skills::load_skill_with_services;

tokio::task_local! {
    /// The session the *currently executing run* belongs to.
    ///
    /// A `ToolRegistry` is process-wide and its `session_id` field is one slot:
    /// last writer wins. That is survivable for interactive chat, where the
    /// bound session is simply "the session the user is in", but it is the
    /// wrong shape for runs that OVERLAP — and they do, by design. A chat turn
    /// claims per-session (`run_chat_turn`) and rebinds the registry
    /// immediately, so a chat can start while a scheduled run is mid-flight.
    /// Anything that wrote the slot on entry and restored it on exit would then
    /// hand the live chat's binding back to a scheduled session that has ended,
    /// and the next session-scoped tool call would be attributed to the wrong
    /// session.
    ///
    /// A task-local has no slot to fight over: the value rides with the run's
    /// future, so each run reads its own and none can clobber another's. Set it
    /// with [`ToolRegistry::with_run_session`].
    static RUN_SESSION_ID: String;
}

/// Registry of available tools
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    /// Alias names (lowercase aliases ARE included in definitions; capitalized ones are not)
    aliases: RwLock<HashSet<String>>,
    /// Reverse map: alias name → canonical target name
    alias_targets: RwLock<HashMap<String, String>>,
    /// Default working directory for tool execution (global fallback)
    default_workdir: RwLock<Option<std::path::PathBuf>>,
    /// Per-session working directories (session_id → root).
    /// `Some(root)` pins the session there; `None` records that the session
    /// resolved to NO workspace, so it must not fall through to the global
    /// default that another session's turn may own.
    session_workdirs: RwLock<HashMap<String, Option<std::path::PathBuf>>>,
    /// Current session ID (set when agent session starts)
    session_id: RwLock<Option<String>>,
    /// Allow/deny policy over canonical tool names. Default permits everything.
    /// Enforced in `execute` AFTER alias/fuzzy resolution — see `policy` module docs.
    policy: RwLock<ToolPolicy>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(HashMap::new()),
            aliases: RwLock::new(HashSet::new()),
            alias_targets: RwLock::new(HashMap::new()),
            default_workdir: RwLock::new(None),
            session_workdirs: RwLock::new(HashMap::new()),
            session_id: RwLock::new(None),
            policy: RwLock::new(ToolPolicy::allow_all()),
        }
    }

    /// Replace the active tool policy.
    ///
    /// The policy gates execution on *canonical* names, so it survives aliasing
    /// (`Bash` → `exec`) and fuzzy resolution.
    pub async fn set_policy(&self, policy: ToolPolicy) {
        let denied: Vec<&str> = policy.denied().collect();
        if !denied.is_empty() {
            info!("Tool policy: {} tool(s) denied: {:?}", denied.len(), denied);
        }
        *self.policy.write().await = policy;
    }

    /// Snapshot the active tool policy.
    pub async fn policy(&self) -> ToolPolicy {
        self.policy.read().await.clone()
    }

    /// Set the default working directory for tool execution.
    /// Called when the active workspace changes.
    ///
    /// CONTROL PLANE ONLY. A run binds its own root with
    /// [`Self::bind_session_workdir`] and never comes here: this call writes
    /// the process-wide slot, so a run that used it would re-root every other
    /// session that has no binding of its own.
    pub async fn set_default_workdir(&self, workdir: Option<std::path::PathBuf>) {
        // ONLY the process-wide default, which is what a session without a
        // binding of its own falls through to.
        //
        // This deliberately does NOT touch `session_workdirs`. It used to also
        // file the root under whichever session owned the shared slot, and that
        // is the re-root: activating a workspace is a control-plane call, so it
        // carries no run scope, but the shared slot still names whichever chat
        // most recently prepared a turn — including one that is streaming right
        // now. The insert therefore rewrote a live turn's root, and its next
        // tool call resolved into the newly-activated project. That is how a
        // session's writes landed in an unrelated checkout.
        //
        // Nothing needs the convenience any more: a turn binds its own root in
        // `prepare_chat_turn` with `bind_session_workdir`, keyed on the session
        // NAMED there, and releases it on teardown.
        debug_assert!(
            RUN_SESSION_ID.try_with(|_| ()).is_err(),
            "set_default_workdir is control-plane only — a run binds its own root"
        );
        *self.default_workdir.write().await = workdir;
    }

    /// Get the current default working directory.
    /// Returns the per-session workdir if available, otherwise the global default.
    pub async fn default_workdir(&self) -> Option<std::path::PathBuf> {
        // Per-session workdir takes priority — keyed on the session THIS run is
        // executing as, so a scheduled run does not inherit the workdir of
        // whichever chat happens to own the shared binding. A session bound to
        // `None` answers `None`: it deliberately has no root, and falling
        // through to the global default here is the same destruction by
        // another door, because that default belongs to whoever activated a
        // workspace last.
        if let Some(ref sid) = self.current_session_id().await {
            if let Some(bound) = self.session_workdirs.read().await.get(sid) {
                return bound.clone();
            }
        }
        self.default_workdir.read().await.clone()
    }

    /// Set the working directory for a specific session.
    pub async fn set_session_workdir(&self, session_id: &str, workdir: std::path::PathBuf) {
        self.session_workdirs
            .write()
            .await
            .insert(session_id.to_string(), Some(workdir));
    }

    /// Bind (or un-bind) a session's root, keyed on the session NAMED HERE —
    /// never on whichever session happens to own the shared slot. This is the
    /// call a turn makes for itself while another turn may be mid-flight.
    ///
    /// `None` is a binding too, not an absence: it records that this session
    /// resolved to no workspace, which is what keeps it out of the global
    /// default a concurrent turn's workspace activation owns.
    pub async fn bind_session_workdir(
        &self,
        session_id: &str,
        workdir: Option<std::path::PathBuf>,
    ) {
        self.session_workdirs
            .write()
            .await
            .insert(session_id.to_string(), workdir);
    }

    /// Remove a session's workdir (call on session cleanup).
    pub async fn clear_session_workdir(&self, session_id: &str) {
        self.session_workdirs.write().await.remove(session_id);
    }

    /// Set the current session ID.
    /// Called when an agent session starts or changes.
    ///
    /// This is the *shared* binding — the session the daemon is interactively
    /// in. A run that merely overlaps that session (a scheduled task, a
    /// heartbeat) must NOT write here; it scopes itself with
    /// [`Self::with_run_session`] instead.
    pub async fn set_session_id(&self, session_id: Option<String>) {
        *self.session_id.write().await = session_id;
    }

    /// Get the current session ID.
    ///
    /// A run-scoped binding from [`Self::with_run_session`] wins over the
    /// shared one, so a scheduled run reads its own session even while a chat
    /// turn owns the shared slot.
    pub async fn session_id(&self) -> Option<String> {
        self.current_session_id().await
    }

    /// The session in effect for the caller: the run-scoped binding when the
    /// caller is inside one, the shared binding otherwise.
    async fn current_session_id(&self) -> Option<String> {
        if let Ok(session_id) = RUN_SESSION_ID.try_with(Clone::clone) {
            return Some(session_id);
        }
        self.session_id.read().await.clone()
    }

    /// Run `future` with `session_id` as the session every tool call inside it
    /// scopes to, leaving the shared binding untouched.
    ///
    /// EVERY run wraps itself in this — an interactive chat turn just as much
    /// as a scheduled task, a heartbeat or a sub-agent. Chat is not the
    /// exception: two chat turns overlap whenever one is still streaming as the
    /// next begins, and the incoming turn's workspace resolution used to re-key
    /// the outgoing turn's root under the shared slot, so a running turn
    /// resolved its relative paths against another project (that is how a
    /// 3,718-line file was overwritten).
    ///
    /// Session-scoped tools (`todo` above all) read their scope from
    /// `Nanna.sessionId()`, which is this value; the scheduler used to call
    /// `agent.chat(&session_id, ..)` without supplying it at all, so
    /// `Nanna.sessionId()` was null and every session-scoped `todo` call died
    /// on "session scope requires session_id" (35 logged failures 2026-07-28 ..
    /// 07-31, all of them `scheduled-heartbeat-*`).
    ///
    /// The binding covers the whole future and nothing else: the agent loop
    /// executes tool calls inline (it never `spawn`s one onto another task), so
    /// [`Self::session_id`] resolves to `session_id` throughout the run, and a
    /// concurrently polled chat future in the same task still reads the shared
    /// binding.
    ///
    /// CONSTRAINT for whoever comes next: a task-local does NOT cross
    /// `tokio::spawn`. Tool execution that is spawned off the run's own future
    /// silently loses this scope and falls back to the shared slot —
    /// reintroducing the bug with no compile error. Execution must stay inside
    /// the future: `execute_parallel` uses `join_all`, the task tool awaits the
    /// spawner inline, and nothing under this crate spawns a tool call.
    pub async fn with_run_session<F>(session_id: String, future: F) -> F::Output
    where
        F: std::future::Future,
    {
        RUN_SESSION_ID.scope(session_id, future).await
    }

    /// Register a tool
    pub async fn register<T: Tool + 'static>(&self, tool: T) {
        let definition = tool.definition();
        let name = definition.name.clone();
        let mut tools = self.tools.write().await;
        tools.insert(name.clone(), Arc::new(tool));
        info!("Registered tool: {}", name);
    }

    /// Register a boxed tool
    pub async fn register_boxed(&self, tool: Arc<dyn Tool>) {
        let definition = tool.definition();
        let name = definition.name.clone();
        let mut tools = self.tools.write().await;
        tools.insert(name.clone(), tool);
        info!("Registered tool: {}", name);
    }

    /// Register an alias for an existing tool.
    /// This allows the same tool to be called by different names.
    /// Lowercase aliases ARE included in API definitions (with correct parameter schemas).
    /// Capitalized aliases (e.g., `Read`, `Bash`) are for execution only.
    pub async fn register_alias(&self, alias: &str, target: &str) {
        let tools = self.tools.read().await;
        if let Some(tool) = tools.get(target).cloned() {
            drop(tools); // Release read lock before acquiring write lock
            let mut tools = self.tools.write().await;
            tools.insert(alias.to_string(), tool);
            drop(tools);
            // Track this as an alias so we don't include it in definitions
            let mut aliases = self.aliases.write().await;
            aliases.insert(alias.to_string());
            drop(aliases);
            // Store reverse mapping: alias → canonical target
            let mut targets = self.alias_targets.write().await;
            targets.insert(alias.to_string(), target.to_string());
            info!("Registered tool alias: {} -> {}", alias, target);
        } else {
            warn!(
                "Cannot create alias '{}': target tool '{}' not found",
                alias, target
            );
        }
    }

    /// Unregister a tool, cascading to any aliases that resolve to it.
    ///
    /// This is the counterpart to [`register`](Self::register) /
    /// [`register_alias`](Self::register_alias) and is what makes a **deleted or
    /// disabled** tool stop being callable *without* a daemon restart (previously
    /// the registry had no removal path, so a deleted tool stayed live until the
    /// process was restarted).
    ///
    /// Semantics:
    /// - If `name` is a canonical tool, it is removed **and** every alias whose
    ///   target is `name` is removed too — so a deleted tool can't be reached
    ///   through a lingering alias.
    /// - If `name` is itself an alias, only that alias entry is removed; the
    ///   canonical target is left intact.
    ///
    /// Returns the number of registry entries removed (`0` if `name` was unknown).
    pub async fn unregister(&self, name: &str) -> usize {
        debug_assert!(!name.is_empty(), "unregister called with empty name");

        let mut tools = self.tools.write().await;
        let mut aliases = self.aliases.write().await;
        let mut alias_targets = self.alias_targets.write().await;

        // Aliases pointing at `name` cascade with the canonical delete. Bounded by
        // the number of registered aliases (finite). Exclude `name` itself so a
        // self-referential entry isn't double-counted.
        let dependent_aliases: Vec<String> = alias_targets
            .iter()
            .filter(|(alias, target)| target.as_str() == name && alias.as_str() != name)
            .map(|(alias, _)| alias.clone())
            .collect();

        let mut removed = 0usize;
        for alias in &dependent_aliases {
            if tools.remove(alias).is_some() {
                removed += 1;
            }
            aliases.remove(alias);
            alias_targets.remove(alias);
        }

        if tools.remove(name).is_some() {
            removed += 1;
        }
        aliases.remove(name);
        alias_targets.remove(name);

        // Postcondition: no entry (canonical or alias) named `name` survives, and
        // no alias still targets `name`.
        debug_assert!(
            !tools.contains_key(name),
            "unregister must leave no entry named '{name}'"
        );
        debug_assert!(
            !alias_targets.values().any(|t| t == name),
            "unregister must leave no alias targeting '{name}'"
        );

        // Release the three write locks before the (non-critical) logging tail.
        drop(tools);
        drop(aliases);
        drop(alias_targets);

        if removed > 0 {
            let plural = if removed == 1 { "entry" } else { "entries" };
            info!("Unregistered tool '{name}' ({removed} {plural} removed)");
        }
        removed
    }

    /// Get a tool by name (exact match only)
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        let tools = self.tools.read().await;
        tools.get(name).cloned()
    }

    /// Get the canonical name for a tool (resolves aliases).
    /// Returns the alias target if `name` is a known alias, otherwise returns `name` as-is.
    pub async fn canonical_name(&self, name: &str) -> String {
        let targets = self.alias_targets.read().await;
        targets
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }

    /// Multi-step tool resolution: exact → case-insensitive → dialect synonym
    /// → fuzzy.
    ///
    /// Returns `(resolved_name, tool)` if found. The resolved name is the
    /// key the tool was registered under (may differ in case from `name`).
    pub async fn resolve_tool(&self, name: &str) -> Option<(String, Arc<dyn Tool>)> {
        let tools = self.tools.read().await;

        // Step 1: Exact match
        if let Some(tool) = tools.get(name) {
            return Some((name.to_string(), tool.clone()));
        }

        // Step 2: Case-insensitive match
        let lower = name.to_lowercase();
        for (key, tool) in tools.iter() {
            if key.to_lowercase() == lower {
                info!(
                    requested = name,
                    resolved = key,
                    step = "case-insensitive",
                    "Tool resolved"
                );
                return Some((key.clone(), tool.clone()));
            }
        }

        // Step 2.5: Dialect synonym — the verb/noun another tool universe
        // taught the model ("ls", "cat", "run", …) mapped to the one tool it
        // can mean here. Runs BEFORE fuzzy so an unambiguous synonym never
        // depends on edit distance ("ls" vs "list_dir" scores 0.25 — fuzzy
        // could never save it). Only fires when nothing registered matched
        // above, and only resolves when the target actually exists in this
        // registry, so a synonym can never shadow a real tool or invent one.
        if let Some(target) = dialect_synonym(&lower) {
            if let Some(tool) = tools.get(target) {
                info!(
                    requested = name,
                    resolved = target,
                    step = "synonym",
                    "Tool resolved via dialect synonym"
                );
                return Some((target.to_string(), tool.clone()));
            }
        }

        // Step 3: Fuzzy match — pick best if score ≥ 0.7 AND gap to second-best ≥ 0.1
        let mut best: Option<(String, f64, Arc<dyn Tool>)> = None;
        let mut second_best_score: f64 = 0.0;

        for (key, tool) in tools.iter() {
            let score = normalized_similarity(&lower, &key.to_lowercase());
            match best {
                Some((_, bs, _)) if score > bs => {
                    second_best_score = bs;
                    best = Some((key.clone(), score, tool.clone()));
                }
                Some((_, bs, _)) if score > second_best_score && score <= bs => {
                    second_best_score = score;
                }
                None => {
                    best = Some((key.clone(), score, tool.clone()));
                }
                _ => {}
            }
        }

        if let Some((key, score, tool)) = best {
            let gap = score - second_best_score;
            if score >= 0.7 && gap >= 0.1 {
                info!(
                    requested = name,
                    resolved = key,
                    score = format!("{score:.2}"),
                    gap = format!("{gap:.2}"),
                    step = "fuzzy",
                    "Tool resolved via fuzzy match"
                );
                return Some((key, tool));
            }
            debug!(
                requested = name,
                best = key,
                score = format!("{score:.2}"),
                gap = format!("{gap:.2}"),
                "Fuzzy match rejected (score or gap too low)"
            );
        }

        None
    }

    /// Get all tool definitions.
    /// Includes lowercase aliases (with the target tool's schema but the alias name)
    /// so the LLM knows correct parameters regardless of which name it uses.
    /// Capitalized aliases (e.g., `Read`, `Bash`) are excluded to avoid bloat.
    pub async fn definitions(&self) -> Vec<ToolDefinition> {
        let tools = self.tools.read().await;
        let aliases = self.aliases.read().await;
        let alias_targets = self.alias_targets.read().await;
        let policy = self.policy.read().await;
        tools
            .iter()
            .filter(|(name, _)| {
                // Include non-aliases AND lowercase-only aliases
                !aliases.contains(name.as_str()) || name.chars().all(|c| !c.is_uppercase())
            })
            // Don't advertise a tool the policy will refuse. `execute` is the
            // actual boundary; this only keeps the prompt honest (and smaller).
            .filter(|(name, _)| {
                let canonical = alias_targets
                    .get(name.as_str())
                    .map_or(name.as_str(), String::as_str);
                policy.permits(canonical)
            })
            .map(|(name, t)| {
                let mut def = t.definition();
                def.name = name.clone(); // Override name to match registered key
                def
            })
            .collect()
    }

    /// Get tool definitions for a specific set of tool names.
    ///
    /// Returns definitions only for tools whose registered name is in `names`.
    /// Lowercase aliases are included ONLY if their canonical target is NOT already
    /// in `names` (to avoid duplicates after OAuth tool-name remapping, where e.g.
    /// both `read` and `read_file` map to `Read`).
    /// Capitalized aliases are excluded as usual.
    pub async fn definitions_for_names(&self, names: &HashSet<String>) -> Vec<ToolDefinition> {
        let tools = self.tools.read().await;
        let aliases = self.aliases.read().await;
        let alias_targets = self.alias_targets.read().await;
        let policy = self.policy.read().await;
        tools
            .iter()
            .filter(|(name, _tool)| {
                let canonical = alias_targets
                    .get(name.as_str())
                    .map_or(name.as_str(), String::as_str);
                policy.permits(canonical)
            })
            .filter(|(name, _tool)| {
                let is_alias = aliases.contains(name.as_str());
                let is_capitalized_alias = is_alias && name.chars().any(|c| c.is_uppercase());

                // Skip capitalized aliases
                if is_capitalized_alias {
                    return false;
                }

                // For a lowercase alias: skip if canonical target is also in `names`
                // (both would map to the same Claude Code tool name, e.g. read+read_file → Read)
                if is_alias {
                    if let Some(canonical) = alias_targets.get(name.as_str()) {
                        if names.contains(canonical) {
                            return false;
                        }
                    }
                    return names.contains(name.as_str());
                }

                // Regular (non-alias) tool: check if its name is in `names`
                names.contains(name.as_str())
            })
            .map(|(name, t)| {
                let mut def = t.definition();
                def.name = name.clone();
                def
            })
            .collect()
    }

    /// Rank canonical tools against `query` — BM25 + porter2 stemming with a
    /// per-term fuzzy-typo fallback (see [`crate::search`]).
    ///
    /// Returns up to `limit` hits, best first, ties broken by name for
    /// determinism. Alias entries are excluded entirely (canonical names
    /// only — no duplicate `write`/`write_file` rows), and, mirroring
    /// [`definitions`](Self::definitions), tools the active policy denies are
    /// never surfaced.
    pub async fn search_tools(&self, query: &str, limit: usize) -> Vec<crate::ToolSearchHit> {
        let docs: Vec<crate::SearchDoc> = {
            let tools = self.tools.read().await;
            let aliases = self.aliases.read().await;
            let policy = self.policy.read().await;
            tools
                .iter()
                // Aliases point at a canonical entry that is also in the map;
                // dropping every alias (lowercase or capitalized) leaves
                // exactly the canonical corpus.
                .filter(|(name, _)| !aliases.contains(name.as_str()))
                .filter(|(name, _)| policy.permits(name.as_str()))
                .map(|(name, t)| crate::SearchDoc {
                    name: name.clone(),
                    description: t.definition().description,
                })
                .collect()
        };
        crate::search::search_docs(&docs, query, limit)
    }

    /// Get tool definitions in Anthropic format.
    /// Includes lowercase aliases so the LLM sees correct parameter schemas.
    pub async fn to_anthropic_format(&self) -> Vec<Value> {
        self.definitions()
            .await
            .into_iter()
            .map(|d| d.to_anthropic_format())
            .collect()
    }

    /// Get tool definitions in `OpenAI` format.
    /// Includes lowercase aliases so the LLM sees correct parameter schemas.
    pub async fn to_openai_format(&self) -> Vec<Value> {
        self.definitions()
            .await
            .into_iter()
            .map(|d| d.to_openai_format())
            .collect()
    }

    /// Enforce the tool policy on an already-resolved call.
    ///
    /// `resolved_name` is the registry key `resolve_tool` landed on; it is
    /// canonicalized here so a denied tool cannot be reached via an alias, a
    /// case variant, or a fuzzy near-miss. Returns `Some(refusal)` to short-circuit
    /// `execute`, or `None` when the call is permitted.
    async fn refuse_by_policy(&self, call: &ToolCall, resolved_name: &str) -> Option<ToolResponse> {
        let canonical = self.canonical_name(resolved_name).await;
        // Bind the verdict so the read guard drops before we build the response.
        let verdict = self.policy.read().await.check(&canonical);
        let reason = verdict.err()?;
        warn!(
            requested = %call.name,
            canonical = %canonical,
            "Tool call refused: {}",
            reason.as_str()
        );
        Some(ToolResponse {
            id: call.id.clone(),
            name: call.name.clone(),
            result: ToolResult::error(format!(
                "Tool '{canonical}' is {} and cannot be called.",
                reason.as_str()
            )),
            output_target: OutputTarget::default(),
        })
    }

    /// Execute a tool call
    pub async fn execute(&self, call: ToolCall) -> ToolResponse {
        // Log input parameters (truncated for readability)
        let params_str = serde_json::to_string(&call.parameters).unwrap_or_default();
        let params_preview = if params_str.len() > 300 {
            let end = truncate_boundary(&params_str, 300);
            format!("{}...", &params_str[..end])
        } else {
            params_str
        };
        debug!(
            "Executing tool: {} (id: {}) with params: {}",
            call.name, call.id, params_preview
        );

        let (resolved_name, tool) = match self.resolve_tool(&call.name).await {
            Some(pair) => pair,
            None => {
                warn!("Tool not found: {}", call.name);
                return ToolResponse {
                    id: call.id,
                    name: call.name.clone(),
                    result: ToolResult::error(format!(
                        "Tool not found: {}. Use discover_tools to see available tools.",
                        call.name
                    )),
                    output_target: OutputTarget::default(),
                };
            }
        };

        if resolved_name != call.name {
            debug!("Tool '{}' resolved to '{}'", call.name, resolved_name);
        }

        // Policy gate. Deliberately AFTER resolution: `resolve_tool` matches
        // exact → case-insensitive → fuzzy, and aliases map onto canonical
        // tools, so gating on `call.name` would let `Bash`, `EXEC`, or a fuzzy
        // near-miss slip past a denylist entry for `exec`.
        if let Some(refusal) = self.refuse_by_policy(&call, &resolved_name).await {
            return refusal;
        }

        let output_target = tool.output_target();
        let start = std::time::Instant::now();

        // Normalize camelCase parameter keys to snake_case.
        // Weaker models (OpenRouter free, small Ollama) often send "filePath"
        // instead of "file_path", etc. We add snake_case aliases without
        // removing the originals so either convention works.
        let parameters = normalize_param_keys(call.parameters.clone());

        // Execute under the backstop timer when the tool declares a ceiling.
        // That ceiling is a floor for this timer, not the timer itself — see
        // `backstop_timeout` for why the backstop has to outlive whatever the
        // tool enforces for itself.
        let result = if let Some(timeout_secs) = tool.timeout_secs() {
            // Only a tool that DECLARES a timeout parameter may have its
            // caller extend the outer net. Passing the raw arguments for every
            // tool let a stray `timeout` key stretch the backstop past the
            // ceiling of a tool that cannot honour it — the safety net is the
            // declared ceiling for those, not whatever the caller typed.
            let declares_timeout = tool
                .definition()
                .parameters
                .iter()
                .any(|p| p.name == "timeout");
            let backstop = if declares_timeout {
                backstop_timeout(timeout_secs, &parameters)
            } else {
                backstop_timeout(timeout_secs, &HashMap::new())
            };
            match tokio::time::timeout(backstop, tool.execute(parameters.clone())).await {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => {
                    warn!("Tool {} execution error: {}", call.name, e);
                    ToolResult::error(e.to_string())
                }
                Err(_) => {
                    let elapsed_ms = start.elapsed().as_millis();
                    let limit_ms = backstop.as_millis();
                    warn!(
                        "Tool {} hit the registry backstop after {}ms (limit {}ms, ceiling {}s)",
                        call.name, elapsed_ms, limit_ms, timeout_secs
                    );
                    ToolResult::error(backstop_message(&call.name, elapsed_ms, limit_ms))
                }
            }
        } else {
            match tool.execute(parameters.clone()).await {
                Ok(result) => result,
                Err(e) => {
                    warn!("Tool {} execution error: {}", call.name, e);
                    ToolResult::error(e.to_string())
                }
            }
        };

        let duration_ms = start.elapsed().as_millis();

        // Prefer compact structured data when the call requested JSON output
        // and the tool attached an `output_schema` / structured `data` payload.
        // Text mode (default) is untouched — content stays as the tool wrote it.
        let mut result = result;
        if result.success {
            let def = tool.definition();
            let formatted = format_tool_output(&result, Some(&def), &parameters);
            if formatted != result.content {
                result.content = formatted;
            }
        }

        // Log result summary. On failure the error text lives in `result.error`
        // (`ToolResult::error` leaves `content` empty), so the preview prefers it.
        let output_preview = result_log_preview(&result);

        if result.success {
            debug!(
                "Tool {} completed in {}ms: {}",
                call.name, duration_ms, output_preview
            );
        } else {
            warn!(
                "Tool {} failed in {}ms: {}",
                call.name, duration_ms, output_preview
            );
        }

        ToolResponse {
            id: call.id,
            name: call.name,
            result,
            output_target,
        }
    }

    /// Execute multiple tool calls in parallel
    pub async fn execute_parallel(&self, calls: Vec<ToolCall>) -> Vec<ToolResponse> {
        let futures: Vec<_> = calls.into_iter().map(|call| self.execute(call)).collect();
        futures::future::join_all(futures).await
    }

    /// Load user-authored skills from a directory
    ///
    /// Discovers and loads all skills from the given directory (e.g., `workspace/skills/`).
    /// Returns the number of skills successfully loaded.
    pub async fn load_skills(&self, skills_dir: &Path) -> usize {
        let discovered = discover_skills(skills_dir);
        let total = discovered.len();
        let mut loaded = 0;

        for skill in discovered {
            match load_skill(&skill.path).await {
                Ok(tool) => {
                    let name = tool.definition().name.clone();
                    self.register_boxed(tool).await;
                    info!(name = %name, path = ?skill.path, "Loaded skill");
                    loaded += 1;
                }
                Err(e) => {
                    warn!(name = %skill.name, error = %e, "Failed to load skill");
                }
            }
        }

        info!(loaded, total, "Skills loaded from {:?}", skills_dir);
        loaded
    }

    /// Load user-authored skills from a directory with service functions
    ///
    /// Like `load_skills` but attaches service functions to scripted tools.
    ///
    /// Takes `Arc<Self>` so every scripted tool gets a weak registry handle —
    /// that handle is how tools see the active workspace workdir and session
    /// id at execute time (without it relative paths fall back to HOME).
    #[cfg(feature = "scripting")]
    pub async fn load_skills_with_services(
        self: &Arc<Self>,
        skills_dir: &Path,
        services: &HashMap<String, nanna_scripting::ServiceFn>,
    ) -> usize {
        let discovered = discover_skills(skills_dir);
        let total = discovered.len();
        let mut loaded = 0;

        for skill in discovered {
            // A tool that declares `requires: [...]` is only registered when
            // every named service is actually present. An advertised tool that
            // can only fail is worse than an absent one: the model cannot tell
            // "permanently broken" from "try again", so it retries. Observed
            // live 2026-07-26 — with memory disabled, `reflect` failed on all
            // 4 calls (`Service not found: memory.list`) and `list_reminders`
            // on all 3 (`schedule.list`, registered nowhere), and the run
            // spent its tail looping over them instead of building anything.
            if let Some(missing) = missing_required_services(&skill.path, services) {
                info!(
                    name = %skill.name,
                    missing = %missing.join(", "),
                    "Skill not registered: required services unavailable"
                );
                continue;
            }
            match load_skill_with_services(&skill.path, services, Some(Arc::downgrade(self))).await
            {
                Ok(tool) => {
                    let name = tool.definition().name.clone();
                    self.register_boxed(tool).await;
                    info!(name = %name, path = ?skill.path, "Loaded skill with services");
                    loaded += 1;
                }
                Err(e) => {
                    warn!(name = %skill.name, error = %e, "Failed to load skill");
                }
            }
        }

        info!(loaded, total, "Skills loaded from {:?}", skills_dir);
        loaded
    }

    /// Check if a tool is registered
    pub async fn has(&self, name: &str) -> bool {
        let tools = self.tools.read().await;
        tools.contains_key(name)
    }


    /// Get the number of registered tools
    pub async fn len(&self) -> usize {
        let tools = self.tools.read().await;
        tools.len()
    }

    /// Get all registered tool names (excluding aliases)
    pub async fn tool_names(&self) -> Vec<String> {
        let tools = self.tools.read().await;
        let aliases = self.aliases.read().await;
        tools
            .keys()
            .filter(|k| !aliases.contains(k.as_str()))
            .cloned()
            .collect()
    }

    /// Check if the registry is empty
    pub async fn is_empty(&self) -> bool {
        let tools = self.tools.read().await;
        tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Dialect synonyms: names other tool universes taught weak models, each
/// mapped to the ONE canonical nanna tool it can mean (P22 Tier 4 — observed
/// live 2026-08-12: an lfm leg wrote 300 prose calls to the non-existent
/// `list_files` over four hours; the model was capable, the dialect was not).
///
/// Losslessness rule (owner): only unambiguous entries. A verb that could
/// mean two registered tools ("delete" — a file? a memory?; "search" — the
/// web? memory? file contents?) is deliberately ABSENT: an ambiguous name
/// must surface as unresolved so the caller can say so, never be guessed.
/// The table is consulted after exact and case-insensitive matching, so a
/// registered tool or alias by one of these names always wins, and an entry
/// resolves only when its target is actually registered.
const DIALECT_SYNONYMS: &[(&str, &str)] = &[
    // directory listing
    ("list_files", "list_dir"),
    ("list_directory", "list_dir"),
    ("ls", "list_dir"),
    ("dir", "list_dir"),
    // file reading
    ("cat", "read_file"),
    ("open", "read_file"),
    ("open_file", "read_file"),
    // file writing
    ("create_file", "write_file"),
    ("save_file", "write_file"),
    // shell execution
    ("run", "exec"),
    ("shell", "exec"),
    ("sh", "exec"),
    ("bash", "exec"),
    ("run_command", "exec"),
    ("run_shell_command", "exec"),
    ("execute", "exec"),
    ("execute_command", "exec"),
];

/// Look up the canonical target for a dialect synonym (`name` must already be
/// lowercased). Public so the agent loop's prose-call salvage can explain a
/// mapping ("`list_files` → `list_dir`") in its corrective notice.
#[must_use]
pub fn dialect_synonym(name: &str) -> Option<&'static str> {
    DIALECT_SYNONYMS
        .iter()
        .find(|(from, _)| *from == name)
        .map(|(_, to)| *to)
}

/// Classic Levenshtein edit distance (single-row DP).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();

    for (i, ca) in a.iter().enumerate() {
        let mut prev = i;
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            let val = (row[j + 1] + 1).min(row[j] + 1).min(prev + cost);
            prev = row[j + 1];
            row[j + 1] = val;
        }
    }
    row[b.len()]
}

/// Normalized similarity: 1.0 means identical, 0.0 means completely different.
fn normalized_similarity(a: &str, b: &str) -> f64 {
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - (levenshtein(a, b) as f64 / max_len as f64)
}

/// Find the largest byte index <= max_bytes that is a valid char boundary.
/// Convert a camelCase string to snake_case.
fn camel_to_snake(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() {
            if i > 0 {
                result.push('_');
            }
            result.push(ch.to_lowercase().next().unwrap_or(ch));
        } else {
            result.push(ch);
        }
    }
    result
}

/// Normalize parameter keys from camelCase to snake_case.
///
/// Adds snake_case aliases for any camelCase keys without removing originals.
/// Example: `{"filePath": "x"}` → `{"filePath": "x", "file_path": "x"}`
fn normalize_param_keys(
    mut params: HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    let aliases: Vec<(String, serde_json::Value)> = params
        .iter()
        .filter_map(|(k, v)| {
            let snake = camel_to_snake(k);
            if snake != *k && !params.contains_key(&snake) {
                Some((snake, v.clone()))
            } else {
                None
            }
        })
        .collect();
    for (key, val) in aliases {
        params.insert(key, val);
    }
    params
}

/// Wall-clock ceiling for the registry's backstop timer on one call.
///
/// `timeout_secs` is what the tool declares for itself, out of a static
/// manifest that has never seen this call. A scripted tool's engine knows more:
/// it derives its deadline from this call's `timeout` input and deliberately
/// extends past it so the shell bridge — the layer that can actually kill the
/// child — fires first, and it answers with elapsed time, which deadline fired,
/// and what is on disk. A backstop pinned to the declared ceiling preempts that
/// answer. It also loses when the two are nominally equal, because the inner
/// handoff still costs something: observed as the backstop firing at 180_004 ms
/// with the tool's own account arriving 1.03 s later, to nobody.
///
/// So derive the backstop from the deadline the engine will really enforce plus
/// one more handoff margin — the same slack the engine already stacks above the
/// bridge — and the inner message wins by construction. A caller who asks for a
/// longer command deadline stops being cut short as a side effect.
#[cfg(feature = "scripting")]
fn backstop_timeout(timeout_secs: u64, parameters: &HashMap<String, Value>) -> std::time::Duration {
    std::time::Duration::from_millis(nanna_scripting::ScriptEngine::supervising_timeout_ms(
        timeout_secs.saturating_mul(1000),
        parameters.get("timeout"),
    ))
}

/// Without the scripting engine there is no inner deadline to outlive: this
/// timer is the only one, so it fires exactly at the declared ceiling.
#[cfg(not(feature = "scripting"))]
fn backstop_timeout(
    timeout_secs: u64,
    _parameters: &HashMap<String, Value>,
) -> std::time::Duration {
    std::time::Duration::from_secs(timeout_secs)
}

/// What the backstop says when it has to abandon a call.
///
/// This is the last resort, reached only when the tool's own deadline did not
/// report first, so it has nothing to relay about the work. Say that plainly:
/// how long we actually waited, that this was the outer net rather than the
/// tool's own limit, and that a tool killed mid-flight may already have written
/// part of its work — disk is the truth, not this message.
fn backstop_message(tool_name: &str, elapsed_ms: u128, limit_ms: u128) -> String {
    format!(
        "Tool '{tool_name}' was still running {elapsed_ms}ms after it was called and hit \
         the registry's {limit_ms}ms backstop. That is the outer safety net, not the \
         tool's own deadline, so the tool never reported what it managed to do. Anything \
         it finished before this point is on disk; disk is truth, so check the current \
         state before re-running. If the work genuinely needs longer and this tool \
         accepts a `timeout` (seconds), raise it."
    )
}

fn truncate_boundary(s: &str, max_bytes: usize) -> usize {
    if s.len() <= max_bytes {
        return s.len();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// Build a bounded, char-boundary-safe log preview for a tool result.
///
/// On failure the message lives in `error` (`ToolResult::error` leaves
/// `content` empty), so prefer it; fall back to `content` for tools that set
/// `success = false` with their own content and no `error`. Without this the
/// failure log line rendered empty (`"Tool exec failed in 1ms: "`), hiding the
/// actual reason.
fn result_log_preview(result: &ToolResult) -> String {
    let source = if result.success {
        result.content.as_str()
    } else {
        result
            .error
            .as_deref()
            .filter(|e| !e.is_empty())
            .unwrap_or(result.content.as_str())
    };
    if source.len() > 200 {
        let end = truncate_boundary(source, 200);
        format!("{}...", &source[..end])
    } else {
        source.to_string()
    }
}

/// Services a skill declares as mandatory but the host does not provide.
///
/// Returns `None` when the skill is fine to register — either it declares no
/// requirements, or every declared one is present. Any parse or read failure
/// is also `None` (permissive): a tool must never disappear because its
/// annotation could not be read.
#[cfg(feature = "scripting")]
fn missing_required_services(
    skill_dir: &Path,
    services: &HashMap<String, nanna_scripting::ServiceFn>,
) -> Option<Vec<String>> {
    // Scripted skills only (same tool.ts/tool.js precedence the loader uses);
    // manifest/executable skills declare no service requirements.
    let ts = skill_dir.join("tool.ts");
    let source_path = if ts.exists() { ts } else { skill_dir.join("tool.js") };
    let source = std::fs::read_to_string(source_path).ok()?;
    let manifest = nanna_scripting::extract_manifest(&source)?;

    let missing: Vec<String> = manifest
        .requires
        .into_iter()
        .filter(|svc| !services.contains_key(svc))
        .collect();
    if missing.is_empty() { None } else { Some(missing) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtin::EchoTool;

    // --- levenshtein / normalized_similarity ---

    #[test]
    fn levenshtein_identical() {
        assert_eq!(levenshtein("abc", "abc"), 0);
    }

    #[test]
    fn levenshtein_empty() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("read_file", "reed_file"), 1);
    }

    #[test]
    fn normalized_similarity_identical() {
        let s = normalized_similarity("read_file", "read_file");
        assert!((s - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn normalized_similarity_empty() {
        let s = normalized_similarity("", "");
        assert!((s - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn normalized_similarity_close() {
        // "reed_file" vs "read_file" — 1 edit out of 9 chars → ~0.89
        let s = normalized_similarity("reed_file", "read_file");
        assert!(s > 0.85);
    }

    #[test]
    fn normalized_similarity_distant() {
        let s = normalized_similarity("xyz", "read_file");
        assert!(s < 0.3);
    }

    // --- resolve_tool ---

    #[tokio::test]
    async fn resolve_tool_exact() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;

        let result = reg.resolve_tool("echo").await;
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "echo");
    }

    #[tokio::test]
    async fn resolve_tool_case_insensitive() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;

        let result = reg.resolve_tool("Echo").await;
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "echo");
    }

    #[tokio::test]
    async fn resolve_tool_fuzzy() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;

        // "echoo" is close enough to "echo" (score ~0.8, only one tool so gap is large)
        let result = reg.resolve_tool("echoo").await;
        assert!(result.is_some());
        let (name, _) = result.unwrap();
        assert_eq!(name, "echo");
    }

    #[tokio::test]
    async fn resolve_tool_no_match() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;

        let result = reg.resolve_tool("completely_unrelated_tool").await;
        assert!(result.is_none());
    }

    // --- dialect synonyms (P22 Tier 4) ---

    /// A tool registered under the synonym's target name so the synonym has
    /// something real to land on.
    struct NamedEchoTool(&'static str);

    #[async_trait::async_trait]
    impl Tool for NamedEchoTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition::new(self.0, "test tool")
        }

        async fn execute(
            &self,
            _params: HashMap<String, Value>,
        ) -> Result<ToolResult, crate::ToolError> {
            Ok(ToolResult::success("named-ok"))
        }
    }

    #[tokio::test]
    async fn synonym_resolves_when_target_is_registered() {
        let reg = ToolRegistry::new();
        reg.register(NamedEchoTool("list_dir")).await;

        for written in ["list_files", "ls", "dir", "list_directory"] {
            let (resolved, _) = reg
                .resolve_tool(written)
                .await
                .unwrap_or_else(|| panic!("`{written}` must resolve via synonym"));
            assert_eq!(resolved, "list_dir", "`{written}` → list_dir");
        }
    }

    #[tokio::test]
    async fn synonym_is_case_insensitive_via_the_lowercase_step() {
        let reg = ToolRegistry::new();
        reg.register(NamedEchoTool("exec")).await;

        // resolve_tool lowercases before the synonym step, so shouting works.
        let (resolved, _) = reg.resolve_tool("RUN").await.expect("RUN must resolve");
        assert_eq!(resolved, "exec");
    }

    #[tokio::test]
    async fn synonym_without_registered_target_does_not_resolve() {
        // `cat` → `read_file`, but no `read_file` is registered here: a
        // synonym must never invent a tool.
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;

        assert!(reg.resolve_tool("cat").await.is_none());
    }

    #[tokio::test]
    async fn registered_name_shadows_the_synonym_table() {
        // A REAL tool registered as `run` must win over the `run` → `exec`
        // synonym: the table only speaks when the registry has no answer.
        let reg = ToolRegistry::new();
        reg.register(NamedEchoTool("run")).await;
        reg.register(NamedEchoTool("exec")).await;

        let (resolved, _) = reg.resolve_tool("run").await.expect("run is registered");
        assert_eq!(resolved, "run", "exact match must beat the synonym table");
    }

    #[test]
    fn ambiguous_verbs_are_deliberately_absent_from_the_synonym_table() {
        // Losslessness: a name that could mean more than one registered tool
        // must surface as unresolved, never be guessed.
        for ambiguous in ["delete", "remove", "search", "find", "list", "get"] {
            assert!(
                dialect_synonym(ambiguous).is_none(),
                "`{ambiguous}` is ambiguous and must not be in the synonym table"
            );
        }
    }

    #[test]
    fn every_synonym_entry_has_one_target_and_no_chains() {
        // The table maps each name to exactly one target, and no target is
        // itself a synonym source (no chains — resolution is single-step).
        let mut seen = std::collections::HashSet::new();
        for (from, to) in DIALECT_SYNONYMS {
            assert!(seen.insert(*from), "duplicate synonym source `{from}`");
            assert!(
                dialect_synonym(to).is_none(),
                "synonym target `{to}` must not itself be a synonym source"
            );
        }
    }

    #[tokio::test]
    async fn canonical_name_alias() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.register_alias("e", "echo").await;

        assert_eq!(reg.canonical_name("e").await, "echo");
        assert_eq!(reg.canonical_name("echo").await, "echo"); // non-alias returns self
    }

    // --- unregister ---

    #[tokio::test]
    async fn unregister_makes_tool_uncallable() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        assert!(reg.get("echo").await.is_some());

        let removed = reg.unregister("echo").await;
        assert_eq!(removed, 1);
        assert!(reg.get("echo").await.is_none());

        // A call to the now-deleted tool must resolve to nothing (was: still callable).
        let call = ToolCall {
            id: "x".into(),
            name: "echo".into(),
            parameters: HashMap::new(),
        };
        let resp = reg.execute(call).await;
        assert!(!resp.result.success);
    }

    #[tokio::test]
    async fn unregister_cascades_to_aliases() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.register_alias("e", "echo").await;
        reg.register_alias("Echo2", "echo").await;
        assert!(reg.get("e").await.is_some());

        // Deleting the canonical tool removes both aliases too.
        let removed = reg.unregister("echo").await;
        assert_eq!(removed, 3);
        assert!(reg.get("echo").await.is_none());
        assert!(reg.get("e").await.is_none());
        assert!(reg.get("Echo2").await.is_none());
        // The alias reverse-map entry is gone, so canonical_name falls back to self.
        assert_eq!(reg.canonical_name("e").await, "e");
    }

    #[tokio::test]
    async fn unregister_alias_leaves_canonical() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.register_alias("e", "echo").await;

        // Removing just the alias must not take down the canonical tool.
        let removed = reg.unregister("e").await;
        assert_eq!(removed, 1);
        assert!(reg.get("e").await.is_none());
        assert!(reg.get("echo").await.is_some());
    }

    #[tokio::test]
    async fn unregister_unknown_is_noop() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        assert_eq!(reg.unregister("does_not_exist").await, 0);
        assert!(reg.get("echo").await.is_some());
    }

    // --- existing test ---

    #[tokio::test]
    async fn test_registry() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool).await;

        let definitions = registry.definitions().await;
        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].name, "echo");

        let call = ToolCall {
            id: "test-1".to_string(),
            name: "echo".to_string(),
            parameters: [("text".to_string(), Value::String("hello".to_string()))]
                .into_iter()
                .collect(),
        };

        let response = registry.execute(call).await;
        assert!(response.result.success);
        assert_eq!(response.result.content, "hello");
    }

    #[tokio::test]
    async fn execute_fuzzy_resolved() {
        let registry = ToolRegistry::new();
        registry.register(EchoTool).await;

        // Use "Echo" (case-insensitive) — should still execute
        let call = ToolCall {
            id: "test-2".to_string(),
            name: "Echo".to_string(),
            parameters: [("text".to_string(), Value::String("hi".to_string()))]
                .into_iter()
                .collect(),
        };

        let response = registry.execute(call).await;
        assert!(response.result.success);
        assert_eq!(response.result.content, "hi");
    }

    #[tokio::test]
    async fn definitions_for_names_dedup_alias_and_canonical() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.register_alias("e", "echo").await;

        // Request both the alias and canonical — alias should be skipped
        let names: HashSet<String> = ["echo", "e"].iter().map(|s| s.to_string()).collect();
        let defs = reg.definitions_for_names(&names).await;
        let def_names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();

        assert_eq!(defs.len(), 1, "Should have 1 def, got: {:?}", def_names);
        assert_eq!(defs[0].name, "echo");
    }

    #[tokio::test]
    async fn definitions_for_names_alias_only() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.register_alias("e", "echo").await;

        // Request ONLY the alias (not canonical) — alias should be included
        let names: HashSet<String> = ["e"].iter().map(|s| s.to_string()).collect();
        let defs = reg.definitions_for_names(&names).await;

        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "e");
    }

    // --- result_log_preview (failure log detail) ---

    #[test]
    fn result_log_preview_uses_error_on_failure() {
        // Regression: previously the failure log rendered `content` (empty for
        // `ToolResult::error`), so the reason was invisible.
        let r = ToolResult::error("connection refused");
        assert_eq!(result_log_preview(&r), "connection refused");
    }

    #[test]
    fn result_log_preview_uses_content_on_success() {
        let r = ToolResult::success("ok output");
        assert_eq!(result_log_preview(&r), "ok output");
    }

    #[test]
    fn result_log_preview_falls_back_to_content_when_no_error() {
        let r = ToolResult {
            success: false,
            content: "partial".into(),
            error: None,
            data: None,
        };
        assert_eq!(result_log_preview(&r), "partial");
    }

    #[test]
    fn result_log_preview_truncates_on_char_boundary() {
        let r = ToolResult::error("é".repeat(300)); // multi-byte, well over 200 bytes
        let preview = result_log_preview(&r);
        assert!(preview.ends_with("..."));
        assert!(preview.len() <= 203);
    }

    // --- tool policy enforcement ---

    fn call(name: &str) -> ToolCall {
        let mut parameters = HashMap::new();
        parameters.insert("text".to_string(), Value::String("hi".to_string()));
        ToolCall {
            id: "test-call".to_string(),
            name: name.to_string(),
            parameters,
        }
    }

    #[tokio::test]
    async fn default_registry_permits_execution() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;

        let resp = reg.execute(call("echo")).await;
        assert!(resp.result.success, "unrestricted registry must execute");
    }

    #[tokio::test]
    async fn denied_tool_does_not_execute() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.set_policy(ToolPolicy::deny_only(["echo"])).await;

        let resp = reg.execute(call("echo")).await;
        assert!(!resp.result.success);
        assert!(
            resp.result
                .error
                .unwrap_or_default()
                .contains("blocked by tool policy"),
            "refusal must explain itself"
        );
    }

    #[tokio::test]
    async fn denied_tool_cannot_be_reached_through_an_alias() {
        // Regression: gating on the *requested* name would let `Echo` (a
        // capitalized alias) execute a tool denied under its canonical name.
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.register_alias("Echo", "echo").await;
        reg.set_policy(ToolPolicy::deny_only(["echo"])).await;

        let resp = reg.execute(call("Echo")).await;
        assert!(!resp.result.success, "alias must not bypass the denylist");
    }

    #[tokio::test]
    async fn denied_tool_cannot_be_reached_through_fuzzy_resolution() {
        // `resolve_tool` falls back to fuzzy matching, so a near-miss spelling
        // resolves to `echo`. The policy gate runs after resolution, so it holds.
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.set_policy(ToolPolicy::deny_only(["echo"])).await;

        let resp = reg.execute(call("ech")).await;
        assert!(
            !resp.result.success,
            "fuzzy match must not bypass the denylist"
        );
    }

    #[tokio::test]
    async fn allowlist_blocks_unlisted_tool() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.set_policy(ToolPolicy::allow_only(["read_file"])).await;

        let resp = reg.execute(call("echo")).await;
        assert!(!resp.result.success);
        assert!(
            resp.result
                .error
                .unwrap_or_default()
                .contains("not in the tool allowlist"),
            "refusal must name the allowlist"
        );
    }

    #[tokio::test]
    async fn denied_tool_is_not_advertised_in_definitions() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        assert_eq!(reg.definitions().await.len(), 1);

        reg.set_policy(ToolPolicy::deny_only(["echo"])).await;
        assert!(
            reg.definitions().await.is_empty(),
            "a denied tool must not be offered to the model"
        );
    }

    #[tokio::test]
    async fn denied_canonical_hides_its_lowercase_alias_from_definitions() {
        let reg = ToolRegistry::new();
        reg.register(EchoTool).await;
        reg.register_alias("say", "echo").await;
        reg.set_policy(ToolPolicy::deny_only(["echo"])).await;

        let defs = reg.definitions().await;
        assert!(
            defs.is_empty(),
            "denying a canonical tool must also hide aliases pointing at it, got {:?}",
            defs.iter().map(|d| &d.name).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn policy_roundtrips_through_the_registry() {
        let reg = ToolRegistry::new();
        assert!(reg.policy().await.is_unrestricted());

        reg.set_policy(ToolPolicy::deny_only(["exec"])).await;
        assert!(!reg.policy().await.permits("exec"));
    }

    // --- run-scoped session ---

    #[tokio::test]
    async fn a_run_scoped_session_wins_and_leaves_the_shared_binding_alone() {
        let reg = ToolRegistry::new();
        reg.set_session_id(Some("chat-a".to_string())).await;

        let seen =
            ToolRegistry::with_run_session("scheduled-run".to_string(), reg.session_id()).await;

        assert_eq!(
            seen.as_deref(),
            Some("scheduled-run"),
            "inside a run scope, tools must read the run's session"
        );
        assert_eq!(
            reg.session_id().await.as_deref(),
            Some("chat-a"),
            "a run scope must leave the shared binding exactly as it found it"
        );
    }

    /// With no run scope there is nothing to prefer, so the shared binding
    /// still answers — chat and sub-agent paths are unchanged by this.
    #[tokio::test]
    async fn outside_a_run_scope_the_shared_binding_still_answers() {
        let reg = ToolRegistry::new();
        assert_eq!(reg.session_id().await, None);

        reg.set_session_id(Some("chat-a".to_string())).await;
        assert_eq!(reg.session_id().await.as_deref(), Some("chat-a"));
    }

    /// Two runs in flight at once — a heartbeat and a sub-agent — over a third
    /// session that owns the shared binding. Each must read only its own, in
    /// both directions, with no ordering assumption between them.
    #[tokio::test]
    async fn overlapping_runs_each_read_their_own_session() {
        let reg = Arc::new(ToolRegistry::new());
        reg.set_session_id(Some("chat-a".to_string())).await;

        let both_started = Arc::new(tokio::sync::Barrier::new(2));

        let one = {
            let (reg, gate) = (reg.clone(), both_started.clone());
            async move {
                gate.wait().await;
                reg.session_id().await
            }
        };
        let two = {
            let (reg, gate) = (reg.clone(), both_started.clone());
            async move {
                gate.wait().await;
                reg.session_id().await
            }
        };

        let (one_seen, two_seen) = tokio::join!(
            ToolRegistry::with_run_session("scheduled-heartbeat".to_string(), one),
            ToolRegistry::with_run_session("sub-agent".to_string(), two),
        );

        assert_eq!(one_seen.as_deref(), Some("scheduled-heartbeat"));
        assert_eq!(two_seen.as_deref(), Some("sub-agent"));
        assert_eq!(reg.session_id().await.as_deref(), Some("chat-a"));
    }

    /// The per-session workdir is keyed on the same answer, so a run must not
    /// pick up the working directory of whichever chat owns the shared binding.
    #[tokio::test]
    async fn a_run_scope_does_not_inherit_the_bound_chats_workdir() {
        let reg = ToolRegistry::new();
        reg.set_session_id(Some("chat-a".to_string())).await;
        reg.set_session_workdir("chat-a", std::path::PathBuf::from("/chat-a"))
            .await;
        *reg.default_workdir.write().await = Some(std::path::PathBuf::from("/default"));

        assert_eq!(
            reg.default_workdir().await,
            Some(std::path::PathBuf::from("/chat-a"))
        );

        let seen =
            ToolRegistry::with_run_session("scheduled-run".to_string(), reg.default_workdir()).await;
        assert_eq!(
            seen,
            Some(std::path::PathBuf::from("/default")),
            "a scheduled run has no workdir of its own and must fall back to the \
             global default, not borrow an unrelated chat's"
        );
    }

    /// "This session has no workspace" is a fact about the session, not an
    /// absence of one. If it falls through to the global default, the session
    /// writes into whatever directory another turn's workspace activation left
    /// there — the original destruction, reached by a different door.
    #[tokio::test]
    async fn a_bound_session_with_no_root_does_not_borrow_the_global_default() {
        let reg = ToolRegistry::new();
        *reg.default_workdir.write().await = Some(std::path::PathBuf::from("/other"));
        reg.bind_session_workdir("chat-a", None).await;

        let seen =
            ToolRegistry::with_run_session("chat-a".to_string(), reg.default_workdir()).await;
        assert_eq!(
            seen, None,
            "a session bound to NO root must resolve to nothing, not to the \
             global default another session owns"
        );
        assert_eq!(
            reg.default_workdir.read().await.clone(),
            Some(std::path::PathBuf::from("/other")),
            "binding a session must not touch the global default"
        );
    }

    /// How an incoming turn files the root it just resolved. This one choice
    /// is the whole bug and the whole fix, so the interleaving below is run
    /// under both.
    #[derive(Clone, Copy)]
    enum Filing {
        /// Pre-fix: ask the registry which session it is "in" — the SHARED
        /// slot — and key the insert on that, then write the process-wide
        /// default too. Spelled out here rather than reached through
        /// `set_default_workdir` so the counter-example keeps reproducing the
        /// old ordering even if that call later changes.
        OnTheSharedSlot,
        /// Now: key the insert on the session the turn actually IS.
        OnItsOwnSession,
    }

    /// Two chat turns overlapping over different projects. `chat-a` is
    /// mid-stream: it owns the shared slot (it is the session the user is in)
    /// and has already bound `/project-a`. `chat-b` arrives, resolves its
    /// workspace, files `/project-b`, and only then does `chat-a` resolve its
    /// next relative path — the interleaving that overwrote a 3,718-line file.
    ///
    /// Returns what each turn resolves against, streaming turn first.
    async fn overlapping_turns(
        filing: Filing,
    ) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
        let reg = Arc::new(ToolRegistry::new());
        reg.set_session_id(Some("chat-a".to_string())).await;
        reg.bind_session_workdir("chat-a", Some(std::path::PathBuf::from("/project-a")))
            .await;

        let filed = Arc::new(tokio::sync::Barrier::new(2));

        let incoming = {
            let (reg, filed) = (reg.clone(), filed.clone());
            async move {
                match filing {
                    Filing::OnTheSharedSlot => {
                        let key = reg
                            .session_id
                            .read()
                            .await
                            .clone()
                            .expect("the outgoing session owns the shared slot");
                        reg.session_workdirs
                            .write()
                            .await
                            .insert(key, Some(std::path::PathBuf::from("/project-b")));
                        *reg.default_workdir.write().await =
                            Some(std::path::PathBuf::from("/project-b"));
                    }
                    Filing::OnItsOwnSession => {
                        reg.bind_session_workdir(
                            "chat-b",
                            Some(std::path::PathBuf::from("/project-b")),
                        )
                        .await;
                    }
                }
                filed.wait().await;
                reg.default_workdir().await
            }
        };

        let streaming = {
            let (reg, filed) = (reg.clone(), filed.clone());
            async move {
                filed.wait().await;
                reg.default_workdir().await
            }
        };

        tokio::join!(
            ToolRegistry::with_run_session("chat-a".to_string(), streaming),
            ToolRegistry::with_run_session("chat-b".to_string(), incoming),
        )
    }

    /// The regression, and the proof that the assertion discriminates: the
    /// same interleaving under the old filing must still re-root the
    /// streaming turn, and under the new one must not.
    #[tokio::test]
    async fn an_incoming_turn_cannot_file_its_root_under_the_running_turns_key() {
        let (streaming, incoming) = overlapping_turns(Filing::OnTheSharedSlot).await;
        assert_eq!(
            streaming,
            Some(std::path::PathBuf::from("/project-b")),
            "counter-example: keying the newcomer's root on the shared slot \
             must still re-root the streaming turn into the other project — \
             if this stops holding, the check below proves nothing"
        );
        assert_eq!(
            incoming,
            Some(std::path::PathBuf::from("/project-b")),
            "counter-example: and the newcomer reached its own project only \
             through the process-wide default, which is why the damage went \
             unnoticed — both turns 'worked', one of them in the wrong tree"
        );

        let (streaming, incoming) = overlapping_turns(Filing::OnItsOwnSession).await;
        assert_eq!(
            streaming,
            Some(std::path::PathBuf::from("/project-a")),
            "a turn already running must keep its own root when a newcomer \
             files one, whoever owns the shared slot"
        );
        assert_eq!(
            incoming,
            Some(std::path::PathBuf::from("/project-b")),
            "and the newcomer must reach its own project by its own binding, \
             not by a default that happens to agree"
        );
    }

    /// The registry-side invariant that keeps the counter-example above
    /// unreachable. `set_default_workdir` writes the process-wide default and
    /// nothing else, but it is still the wrong call to make from inside a run:
    /// the default belongs to whoever activated a workspace last, and a run
    /// that moved it would change the root of every session with no binding of
    /// its own. So the separation is asserted rather than commented — a run
    /// that reaches for this call dies here and in the debug daemon.
    #[cfg(debug_assertions)]
    #[tokio::test]
    #[should_panic(expected = "control-plane only")]
    async fn a_run_may_not_reach_for_the_control_plane_workdir_call() {
        let reg = ToolRegistry::new();
        reg.set_session_id(Some("chat-a".to_string())).await;

        ToolRegistry::with_run_session(
            "chat-b".to_string(),
            reg.set_default_workdir(Some(std::path::PathBuf::from("/project-b"))),
        )
        .await;
    }

    /// The production path, not a re-spelling of it. The counter-example above
    /// open-codes the old filing so it keeps reproducing even if the real call
    /// changes; this one drives `set_default_workdir` ITSELF, because the way
    /// this reached a user was a workspace activation over IPC — a genuine
    /// control-plane call, carrying no run scope, made while a turn was
    /// streaming. The shared slot still names that streaming chat, so the
    /// convenience insert that used to live in this function rewrote its root
    /// and its next tool call resolved into the newly-activated project.
    #[tokio::test]
    async fn activating_a_workspace_cannot_re_root_a_streaming_turn() {
        let reg = ToolRegistry::new();

        // A turn is live: it bound its own root and owns the shared slot,
        // exactly as `prepare_chat_turn` leaves things.
        reg.set_session_id(Some("streaming-chat".to_string())).await;
        reg.bind_session_workdir("streaming-chat", Some(std::path::PathBuf::from("/project-a")))
            .await;

        // The user clicks another workspace in the sidebar mid-turn.
        reg.set_default_workdir(Some(std::path::PathBuf::from("/project-b")))
            .await;

        let streaming = ToolRegistry::with_run_session(
            "streaming-chat".to_string(),
            reg.default_workdir(),
        )
        .await;
        assert_eq!(
            streaming,
            Some(std::path::PathBuf::from("/project-a")),
            "activating a workspace must not move a running turn's root — this \
             is the interleaving that wrote one session's files into another \
             project's checkout"
        );

        // The activation still did its job for everyone without a binding.
        // Read it as such a session would — inside its own run scope — because
        // reading it bare would fall through to the shared slot, which still
        // names the streaming chat and would answer with ITS root.
        let unbound = ToolRegistry::with_run_session(
            "chat-with-no-binding".to_string(),
            reg.default_workdir(),
        )
        .await;
        assert_eq!(
            unbound,
            Some(std::path::PathBuf::from("/project-b")),
            "and the new workspace must still become the default that a \
             session with no binding of its own follows"
        );
    }

    /// The same key choice at the other end of a turn. `chat-b` finishes and
    /// tears its binding down while `chat-a` is still streaming: teardown is
    /// keyed on the session that ENDED, so the running turn keeps its root.
    /// Had it been keyed on the shared slot it would drop `chat-a`'s instead,
    /// and the streaming turn would fall through to whatever workspace
    /// activation last left in the global default — the same destruction, one
    /// door further along. Only the session that ended returns to the default,
    /// and it must return all the way: a stale entry left behind would now win
    /// over the default it should be following.
    #[tokio::test]
    async fn a_turn_ending_clears_only_its_own_binding() {
        let reg = ToolRegistry::new();
        *reg.default_workdir.write().await =
            Some(std::path::PathBuf::from("/last-activated-workspace"));
        reg.set_session_id(Some("chat-a".to_string())).await;
        reg.bind_session_workdir("chat-a", Some(std::path::PathBuf::from("/project-a")))
            .await;
        reg.bind_session_workdir("chat-b", Some(std::path::PathBuf::from("/project-b")))
            .await;

        ToolRegistry::with_run_session("chat-b".to_string(), async {
            reg.clear_session_workdir("chat-b").await;
        })
        .await;

        let streaming =
            ToolRegistry::with_run_session("chat-a".to_string(), reg.default_workdir()).await;
        assert_eq!(
            streaming,
            Some(std::path::PathBuf::from("/project-a")),
            "a turn ending must not hand a still-running turn back to the \
             global default"
        );

        let ended =
            ToolRegistry::with_run_session("chat-b".to_string(), reg.default_workdir()).await;
        assert_eq!(
            ended,
            Some(std::path::PathBuf::from("/last-activated-workspace")),
            "teardown must remove the binding entirely, not leave a stale entry \
             that now wins over the default"
        );
    }

    // --- backstop timer (the registry is the outer net, not the authority) ---

    /// A tool that outlives its declared ceiling and then answers for itself,
    /// the way a shell command running under a longer command deadline does.
    #[cfg(feature = "scripting")]
    struct SlowTool;

    #[cfg(feature = "scripting")]
    #[async_trait::async_trait]
    impl Tool for SlowTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "slow".to_string(),
                description: "Runs past its declared ceiling".to_string(),
                parameters: vec![],
                output_schema: None,
            }
        }

        async fn execute(
            &self,
            _params: HashMap<String, Value>,
        ) -> Result<ToolResult, crate::ToolError> {
            tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
            Ok(ToolResult::success("answer from the tool itself"))
        }

        fn timeout_secs(&self) -> Option<u64> {
            Some(1)
        }
    }

    /// The bug this guards: a caller asking for a longer command deadline was
    /// killed at the static manifest ceiling, and the account the tool had
    /// built of what happened arrived a second later, to nobody.
    #[cfg(feature = "scripting")]
    #[tokio::test]
    async fn backstop_does_not_preempt_a_call_that_asked_for_longer() {
        let reg = ToolRegistry::new();
        reg.register(SlowTool).await;

        let response = reg
            .execute(ToolCall {
                id: "slow-1".to_string(),
                name: "slow".to_string(),
                parameters: [("timeout".to_string(), Value::from(3))]
                    .into_iter()
                    .collect(),
            })
            .await;

        assert!(
            response.result.success,
            "the tool answered within the deadline it was given, so the \
             backstop must not have fired first: {:?}",
            response.result
        );
        assert_eq!(response.result.content, "answer from the tool itself");
    }

    /// The derivation itself: the backstop must sit strictly above the deadline
    /// the script engine will enforce, both when the call requests a longer
    /// command deadline and when it requests nothing at all.
    #[cfg(feature = "scripting")]
    #[test]
    fn backstop_outlives_every_inner_deadline() {
        let requested: HashMap<String, Value> = [("timeout".to_string(), Value::from(600))]
            .into_iter()
            .collect();
        assert!(
            backstop_timeout(180, &requested) > std::time::Duration::from_secs(600),
            "a 600s command deadline must not be cut short by a 180s ceiling"
        );
        assert!(
            backstop_timeout(180, &HashMap::new()) > std::time::Duration::from_secs(180),
            "even with nothing requested the backstop must outlive the \
             ceiling, or the two race and the outer one wins"
        );
    }

    /// When the backstop really is the last resort it has no tool report to
    /// relay, so it must say how long it waited and warn that the abandoned
    /// tool may already have written part of its work.
    #[test]
    fn backstop_message_states_elapsed_time_and_side_effects() {
        let msg = backstop_message("exec", 180_004, 190_000);
        assert!(msg.contains("exec"));
        assert!(msg.contains("180004ms"), "must state elapsed wall time: {msg}");
        assert!(msg.contains("190000ms"), "must state the limit that fired: {msg}");
        assert!(msg.contains("on disk"), "must warn about side effects: {msg}");
    }
}
