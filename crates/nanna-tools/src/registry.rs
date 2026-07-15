//! Tool registry for managing available tools

use crate::{Tool, ToolCall, ToolDefinition, ToolResponse, ToolResult, OutputTarget};
use crate::skills::{load_skill, discover_skills};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

#[cfg(feature = "scripting")]
use crate::skills::load_skill_with_services;

/// Registry of available tools
pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    /// Alias names (lowercase aliases ARE included in definitions; capitalized ones are not)
    aliases: RwLock<HashSet<String>>,
    /// Reverse map: alias name → canonical target name
    alias_targets: RwLock<HashMap<String, String>>,
    /// Default working directory for tool execution (global fallback)
    default_workdir: RwLock<Option<std::path::PathBuf>>,
    /// Per-session working directories (session_id → workdir).
    /// Takes precedence over default_workdir when a session is active.
    session_workdirs: RwLock<HashMap<String, std::path::PathBuf>>,
    /// Current session ID (set when agent session starts)
    session_id: RwLock<Option<String>>,
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
        }
    }

    /// Set the default working directory for tool execution.
    /// Called when the active workspace changes.
    pub async fn set_default_workdir(&self, workdir: Option<std::path::PathBuf>) {
        // Also set for the current session if one is active
        if let Some(ref sid) = *self.session_id.read().await {
            if let Some(ref wd) = workdir {
                self.session_workdirs.write().await.insert(sid.clone(), wd.clone());
            }
        }
        *self.default_workdir.write().await = workdir;
    }

    /// Get the current default working directory.
    /// Returns the per-session workdir if available, otherwise the global default.
    pub async fn default_workdir(&self) -> Option<std::path::PathBuf> {
        // Per-session workdir takes priority
        if let Some(ref sid) = *self.session_id.read().await {
            if let Some(wd) = self.session_workdirs.read().await.get(sid) {
                return Some(wd.clone());
            }
        }
        self.default_workdir.read().await.clone()
    }

    /// Set the working directory for a specific session.
    pub async fn set_session_workdir(&self, session_id: &str, workdir: std::path::PathBuf) {
        self.session_workdirs.write().await.insert(session_id.to_string(), workdir);
    }

    /// Remove a session's workdir (call on session cleanup).
    pub async fn clear_session_workdir(&self, session_id: &str) {
        self.session_workdirs.write().await.remove(session_id);
    }

    /// Set the current session ID.
    /// Called when an agent session starts or changes.
    pub async fn set_session_id(&self, session_id: Option<String>) {
        *self.session_id.write().await = session_id;
    }

    /// Get the current session ID.
    pub async fn session_id(&self) -> Option<String> {
        self.session_id.read().await.clone()
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
            warn!("Cannot create alias '{}': target tool '{}' not found", alias, target);
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
        targets.get(name).cloned().unwrap_or_else(|| name.to_string())
    }

    /// Multi-step tool resolution: exact → case-insensitive → fuzzy.
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
                info!(requested = name, resolved = key, step = "case-insensitive", "Tool resolved");
                return Some((key.clone(), tool.clone()));
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
        tools
            .iter()
            .filter(|(name, _)| {
                // Include non-aliases AND lowercase-only aliases
                !aliases.contains(name.as_str()) || name.chars().all(|c| !c.is_uppercase())
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
        tools
            .iter()
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

    /// Get tool definitions in Anthropic format.
    /// Includes lowercase aliases so the LLM sees correct parameter schemas.
    pub async fn to_anthropic_format(&self) -> Vec<Value> {
        self.definitions().await
            .into_iter()
            .map(|d| d.to_anthropic_format())
            .collect()
    }

    /// Get tool definitions in `OpenAI` format.
    /// Includes lowercase aliases so the LLM sees correct parameter schemas.
    pub async fn to_openai_format(&self) -> Vec<Value> {
        self.definitions().await
            .into_iter()
            .map(|d| d.to_openai_format())
            .collect()
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
        debug!("Executing tool: {} (id: {}) with params: {}", call.name, call.id, params_preview);

        let (resolved_name, tool) = match self.resolve_tool(&call.name).await {
            Some(pair) => pair,
            None => {
                warn!("Tool not found: {}", call.name);
                return ToolResponse {
                    id: call.id,
                    name: call.name.clone(),
                    result: ToolResult::error(format!("Tool not found: {}. Use discover_tools to see available tools.", call.name)),
                    output_target: OutputTarget::default(),
                };
            }
        };

        if resolved_name != call.name {
            debug!("Tool '{}' resolved to '{}'", call.name, resolved_name);
        }

        let output_target = tool.output_target();
        let start = std::time::Instant::now();

        // Normalize camelCase parameter keys to snake_case.
        // Weaker models (OpenRouter free, small Ollama) often send "filePath"
        // instead of "file_path", etc. We add snake_case aliases without
        // removing the originals so either convention works.
        let parameters = normalize_param_keys(call.parameters.clone());

        // Execute with timeout if specified
        let result = if let Some(timeout_secs) = tool.timeout_secs() {
            match tokio::time::timeout(
                std::time::Duration::from_secs(timeout_secs),
                tool.execute(parameters.clone()),
            )
            .await
            {
                Ok(Ok(result)) => result,
                Ok(Err(e)) => {
                    warn!("Tool {} execution error: {}", call.name, e);
                    ToolResult::error(e.to_string())
                }
                Err(_) => {
                    warn!("Tool {} timed out after {}s", call.name, timeout_secs);
                    ToolResult::error("Tool execution timed out")
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
        
        // Log result summary
        let output_preview = if result.content.len() > 200 {
            let end = truncate_boundary(&result.content, 200);
            format!("{}...", &result.content[..end])
        } else {
            result.content.clone()
        };
        
        if result.success {
            debug!("Tool {} completed in {}ms: {}", call.name, duration_ms, output_preview);
        } else {
            warn!("Tool {} failed in {}ms: {}", call.name, duration_ms, output_preview);
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
    #[cfg(feature = "scripting")]
    pub async fn load_skills_with_services(
        &self,
        skills_dir: &Path,
        services: &HashMap<String, nanna_scripting::ServiceFn>,
    ) -> usize {
        let discovered = discover_skills(skills_dir);
        let total = discovered.len();
        let mut loaded = 0;

        for skill in discovered {
            match load_skill_with_services(&skill.path, services).await {
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
        tools.keys()
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
            let val = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(prev + cost);
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
fn normalize_param_keys(mut params: HashMap<String, serde_json::Value>) -> HashMap<String, serde_json::Value> {
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
}
