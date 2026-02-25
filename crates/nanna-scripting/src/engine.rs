//! Script engine abstraction with automatic fallback

use crate::{Result, ScriptError, ScriptedTool, NannaBridge, bridge::ServiceFn};
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
        let mut bridge = NannaBridge::new(tool.permissions.clone());
        if let Some(defs) = tool_definitions {
            bridge = bridge.with_tool_definitions(defs);
        }
        if let Some(svcs) = services {
            bridge = bridge.with_services(svcs);
        }
        let bridge = Arc::new(bridge);
        let start = std::time::Instant::now();

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
}
