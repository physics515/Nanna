//! System handlers for the [`ControlPlane`].

use super::*;

impl ControlPlane {
    // =========================================================================
    // System Handlers
    // =========================================================================
    
    pub(super) async fn handle_system(&self, _client_id: &str, action: SystemAction) -> Value {
        match action {
            SystemAction::Status => {
                let memory_stats = if let Some(ref memory) = self.memory {
                    let stats = memory.stats().await;
                    Some(json!({
                        "total": stats.total,
                        "active": stats.active,
                    }))
                } else {
                    None
                };
                
                let tool_count = if let Some(ref tools) = self.tools {
                    Some(tools.definitions().await.len())
                } else {
                    None
                };
                
                let workspace_count = self.workspaces.read().await.len();
                let scheduler_available = self.scheduler.is_some();

                // The router's live provider set. This is what actually routes
                // a chat request — the GUI model picker gates on this instead
                // of its own login state, so the two can't split-brain about
                // which providers exist.
                let llm_providers: Vec<&'static str> = self
                    .router
                    .as_ref()
                    .map(|r| {
                        r.available_providers_sorted()
                            .into_iter()
                            .map(crate::llm_router::ProviderId::name)
                            .collect()
                    })
                    .unwrap_or_default();

                // Durable-store health: a corrupt row skipped on load leaves the
                // store degraded — surface it rather than a silent empty store.
                let (memory_degraded, memory_corrupt_rows) = if let Some(ref m) = self.memory {
                    let h = m.store_health().await;
                    (h.degraded, h.corrupt_rows)
                } else {
                    (false, 0)
                };

                // Startup quarantine + rebuild (page-level corruption): keep it
                // on /status for the daemon's lifetime — clients that connect
                // after boot never saw the MemoryStoreRebuilt event.
                let (memory_rebuilt, memory_recovered_rows, memory_expected_rows) = self
                    .memory_recovery
                    .as_ref()
                    .map_or((false, 0, None), |r| {
                        (true, r.memories_recovered, r.memories_expected)
                    });

                json!({
                    "status": "running",
                    "version": env!("CARGO_PKG_VERSION"),
                    "uptime_secs": self.uptime_secs(),
                    "sessions": self.sessions.count().await,
                    "workspaces": workspace_count,
                    "agent_available": self.agent.is_some(),
                    "memory_available": self.memory.is_some(),
                    "memory_degraded": memory_degraded,
                    "memory_corrupt_rows": memory_corrupt_rows,
                    "memory_rebuilt": memory_rebuilt,
                    "memory_recovered_rows": memory_recovered_rows,
                    "memory_expected_rows": memory_expected_rows,
                    "memory_stats": memory_stats,
                    "tools_available": self.tools.is_some(),
                    "tool_count": tool_count,
                    "scheduler_available": scheduler_available,
                    "llm_providers": llm_providers,
                    "config_path": self.config_path,
                })
            }
            SystemAction::Restart => {
                info!("Restart requested");
                json!({ "status": "restarting" })
            }
            SystemAction::Shutdown => {
                info!("Shutdown requested");
                if let Some(tx) = self.shutdown_tx.clone() {
                    // Fire after a short grace so the response below flushes to
                    // the requesting client before the IPC server tears down.
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                        let _ = tx.send(());
                    });
                    json!({ "status": "shutting_down" })
                } else {
                    // Minimal construction with no shutdown handle: say so
                    // instead of pretending — callers fall back to a tree-kill.
                    json!({ "status": "unsupported", "error": "no shutdown handle wired" })
                }
            }
            SystemAction::Version => {
                json!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "name": "nanna-daemon",
                    "rust_version": env!("CARGO_PKG_RUST_VERSION"),
                })
            }
            SystemAction::CheckUpdate => {
                json!({ "update_available": false })
            }
            SystemAction::Update => {
                json!({ "error": "not_implemented" })
            }
            SystemAction::Logs { lines, level } => {
                if let Some(ref buf) = self.log_buffer {
                    let entries = buf.get_recent(lines.unwrap_or(1000));
                    // Filter by level if specified
                    let filtered: Vec<_> = if let Some(ref lvl) = level {
                        let lvl = lvl.to_lowercase();
                        entries.into_iter().filter(|e| e.level == lvl).collect()
                    } else {
                        entries
                    };
                    json!({ "logs": filtered })
                } else {
                    json!({ "logs": [], "message": "Log buffer not available" })
                }
            }
            SystemAction::Health => {
                let memory_ok = self.memory.is_some();
                let tools_ok = self.tools.is_some();
                let agent_ok = self.agent.is_some();
                let scheduler_ok = self.scheduler.is_some();
                let all_ok = agent_ok; // Agent is the critical service
                
                json!({
                    "healthy": all_ok,
                    "checks": {
                        "sessions": "ok",
                        "agent": if agent_ok { "ok" } else { "unavailable" },
                        "memory": if memory_ok { "ok" } else { "unavailable" },
                        "tools": if tools_ok { "ok" } else { "unavailable" },
                        "scheduler": if scheduler_ok { "ok" } else { "unavailable" },
                        "config": "ok",
                        "workspaces": "ok",
                    }
                })
            }
            SystemAction::ModelStats => {
                let summaries = self.model_stats.summaries().await;
                // Estimated USD spend per model (reference list prices); local
                // models come back flagged unpriced rather than as $0.
                let costs = self.model_stats.cost_report().await;
                let total_cost_usd = self.model_stats.total_cost_usd().await;
                json!({
                    "models": summaries,
                    "costs": costs,
                    "total_cost_usd": total_cost_usd,
                })
            }
            SystemAction::ToolStats => {
                let summaries = self.tool_stats.summaries().await;
                json!({
                    "tools": summaries,
                })
            }
            SystemAction::GlobalStats => {
                let global = self.tool_stats.global_stats().await;
                json!(global)
            }
            SystemAction::ToolStatsHourly { tool_name, hours } => {
                if let Some(ref storage) = self.storage {
                    match storage.get_tool_stats_hourly(tool_name.as_deref(), hours.unwrap_or(24)).await {
                        Ok(data) => json!({ "buckets": data }),
                        Err(e) => json!({ "error": e.to_string() }),
                    }
                } else {
                    json!({ "buckets": [], "error": "Storage not available" })
                }
            }
            SystemAction::ToolStatsDaily { tool_name, days } => {
                if let Some(ref storage) = self.storage {
                    match storage.get_tool_stats_daily(tool_name.as_deref(), days.unwrap_or(30)).await {
                        Ok(data) => json!({ "buckets": data }),
                        Err(e) => json!({ "error": e.to_string() }),
                    }
                } else {
                    json!({ "buckets": [], "error": "Storage not available" })
                }
            }
            SystemAction::ToolCallLog { tool_name, limit } => {
                if let Some(ref storage) = self.storage {
                    match storage.get_tool_call_log(tool_name.as_deref(), limit.unwrap_or(50)).await {
                        Ok(entries) => json!({ "entries": entries }),
                        Err(e) => json!({ "error": e.to_string() }),
                    }
                } else {
                    json!({ "entries": [], "error": "Storage not available" })
                }
            }
        }
    }
}
