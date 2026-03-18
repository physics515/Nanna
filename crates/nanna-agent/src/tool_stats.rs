//! Tool performance statistics tracking.
//!
//! Tracks per-tool metrics across invocations to provide visibility
//! into tool behavior:
//! - Call count, success/failure rates
//! - Latency percentiles (P50, P95, P99)
//! - Output sizes
//! - Common errors
//! - Per-session aggregates (tool time vs LLM time)

use std::collections::HashMap;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Maximum number of latency samples to keep per tool (ring buffer).
const MAX_LATENCY_SAMPLES: usize = 200;
/// Maximum number of output-size samples to keep per tool (ring buffer).
const MAX_OUTPUT_SAMPLES: usize = 200;
/// Maximum number of distinct error messages to track per tool.
const MAX_ERROR_ENTRIES: usize = 50;

// =============================================================================
// Core tracker
// =============================================================================

/// Global tool statistics tracker. Thread-safe, designed for concurrent access.
#[derive(Debug, Clone)]
pub struct ToolStatsTracker {
    inner: Arc<RwLock<ToolStatsInner>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ToolStatsInner {
    /// Per-tool statistics keyed by tool name.
    tools: HashMap<String, ToolStats>,
    /// Per-session statistics keyed by session ID.
    sessions: HashMap<String, SessionStats>,
}

/// Accumulated statistics for a single tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStats {
    /// Tool name
    pub name: String,
    /// Total invocations
    pub call_count: u64,
    /// Successful invocations
    pub success_count: u64,
    /// Failed invocations
    pub failure_count: u64,
    /// Recent latencies in milliseconds (ring buffer, last N)
    pub latencies_ms: Vec<u64>,
    /// Recent output sizes in bytes/chars (ring buffer, last N)
    pub output_sizes: Vec<usize>,
    /// Epoch-ms timestamp of the most recent invocation
    pub last_called: Option<u64>,
    /// Common error messages with occurrence counts
    pub errors: Vec<(String, u64)>,
}

/// Per-session aggregate statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    /// Session identifier
    pub session_id: String,
    /// Number of agent-loop iterations
    pub iterations: u64,
    /// Total tool invocations in this session
    pub tool_calls: u64,
    /// Cumulative tool execution time (ms)
    pub tool_time_ms: u64,
    /// Cumulative LLM request time (ms)
    pub llm_time_ms: u64,
    /// Cumulative input tokens
    pub input_tokens: u64,
    /// Cumulative output tokens
    pub output_tokens: u64,
}

/// A single tool invocation observation to record.
#[derive(Debug)]
pub struct ToolObservation {
    pub tool_name: String,
    pub success: bool,
    pub duration_ms: u64,
    pub output_size: usize,
    /// Error message (if any)
    pub error: Option<String>,
    /// Optional session ID to attribute this call to
    pub session_id: Option<String>,
}

/// Summary stats for a single tool, suitable for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolStatsSummary {
    pub name: String,
    pub call_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub success_rate: f64,
    pub avg_latency_ms: u64,
    pub p50_latency_ms: u64,
    pub p95_latency_ms: u64,
    pub p99_latency_ms: u64,
    pub avg_output_size: usize,
    pub last_called: Option<u64>,
    pub top_errors: Vec<(String, u64)>,
}

/// Global dashboard summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalToolStats {
    /// Total tool calls across all tools
    pub total_calls: u64,
    /// Overall average latency (ms)
    pub avg_latency_ms: u64,
    /// Overall success rate
    pub success_rate: f64,
    /// Top 5 slowest tools by P95
    pub slowest_tools: Vec<ToolStatsSummary>,
    /// Top 10 most-used tools
    pub most_used_tools: Vec<ToolStatsSummary>,
    /// Top 5 most-failed tools (by error rate)
    pub most_failed_tools: Vec<ToolStatsSummary>,
    /// Aggregate session stats
    pub session_totals: SessionTotals,
}

/// Aggregated session totals across all sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionTotals {
    pub total_iterations: u64,
    pub total_tool_calls: u64,
    pub total_tool_time_ms: u64,
    pub total_llm_time_ms: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
}

// =============================================================================
// Implementation
// =============================================================================

impl ToolStatsTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(ToolStatsInner::default())),
        }
    }

    /// Record a completed tool invocation.
    pub async fn record(&self, obs: ToolObservation) {
        let mut inner = self.inner.write().await;
        let stats = inner.tools.entry(obs.tool_name.clone())
            .or_insert_with(|| ToolStats::new(&obs.tool_name));

        stats.call_count += 1;
        stats.last_called = Some(now_epoch_ms());

        if obs.success {
            stats.success_count += 1;
        } else {
            stats.failure_count += 1;
        }

        // Ring-buffer latency
        if stats.latencies_ms.len() >= MAX_LATENCY_SAMPLES {
            stats.latencies_ms.remove(0);
        }
        stats.latencies_ms.push(obs.duration_ms);

        // Ring-buffer output size
        if stats.output_sizes.len() >= MAX_OUTPUT_SAMPLES {
            stats.output_sizes.remove(0);
        }
        stats.output_sizes.push(obs.output_size);

        // Track errors
        if let Some(ref err_msg) = obs.error {
            // Truncate error to first 200 chars for dedup
            let key = if err_msg.len() > 200 {
                format!("{}...", &err_msg[..err_msg.floor_char_boundary(200)])
            } else {
                err_msg.clone()
            };
            if let Some(entry) = stats.errors.iter_mut().find(|(e, _)| e == &key) {
                entry.1 += 1;
            } else if stats.errors.len() < MAX_ERROR_ENTRIES {
                stats.errors.push((key, 1));
            }
        }

        // Attribute to session
        if let Some(ref sid) = obs.session_id {
            let session = inner.sessions.entry(sid.clone())
                .or_insert_with(|| SessionStats::new(sid));
            session.tool_calls += 1;
            session.tool_time_ms += obs.duration_ms;
        }

        if obs.duration_ms > 5_000 {
            warn!(
                tool = %obs.tool_name,
                duration_ms = obs.duration_ms,
                "⚠️ Slow tool execution recorded in stats (>5s)"
            );
        }

        debug!(
            tool = %obs.tool_name,
            success = obs.success,
            duration_ms = obs.duration_ms,
            output_size = obs.output_size,
            "📊 Tool stats recorded"
        );
    }

    /// Record LLM time for a session.
    pub async fn record_llm_time(&self, session_id: &str, llm_time_ms: u64, input_tokens: u64, output_tokens: u64) {
        let mut inner = self.inner.write().await;
        let session = inner.sessions.entry(session_id.to_string())
            .or_insert_with(|| SessionStats::new(session_id));
        session.llm_time_ms += llm_time_ms;
        session.input_tokens += input_tokens;
        session.output_tokens += output_tokens;
    }

    /// Record an iteration for a session.
    pub async fn record_iteration(&self, session_id: &str) {
        let mut inner = self.inner.write().await;
        let session = inner.sessions.entry(session_id.to_string())
            .or_insert_with(|| SessionStats::new(session_id));
        session.iterations += 1;
    }

    /// Get summary statistics for all tracked tools.
    pub async fn summaries(&self) -> Vec<ToolStatsSummary> {
        let inner = self.inner.read().await;
        inner.tools.values().map(ToolStats::summary).collect()
    }

    /// Get summary for a specific tool.
    pub async fn summary(&self, tool_name: &str) -> Option<ToolStatsSummary> {
        let inner = self.inner.read().await;
        inner.tools.get(tool_name).map(ToolStats::summary)
    }

    /// Get the global dashboard stats.
    pub async fn global_stats(&self) -> GlobalToolStats {
        let inner = self.inner.read().await;
        let mut all_summaries: Vec<ToolStatsSummary> = inner.tools.values()
            .map(ToolStats::summary)
            .collect();

        let total_calls: u64 = all_summaries.iter().map(|s| s.call_count).sum();
        let total_success: u64 = all_summaries.iter().map(|s| s.success_count).sum();

        // Overall average latency (weighted by call count)
        let weighted_latency: u64 = all_summaries.iter()
            .map(|s| s.avg_latency_ms * s.call_count)
            .sum();
        let avg_latency_ms = if total_calls > 0 { weighted_latency / total_calls } else { 0 };
        let success_rate = if total_calls > 0 { total_success as f64 / total_calls as f64 } else { 1.0 };

        // Top 5 slowest by P95
        let mut slowest = all_summaries.clone();
        slowest.sort_by(|a, b| b.p95_latency_ms.cmp(&a.p95_latency_ms));
        slowest.truncate(5);

        // Top 10 most-used
        all_summaries.sort_by(|a, b| b.call_count.cmp(&a.call_count));
        let most_used: Vec<_> = all_summaries.iter().take(10).cloned().collect();

        // Top 5 most-failed (by error rate, min 2 calls)
        let mut by_error: Vec<_> = all_summaries.iter()
            .filter(|s| s.call_count >= 2)
            .cloned()
            .collect();
        by_error.sort_by(|a, b| {
            let rate_a = if a.call_count > 0 { a.failure_count as f64 / a.call_count as f64 } else { 0.0 };
            let rate_b = if b.call_count > 0 { b.failure_count as f64 / b.call_count as f64 } else { 0.0 };
            rate_b.partial_cmp(&rate_a).unwrap_or(std::cmp::Ordering::Equal)
        });
        by_error.truncate(5);

        // Session totals
        let session_totals = SessionTotals {
            total_iterations: inner.sessions.values().map(|s| s.iterations).sum(),
            total_tool_calls: inner.sessions.values().map(|s| s.tool_calls).sum(),
            total_tool_time_ms: inner.sessions.values().map(|s| s.tool_time_ms).sum(),
            total_llm_time_ms: inner.sessions.values().map(|s| s.llm_time_ms).sum(),
            total_input_tokens: inner.sessions.values().map(|s| s.input_tokens).sum(),
            total_output_tokens: inner.sessions.values().map(|s| s.output_tokens).sum(),
        };

        GlobalToolStats {
            total_calls,
            avg_latency_ms,
            success_rate,
            slowest_tools: slowest,
            most_used_tools: most_used,
            most_failed_tools: by_error,
            session_totals,
        }
    }

    /// Get session stats for a specific session.
    pub async fn session_stats(&self, session_id: &str) -> Option<SessionStats> {
        let inner = self.inner.read().await;
        inner.sessions.get(session_id).cloned()
    }

    /// Get all raw tool stats (for persistence/export).
    pub async fn all_stats(&self) -> HashMap<String, ToolStats> {
        let inner = self.inner.read().await;
        inner.tools.clone()
    }

    /// Export the full inner state as JSON for persistence.
    pub async fn export_json(&self) -> serde_json::Value {
        let inner = self.inner.read().await;
        serde_json::to_value(&*inner).unwrap_or(serde_json::Value::Null)
    }

    /// Import stats from a previously persisted JSON blob.
    pub async fn import_json(&self, data: &serde_json::Value) {
        match serde_json::from_value::<ToolStatsInner>(data.clone()) {
            Ok(imported) => {
                let mut inner = self.inner.write().await;
                // Merge: for each tool, add to existing if present
                for (name, stats) in imported.tools {
                    inner.tools.entry(name).or_insert(stats);
                }
                for (sid, session) in imported.sessions {
                    inner.sessions.entry(sid).or_insert(session);
                }
                info!(
                    tools = inner.tools.len(),
                    sessions = inner.sessions.len(),
                    "Imported tool stats from persistence"
                );
            }
            Err(e) => {
                warn!("Failed to import tool stats: {e}");
            }
        }
    }
}

impl Default for ToolStatsTracker {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// ToolStats helpers
// =============================================================================

impl ToolStats {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            call_count: 0,
            success_count: 0,
            failure_count: 0,
            latencies_ms: Vec::with_capacity(MAX_LATENCY_SAMPLES),
            output_sizes: Vec::with_capacity(MAX_OUTPUT_SAMPLES),
            last_called: None,
            errors: Vec::new(),
        }
    }

    fn summary(&self) -> ToolStatsSummary {
        let success_rate = if self.call_count > 0 {
            self.success_count as f64 / self.call_count as f64
        } else {
            1.0
        };

        let avg_latency_ms = if self.latencies_ms.is_empty() {
            0
        } else {
            self.latencies_ms.iter().sum::<u64>() / self.latencies_ms.len() as u64
        };

        let avg_output_size = if self.output_sizes.is_empty() {
            0
        } else {
            self.output_sizes.iter().sum::<usize>() / self.output_sizes.len()
        };

        // Sort top errors by count (descending), take top 5
        let mut top_errors = self.errors.clone();
        top_errors.sort_by(|a, b| b.1.cmp(&a.1));
        top_errors.truncate(5);

        ToolStatsSummary {
            name: self.name.clone(),
            call_count: self.call_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            success_rate,
            avg_latency_ms,
            p50_latency_ms: percentile(&self.latencies_ms, 50),
            p95_latency_ms: percentile(&self.latencies_ms, 95),
            p99_latency_ms: percentile(&self.latencies_ms, 99),
            avg_output_size,
            last_called: self.last_called,
            top_errors,
        }
    }
}

impl SessionStats {
    fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            iterations: 0,
            tool_calls: 0,
            tool_time_ms: 0,
            llm_time_ms: 0,
            input_tokens: 0,
            output_tokens: 0,
        }
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn percentile(data: &[u64], pct: usize) -> u64 {
    if data.is_empty() {
        return 0;
    }
    let mut sorted = data.to_vec();
    sorted.sort_unstable();
    let idx = (pct * sorted.len() / 100).min(sorted.len() - 1);
    sorted[idx]
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
