//! Script engine abstraction with automatic fallback

use crate::{Result, ScriptError, ScriptedTool, NannaBridge, bridge::{ServiceFn, ToolSearchFn}};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Which engine executed the script
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineKind {
    Boa,
    Deno,
}

impl std::fmt::Display for EngineKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Boa => write!(f, "Boa"),
            Self::Deno => write!(f, "Deno"),
        }
    }
}

/// Result of script execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// The return value
    pub value: Value,
    /// Which engine was used
    pub engine: EngineKind,
    /// Execution time in milliseconds
    pub duration_ms: u64,
    /// Whether fallback was used
    pub used_fallback: bool,
    /// Error from primary engine (if fallback was used)
    pub primary_error: Option<String>,
}

/// Unified script engine with automatic fallback
pub struct ScriptEngine {
    /// Preferred engine order
    prefer_boa: bool,
    /// Whether to enable fallback
    enable_fallback: bool,
}

impl ScriptEngine {
    /// Create a new script engine (Boa preferred, Deno fallback)
    #[must_use]
    pub fn new() -> Self {
        Self {
            prefer_boa: true,
            enable_fallback: cfg!(all(feature = "boa", feature = "deno")),
        }
    }

    /// Prefer Deno over Boa
    #[must_use]
    pub const fn prefer_deno(mut self) -> Self {
        self.prefer_boa = false;
        self
    }

    /// Disable automatic fallback
    #[must_use]
    pub const fn no_fallback(mut self) -> Self {
        self.enable_fallback = false;
        self
    }

    /// Execute a scripted tool
    ///
    /// `tool_definitions` is an optional JSON array of tool definitions for `Nanna.listTools()`.
    /// `services` is an optional map of service functions callable via `Nanna.service()`.
    pub async fn execute(
        &self,
        tool: &ScriptedTool,
        input: Value,
        tool_definitions: Option<Value>,
        services: Option<HashMap<String, ServiceFn>>,
    ) -> Result<ExecutionResult> {
        self.execute_with_workdir(tool, input, tool_definitions, services, None).await
    }

    /// Execute a scripted tool with an optional default working directory.
    pub async fn execute_with_workdir(
        &self,
        tool: &ScriptedTool,
        input: Value,
        tool_definitions: Option<Value>,
        services: Option<HashMap<String, ServiceFn>>,
        default_workdir: Option<std::path::PathBuf>,
    ) -> Result<ExecutionResult> {
        self.execute_with_workdir_and_session(tool, input, tool_definitions, services, default_workdir, None).await
    }

    /// Execute a scripted tool with optional working directory and session ID.
    pub async fn execute_with_workdir_and_session(
        &self,
        tool: &ScriptedTool,
        input: Value,
        tool_definitions: Option<Value>,
        services: Option<HashMap<String, ServiceFn>>,
        default_workdir: Option<std::path::PathBuf>,
        session_id: Option<String>,
    ) -> Result<ExecutionResult> {
        self.execute_full(tool, input, tool_definitions, services, default_workdir, session_id, None).await
    }

    /// Execute a scripted tool with every optional bridge capability,
    /// including the ranked tool search behind `Nanna.searchTools()`.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_full(
        &self,
        tool: &ScriptedTool,
        input: Value,
        tool_definitions: Option<Value>,
        services: Option<HashMap<String, ServiceFn>>,
        default_workdir: Option<std::path::PathBuf>,
        session_id: Option<String>,
        tool_search: Option<ToolSearchFn>,
    ) -> Result<ExecutionResult> {
        let mut bridge = NannaBridge::new(tool.permissions.clone());
        if let Some(defs) = tool_definitions {
            bridge = bridge.with_tool_definitions(defs);
        }
        if let Some(search) = tool_search {
            bridge = bridge.with_tool_search(search);
        }
        if let Some(svcs) = services {
            bridge = bridge.with_services(svcs);
        }
        if let Some(wd) = default_workdir {
            bridge = bridge.with_default_workdir(wd);
        }
        if let Some(sid) = session_id {
            bridge = bridge.with_session_id(sid);
        }
        let bridge = Arc::new(bridge);
        let start = std::time::Instant::now();

        // The script-engine deadline wraps the *whole* script, including any shell
        // command it runs via the bridge. A tool that shells out (e.g. `exec`)
        // forwards an integer `timeout` (seconds) input to the bridge, which owns
        // the *command* deadline and can kill the child. If the engine deadline
        // were shorter than that command deadline it would preempt a legitimately
        // long command — and worse, orphan the child the bridge would otherwise
        // reap. So extend the engine deadline to cover the requested command
        // deadline. We only ever *extend*, never shorten, so tools without a
        // meaningful `timeout` input keep their configured deadline.
        let effective_timeout_ms = effective_timeout_ms(tool.timeout_ms, &input);
        let tool_owned;
        let tool: &ScriptedTool = if effective_timeout_ms == tool.timeout_ms {
            tool
        } else {
            tool_owned = tool.clone().with_timeout(effective_timeout_ms);
            &tool_owned
        };

        // Check if script needs an advanced engine (async/await, TypeScript, etc.)
        let needs_advanced = needs_advanced_engine(&tool.source);

        // Determine engine order
        let (primary, secondary): (EngineKind, Option<EngineKind>) = if needs_advanced {
            // Skip Boa for scripts that use features it can't handle
            debug!(tool = %tool.name, "Script needs advanced engine, preferring Deno");
            #[cfg(feature = "deno")]
            let p = EngineKind::Deno;
            #[cfg(not(feature = "deno"))]
            let p = EngineKind::Boa; // Try Boa anyway if Deno unavailable

            #[cfg(all(feature = "deno", feature = "boa"))]
            let s = Some(EngineKind::Boa);
            #[cfg(not(all(feature = "deno", feature = "boa")))]
            let s = None;

            (p, s)
        } else if self.prefer_boa {
            #[cfg(feature = "boa")]
            let p = EngineKind::Boa;
            #[cfg(not(feature = "boa"))]
            let p = EngineKind::Deno;

            #[cfg(all(feature = "deno", feature = "boa"))]
            let s = Some(EngineKind::Deno);
            #[cfg(not(all(feature = "deno", feature = "boa")))]
            let s = None;

            (p, s)
        } else {
            #[cfg(feature = "deno")]
            let p = EngineKind::Deno;
            #[cfg(not(feature = "deno"))]
            let p = EngineKind::Boa;

            #[cfg(all(feature = "deno", feature = "boa"))]
            let s = Some(EngineKind::Boa);
            #[cfg(not(all(feature = "deno", feature = "boa")))]
            let s = None;

            (p, s)
        };

        debug!(tool = %tool.name, engine = %primary, "Executing script");

        // Try primary engine
        let primary_result = self.execute_with_engine(tool, &input, &bridge, primary).await;

        match primary_result {
            Ok(value) => {
                let duration_ms = start.elapsed().as_millis() as u64;
                info!(tool = %tool.name, engine = %primary, duration_ms, "Script executed successfully");
                
                Ok(ExecutionResult {
                    value,
                    engine: primary,
                    duration_ms,
                    used_fallback: false,
                    primary_error: None,
                })
            }
            Err(primary_err) => {
                // Try fallback if enabled
                if self.enable_fallback {
                    if let Some(fallback) = secondary {
                        warn!(
                            tool = %tool.name,
                            primary = %primary,
                            error = %primary_err,
                            fallback = %fallback,
                            "Primary engine failed, trying fallback"
                        );

                        match self.execute_with_engine(tool, &input, &bridge, fallback).await {
                            Ok(value) => {
                                let duration_ms = start.elapsed().as_millis() as u64;
                                info!(
                                    tool = %tool.name,
                                    engine = %fallback,
                                    duration_ms,
                                    "Fallback engine succeeded"
                                );

                                return Ok(ExecutionResult {
                                    value,
                                    engine: fallback,
                                    duration_ms,
                                    used_fallback: true,
                                    primary_error: Some(primary_err.to_string()),
                                });
                            }
                            Err(fallback_err) => {
                                // Both failed, return combined error
                                return Err(ScriptError::Execution(format!(
                                    "{primary} failed: {primary_err}; {fallback} failed: {fallback_err}"
                                )));
                            }
                        }
                    }
                }

                Err(primary_err)
            }
        }
    }

    /// Execute with a specific engine
    async fn execute_with_engine(
        &self,
        tool: &ScriptedTool,
        input: &Value,
        bridge: &Arc<NannaBridge>,
        engine: EngineKind,
    ) -> Result<Value> {
        match engine {
            EngineKind::Boa => {
                #[cfg(feature = "boa")]
                {
                    crate::boa_impl::execute(tool, input, bridge).await
                }
                #[cfg(not(feature = "boa"))]
                {
                    Err(ScriptError::EngineNotAvailable("Boa".to_string()))
                }
            }
            EngineKind::Deno => {
                #[cfg(feature = "deno")]
                {
                    crate::deno_impl::execute(tool, input, bridge).await
                }
                #[cfg(not(feature = "deno"))]
                {
                    Err(ScriptError::EngineNotAvailable("Deno".to_string()))
                }
            }
        }
    }

    /// Check which engines are available
    #[must_use]
    pub fn available_engines(&self) -> Vec<EngineKind> {
        let mut engines = Vec::new();
        
        #[cfg(feature = "boa")]
        engines.push(EngineKind::Boa);
        
        #[cfg(feature = "deno")]
        engines.push(EngineKind::Deno);
        
        engines
    }
}

impl Default for ScriptEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Slack between the bridge's per-command deadline and the outer engine
/// deadline, in ms. The engine deadline must OUTLIVE the command deadline so the
/// bridge (which can kill the child) always fires first; this covers the
/// script/bridge handoff overhead (TS transpile, thread + runtime spawn, result
/// marshaling), all sub-second, with margin.
const ENGINE_TIMEOUT_HANDOFF_MARGIN_MS: u64 = 10_000;

/// Compute the effective script-engine deadline for one execution.
///
/// Extends `base_ms` (the tool's configured deadline) to cover a `timeout`
/// (seconds) declared in `input` — the deadline the shell bridge will enforce
/// for a command — plus [`ENGINE_TIMEOUT_HANDOFF_MARGIN_MS`]. Only ever extends;
/// a tool with no `timeout` input (or a zero/absent one) keeps `base_ms`.
fn effective_timeout_ms(base_ms: u64, input: &Value) -> u64 {
    let requested_ms = input
        .get("timeout")
        .and_then(Value::as_u64)
        .filter(|&s| s > 0)
        .and_then(|s| s.checked_mul(1000))
        .map(|ms| ms.saturating_add(ENGINE_TIMEOUT_HANDOFF_MARGIN_MS));
    match requested_ms {
        Some(req) => base_ms.max(req),
        None => base_ms,
    }
}

/// Detect if a script uses features that Boa can't handle.
///
/// Returns `true` if the script should skip Boa and go straight to Deno.
/// Uses simple string checks (cheap, no regex needed).
fn needs_advanced_engine(source: &str) -> bool {
    source.contains("async ") || source.contains("await ")
        || source.contains(": string") || source.contains(": number") || source.contains(": boolean")
        || source.contains("interface ") || source.contains("import {")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_display() {
        assert_eq!(EngineKind::Boa.to_string(), "Boa");
        assert_eq!(EngineKind::Deno.to_string(), "Deno");
    }

    #[test]
    fn test_available_engines() {
        let engine = ScriptEngine::new();
        let available = engine.available_engines();

        #[cfg(feature = "boa")]
        assert!(available.contains(&EngineKind::Boa));

        #[cfg(feature = "deno")]
        assert!(available.contains(&EngineKind::Deno));
    }

    #[test]
    fn effective_timeout_extends_for_requested_command_timeout() {
        // A large explicit `timeout` input pushes the engine deadline past it by
        // the handoff margin, so the bridge's command deadline fires first.
        let base = 30_000;
        let input = serde_json::json!({ "command": "cargo build", "timeout": 600 });
        let eff = effective_timeout_ms(base, &input);
        assert_eq!(eff, 600_000 + ENGINE_TIMEOUT_HANDOFF_MARGIN_MS);
        assert!(eff > 600_000, "engine must outlive the 600s command deadline");
    }

    #[test]
    fn effective_timeout_never_shortens() {
        // A small requested timeout must not pull the engine deadline below the
        // tool's configured base (e.g. exec's 180s ceiling).
        let base = 180_000;
        let input = serde_json::json!({ "timeout": 5 });
        assert_eq!(effective_timeout_ms(base, &input), base);
    }

    #[test]
    fn effective_timeout_ignores_absent_or_zero() {
        let base = 30_000;
        assert_eq!(effective_timeout_ms(base, &serde_json::json!({})), base);
        assert_eq!(
            effective_timeout_ms(base, &serde_json::json!({ "timeout": 0 })),
            base
        );
        // Non-integer timeouts are ignored (no partial/garbage parse).
        assert_eq!(
            effective_timeout_ms(base, &serde_json::json!({ "timeout": "soon" })),
            base
        );
    }
}
