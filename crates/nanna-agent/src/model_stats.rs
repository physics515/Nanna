//! Model performance statistics tracking.
//!
//! Tracks per-model metrics across requests to inform routing decisions
//! and provide visibility into model behavior:
//! - Response latency (p50, p95, p99)
//! - Token throughput (tokens/sec)
//! - Error rates and downtime detection
//! - Cost tracking (input/output/cache tokens)
//! - Success/failure rates per complexity tier

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, info};
use nanna_storage::StoredModelStats;

/// Global model statistics tracker. Thread-safe, designed for concurrent access.
#[derive(Debug, Clone)]
pub struct ModelStatsTracker {
    inner: Arc<RwLock<StatsInner>>, 
}

#[derive(Debug, Default)]
struct StatsInner {
    /// Per-model statistics
    models: HashMap<String, ModelStats>,
}

/// Accumulated statistics for a single model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStats {
    /// Model identifier
    pub model: String,
    /// Total requests made
    pub total_requests: u64,
    /// Successful requests
    pub successful_requests: u64,
    /// Failed requests (errors, timeouts, malformed responses)
    pub failed_requests: u64,
    /// Total input tokens consumed
    pub total_input_tokens: u64,
    /// Total output tokens generated
    pub total_output_tokens: u64,
    /// Total tokens served from cache
    pub total_cache_read_tokens: u64,
    /// Total tokens written to cache
    pub total_cache_creation_tokens: u64,
    /// Response latencies in milliseconds (ring buffer, last N)
    pub latencies_ms: Vec<u64>,
    /// Tokens per second measurements (ring buffer, last N)
    pub throughput_tps: Vec<f64>,
    /// Consecutive failures (reset on success) - for downtime detection
    pub consecutive_failures: u32,
    /// Timestamp of last successful request
    pub last_success_epoch_ms: u64,
    /// Timestamp of last failure
    pub last_failure_epoch_ms: u64,
    /// Per-tier success counts (for routing quality feedback)
    pub tier_successes: TierCounts,
    /// Per-tier failure counts
    pub tier_failures: TierCounts,
    /// Number of escalations (cheap model failed, had to use more expensive one)
    pub escalations: u64,
}

/// Per-complexity-tier counters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TierCounts {
    pub simple: u64,
    pub medium: u64,
    pub complex: u64,
}

/// A completed request observation to record.
#[derive(Debug)]
pub struct RequestObservation {
    pub model: String,
    pub success: bool,
    pub latency: Duration,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cache_read_tokens: u32,
    pub cache_creation_tokens: u32,
    /// What complexity tier was this request classified as?
    pub tier: Option<super::loop_runner::TaskComplexity>,
    /// Was this an escalation from a cheaper model?
    pub escalated: bool,
}

/// Summary stats for a model, suitable for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelStatsSummary {
    pub model: String,
    pub total_requests: u64,
    pub success_rate: f64,
    pub avg_latency_ms: u64,
    pub p95_latency_ms: u64,
    pub avg_throughput_tps: f64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub cache_hit_rate: f64,
    pub consecutive_failures: u32,
    pub is_healthy: bool,
    pub escalation_count: u64,
}

/// Per-request stats to attach to an AgentResponse for UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestModelStats {
    /// Which model actually handled this request
    pub model: String,
    /// Was this a routed request (vs primary model)?
    pub was_routed: bool,
    /// Complexity tier classification
    pub tier: String,
    /// Latency for this specific request (ms)
    pub latency_ms: u64,
    /// Tokens per second for this request
    pub throughput_tps: f64,
    /// Cache tokens read (saved money)
    pub cache_read_tokens: u32,
    /// Cache tokens written
    pub cache_creation_tokens: u32,
    /// Input tokens
    pub input_tokens: u32,
    /// Output tokens
    pub output_tokens: u32,
}

const MAX_LATENCY_SAMPLES: usize = 100;
const MAX_THROUGHPUT_SAMPLES: usize = 100;
/// A model with this many consecutive failures is considered unhealthy
const UNHEALTHY_THRESHOLD: u32 = 3;

impl ModelStatsTracker {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(StatsInner::default())),
        }
    }

    /// Record a completed request observation.
    pub async fn record(&self, obs: RequestObservation) {
        let mut inner = self.inner.write().await;
        let stats = inner.models.entry(obs.model.clone()).or_insert_with(|| ModelStats::new(&obs.model));

        stats.total_requests += 1;
        let latency_ms = obs.latency.as_millis() as u64;

        if obs.success {
            stats.successful_requests += 1;
            stats.consecutive_failures = 0;
            stats.last_success_epoch_ms = now_epoch_ms();

            // Record latency
            if stats.latencies_ms.len() >= MAX_LATENCY_SAMPLES {
                stats.latencies_ms.remove(0);
            }
            stats.latencies_ms.push(latency_ms);

            // Record throughput
            if obs.latency.as_millis() > 0 {
                let tps = f64::from(obs.output_tokens) / obs.latency.as_secs_f64();
                if stats.throughput_tps.len() >= MAX_THROUGHPUT_SAMPLES {
                    stats.throughput_tps.remove(0);
                }
                stats.throughput_tps.push(tps);
            }

            // Tier tracking
            if let Some(tier) = obs.tier {
                match tier {
                    super::loop_runner::TaskComplexity::Simple => stats.tier_successes.simple += 1,
                    super::loop_runner::TaskComplexity::Medium => stats.tier_successes.medium += 1,
                    super::loop_runner::TaskComplexity::Complex => stats.tier_successes.complex += 1,
                }
            }
        } else {
            stats.failed_requests += 1;
            stats.consecutive_failures += 1;
            stats.last_failure_epoch_ms = now_epoch_ms();

            if let Some(tier) = obs.tier {
                match tier {
                    super::loop_runner::TaskComplexity::Simple => stats.tier_failures.simple += 1,
                    super::loop_runner::TaskComplexity::Medium => stats.tier_failures.medium += 1,
                    super::loop_runner::TaskComplexity::Complex => stats.tier_failures.complex += 1,
                }
            }
        }

        // Token accounting
        stats.total_input_tokens += u64::from(obs.input_tokens);
        stats.total_output_tokens += u64::from(obs.output_tokens);
        stats.total_cache_read_tokens += u64::from(obs.cache_read_tokens);
        stats.total_cache_creation_tokens += u64::from(obs.cache_creation_tokens);

        if obs.escalated {
            stats.escalations += 1;
        }

        debug!(model = %obs.model, success = obs.success, latency_ms = latency_ms, input = obs.input_tokens, output = obs.output_tokens, cache_read = obs.cache_read_tokens, "📊 Model stats recorded");
    }

    /// Check if a model is considered healthy (not in a failure streak).
    pub async fn is_healthy(&self, model: &str) -> bool {
        let inner = self.inner.read().await;
        inner.models.get(model)
            .map_or(true, |s| s.consecutive_failures < UNHEALTHY_THRESHOLD)
    }

    /// Get summary statistics for all tracked models.
    pub async fn summaries(&self) -> Vec<ModelStatsSummary> {
        let inner = self.inner.read().await;
        inner.models.values().map(ModelStats::summary).collect()
    }

    /// Get summary for a specific model.
    pub async fn summary(&self, model: &str) -> Option<ModelStatsSummary> {
        let inner = self.inner.read().await;
        inner.models.get(model).map(ModelStats::summary)
    }

    /// Get all raw stats (for persistence/export).
    pub async fn all_stats(&self) -> HashMap<String, ModelStats> {
        let inner = self.inner.read().await;
        inner.models.clone()
    }

    /// Load stats from a previous session (e.g., from disk).
    pub async fn load(&self, stats: HashMap<String, ModelStats>) {
        let mut inner = self.inner.write().await;
        inner.models = stats;
        info!(models = inner.models.len(), "Loaded model stats from persistence");
    }
}

impl Default for ModelStatsTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelStats {
    fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            total_requests: 0,
            successful_requests: 0,
            failed_requests: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            total_cache_read_tokens: 0,
            total_cache_creation_tokens: 0,
            latencies_ms: Vec::with_capacity(MAX_LATENCY_SAMPLES),
            throughput_tps: Vec::with_capacity(MAX_THROUGHPUT_SAMPLES),
            consecutive_failures: 0,
            last_success_epoch_ms: 0,
            last_failure_epoch_ms: 0,
            tier_successes: TierCounts::default(),
            tier_failures: TierCounts::default(),
            escalations: 0,
        }
    }

    fn summary(&self) -> ModelStatsSummary {
        let success_rate = if self.total_requests > 0 {
            self.successful_requests as f64 / self.total_requests as f64
        } else {
            1.0
        };

        let avg_latency_ms = if self.latencies_ms.is_empty() {
            0
        } else {
            self.latencies_ms.iter().sum::<u64>() / self.latencies_ms.len() as u64
        };

        let p95_latency_ms = percentile(&self.latencies_ms, 95);

        let avg_throughput_tps = if self.throughput_tps.is_empty() {
            0.0
        } else {
            self.throughput_tps.iter().sum::<f64>() / self.throughput_tps.len() as f64
        };

        let total_cacheable = self.total_input_tokens + self.total_cache_read_tokens;
        let cache_hit_rate = if total_cacheable > 0 {
            self.total_cache_read_tokens as f64 / total_cacheable as f64
        } else {
            0.0
        };

        ModelStatsSummary {
            model: self.model.clone(),
            total_requests: self.total_requests,
            success_rate,
            avg_latency_ms,
            p95_latency_ms,
            avg_throughput_tps,
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
            total_cache_read_tokens: self.total_cache_read_tokens,
            cache_hit_rate,
            consecutive_failures: self.consecutive_failures,
            is_healthy: self.consecutive_failures < UNHEALTHY_THRESHOLD,
            escalation_count: self.escalations,
        }
    }
}

fn percentile(sorted_data: &[u64], pct: usize) -> u64 {
    if sorted_data.is_empty() {
        return 0;
    }
    let mut sorted = sorted_data.to_vec();
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

// =============================================================================
// Storage bridge (converts to/from nanna-storage::StoredModelStats)
// =============================================================================

impl ModelStatsTracker {
    /// Export all stats as storable format (for nanna-storage persistence).
    pub async fn export_for_storage(&self) -> Vec<StorableModelStats> {
        let inner = self.inner.read().await;
        inner.models.values().map(|s| StorableModelStats {
            model: s.model.clone(),
            total_requests: s.total_requests,
            successful_requests: s.successful_requests,
            failed_requests: s.failed_requests,
            total_input_tokens: s.total_input_tokens,
            total_output_tokens: s.total_output_tokens,
            total_cache_read_tokens: s.total_cache_read_tokens,
            total_cache_creation_tokens: s.total_cache_creation_tokens,
            consecutive_failures: s.consecutive_failures,
            last_success_epoch_ms: s.last_success_epoch_ms,
            last_failure_epoch_ms: s.last_failure_epoch_ms,
            tier_successes_simple: s.tier_successes.simple,
            tier_successes_medium: s.tier_successes.medium,
            tier_successes_complex: s.tier_successes.complex,
            tier_failures_simple: s.tier_failures.simple,
            tier_failures_medium: s.tier_failures.medium,
            tier_failures_complex: s.tier_failures.complex,
            escalations: s.escalations,
            latencies_ms: s.latencies_ms.clone(),
            throughput_tps: s.throughput_tps.clone(),
        }).collect()
    }

    /// Import stats from storage format.
    pub async fn import_from_storage(&self, stored: Vec<StorableModelStats>) {
        let mut inner = self.inner.write().await;
        for s in stored {
            let stats = ModelStats {
                model: s.model.clone(),
                total_requests: s.total_requests,
                successful_requests: s.successful_requests,
                failed_requests: s.failed_requests,
                total_input_tokens: s.total_input_tokens,
                total_output_tokens: s.total_output_tokens,
                total_cache_read_tokens: s.total_cache_read_tokens,
                total_cache_creation_tokens: s.total_cache_creation_tokens,
                consecutive_failures: s.consecutive_failures,
                last_success_epoch_ms: s.last_success_epoch_ms,
                last_failure_epoch_ms: s.last_failure_epoch_ms,
                tier_successes: TierCounts {
                    simple: s.tier_successes_simple,
                    medium: s.tier_successes_medium,
                    complex: s.tier_successes_complex,
                },
                tier_failures: TierCounts {
                    simple: s.tier_failures_simple,
                    medium: s.tier_failures_medium,
                    complex: s.tier_failures_complex,
                },
                escalations: s.escalations,
                latencies_ms: s.latencies_ms,
                throughput_tps: s.throughput_tps,
            };
            inner.models.insert(s.model, stats);
        }
        info!(models = inner.models.len(), "Imported model stats from storage");
    }
}

/// Flat struct matching nanna-storage::StoredModelStats layout.
/// This avoids a cross-crate dependency while keeping the types aligned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorableModelStats {
    pub model: String,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_cache_read_tokens: u64,
    pub total_cache_creation_tokens: u64,
    pub consecutive_failures: u32,
    pub last_success_epoch_ms: u64,
    pub last_failure_epoch_ms: u64,
    pub tier_successes_simple: u64,
    pub tier_successes_medium: u64,
    pub tier_successes_complex: u64,
    pub tier_failures_simple: u64,
    pub tier_failures_medium: u64,
    pub tier_failures_complex: u64,
    pub escalations: u64,
    pub latencies_ms: Vec<u64>,
    pub throughput_tps: Vec<f64>,
}

// =============================================================================
// From impls between StorableModelStats <-> StoredModelStats
// =============================================================================

impl From<StoredModelStats> for StorableModelStats {
    fn from(s: StoredModelStats) -> Self {
        Self {
            model: s.model,
            total_requests: s.total_requests,
            successful_requests: s.successful_requests,
            failed_requests: s.failed_requests,
            total_input_tokens: s.total_input_tokens,
            total_output_tokens: s.total_output_tokens,
            total_cache_read_tokens: s.total_cache_read_tokens,
            total_cache_creation_tokens: s.total_cache_creation_tokens,
            consecutive_failures: s.consecutive_failures,
            last_success_epoch_ms: s.last_success_epoch_ms,
            last_failure_epoch_ms: s.last_failure_epoch_ms,
            tier_successes_simple: s.tier_successes_simple,
            tier_successes_medium: s.tier_successes_medium,
            tier_successes_complex: s.tier_successes_complex,
            tier_failures_simple: s.tier_failures_simple,
            tier_failures_medium: s.tier_failures_medium,
            tier_failures_complex: s.tier_failures_complex,
            escalations: s.escalations,
            latencies_ms: s.latencies_ms,
            throughput_tps: s.throughput_tps,
        }
    }
}

impl From<StorableModelStats> for StoredModelStats {
    fn from(s: StorableModelStats) -> Self {
        Self {
            model: s.model,
            total_requests: s.total_requests,
            successful_requests: s.successful_requests,
            failed_requests: s.failed_requests,
            total_input_tokens: s.total_input_tokens,
            total_output_tokens: s.total_output_tokens,
            total_cache_read_tokens: s.total_cache_read_tokens,
            total_cache_creation_tokens: s.total_cache_creation_tokens,
            consecutive_failures: s.consecutive_failures,
            last_success_epoch_ms: s.last_success_epoch_ms,
            last_failure_epoch_ms: s.last_failure_epoch_ms,
            tier_successes_simple: s.tier_successes_simple,
            tier_successes_medium: s.tier_successes_medium,
            tier_successes_complex: s.tier_successes_complex,
            tier_failures_simple: s.tier_failures_simple,
            tier_failures_medium: s.tier_failures_medium,
            tier_failures_complex: s.tier_failures_complex,
            escalations: s.escalations,
            latencies_ms: s.latencies_ms,
            throughput_tps: s.throughput_tps,
        }
    }
}