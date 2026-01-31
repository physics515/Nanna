//! Channel status tracking and live updates
//!
//! Provides real-time health monitoring for messaging channels with:
//! - Connection state tracking
//! - Health check probes
//! - Event emission for UI updates
//! - Automatic reconnection handling

use crate::queue::QueueStats;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, RwLock};
use tracing::warn;

/// Connection state for a channel
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    /// Not configured
    Unconfigured,
    /// Configured but not connected
    Disconnected,
    /// Attempting to connect
    Connecting,
    /// Connected and healthy
    Connected,
    /// Connected but experiencing issues
    Degraded,
    /// Rate limited
    RateLimited,
    /// Authentication failed
    AuthFailed,
    /// Temporarily unavailable
    Unavailable,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::Unconfigured
    }
}

impl ConnectionState {
    /// Check if the state represents a connected channel
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected | Self::Degraded | Self::RateLimited)
    }

    /// Check if the state represents a healthy channel
    pub fn is_healthy(&self) -> bool {
        matches!(self, Self::Connected)
    }

    /// Get a human-readable status string
    pub fn status_text(&self) -> &'static str {
        match self {
            Self::Unconfigured => "Not configured",
            Self::Disconnected => "Disconnected",
            Self::Connecting => "Connecting...",
            Self::Connected => "Connected",
            Self::Degraded => "Degraded",
            Self::RateLimited => "Rate limited",
            Self::AuthFailed => "Auth failed",
            Self::Unavailable => "Unavailable",
        }
    }
}

/// Health metrics for a channel
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HealthMetrics {
    /// Last successful health check
    pub last_healthy: Option<i64>,
    /// Last failed health check
    pub last_failure: Option<i64>,
    /// Consecutive failures
    pub consecutive_failures: u32,
    /// Average response time (ms)
    pub avg_response_ms: Option<f64>,
    /// Messages sent in last hour
    pub messages_sent_hour: u32,
    /// Messages failed in last hour
    pub messages_failed_hour: u32,
    /// Current rate limit cooldown remaining (ms)
    pub rate_limit_remaining_ms: Option<u64>,
}

/// Complete status for a channel
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStatus {
    /// Provider name (telegram, discord, etc.)
    pub provider: String,
    /// Display name
    pub name: String,
    /// Connection state
    pub state: ConnectionState,
    /// Whether the channel is configured
    pub configured: bool,
    /// Whether the channel is enabled
    pub enabled: bool,
    /// Health metrics
    pub health: HealthMetrics,
    /// Queue statistics
    pub queue: QueueStats,
    /// Last state change timestamp (Unix ms)
    pub last_state_change: i64,
    /// Additional details (e.g., error message)
    pub details: Option<String>,
}

impl Default for ChannelStatus {
    fn default() -> Self {
        Self {
            provider: String::new(),
            name: String::new(),
            state: ConnectionState::Unconfigured,
            configured: false,
            enabled: false,
            health: HealthMetrics::default(),
            queue: QueueStats::default(),
            last_state_change: chrono::Utc::now().timestamp_millis(),
            details: None,
        }
    }
}

/// Event emitted when channel status changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusEvent {
    /// Provider that changed
    pub provider: String,
    /// New status
    pub status: ChannelStatus,
    /// Previous state (if changed)
    pub previous_state: Option<ConnectionState>,
    /// Timestamp
    pub timestamp: i64,
}

/// Channel status manager
pub struct StatusManager {
    /// Current status for all channels
    statuses: Arc<RwLock<HashMap<String, ChannelStatus>>>,
    /// Event broadcaster
    event_tx: broadcast::Sender<StatusEvent>,
    /// Health check interval
    health_check_interval: Duration,
    /// Response time samples for averaging
    response_times: Arc<RwLock<HashMap<String, Vec<f64>>>>,
}

impl StatusManager {
    /// Create a new status manager
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(100);
        Self {
            statuses: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
            health_check_interval: Duration::from_secs(30),
            response_times: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Set the health check interval
    pub fn with_health_check_interval(mut self, interval: Duration) -> Self {
        self.health_check_interval = interval;
        self
    }

    /// Subscribe to status events
    pub fn subscribe(&self) -> broadcast::Receiver<StatusEvent> {
        self.event_tx.subscribe()
    }

    /// Initialize status for a provider
    pub async fn register(&self, provider: &str, name: &str, configured: bool, enabled: bool) {
        let mut statuses = self.statuses.write().await;
        let status = statuses.entry(provider.to_string()).or_insert_with(|| {
            ChannelStatus {
                provider: provider.to_string(),
                name: name.to_string(),
                ..Default::default()
            }
        });

        status.name = name.to_string();
        status.configured = configured;
        status.enabled = enabled;
        
        if !configured {
            status.state = ConnectionState::Unconfigured;
        } else if !enabled {
            status.state = ConnectionState::Disconnected;
        }
    }

    /// Update channel state
    pub async fn set_state(&self, provider: &str, state: ConnectionState, details: Option<String>) {
        let event = {
            let mut statuses = self.statuses.write().await;
            let status = statuses.get_mut(provider);
            
            let Some(status) = status else {
                warn!("Attempted to set state for unknown provider: {}", provider);
                return;
            };

            let previous_state = if status.state != state {
                Some(status.state)
            } else {
                None
            };

            status.state = state;
            status.details = details;
            status.last_state_change = chrono::Utc::now().timestamp_millis();

            // Update health metrics based on state
            match state {
                ConnectionState::Connected => {
                    status.health.last_healthy = Some(chrono::Utc::now().timestamp_millis());
                    status.health.consecutive_failures = 0;
                }
                ConnectionState::RateLimited => {
                    // Don't count as failure, just degraded
                }
                ConnectionState::AuthFailed | ConnectionState::Unavailable => {
                    status.health.last_failure = Some(chrono::Utc::now().timestamp_millis());
                    status.health.consecutive_failures += 1;
                }
                _ => {}
            }

            StatusEvent {
                provider: provider.to_string(),
                status: status.clone(),
                previous_state,
                timestamp: chrono::Utc::now().timestamp_millis(),
            }
        };

        // Broadcast event (ignore send errors if no subscribers)
        let _ = self.event_tx.send(event);
    }

    /// Record a successful health check
    pub async fn record_health_check(&self, provider: &str, response_time_ms: f64) {
        let mut statuses = self.statuses.write().await;
        let Some(status) = statuses.get_mut(provider) else {
            return;
        };

        status.health.last_healthy = Some(chrono::Utc::now().timestamp_millis());
        status.health.consecutive_failures = 0;

        // Update average response time
        drop(statuses);
        self.update_response_time(provider, response_time_ms).await;
    }

    /// Record a failed health check
    pub async fn record_health_failure(&self, provider: &str) {
        let mut statuses = self.statuses.write().await;
        let Some(status) = statuses.get_mut(provider) else {
            return;
        };

        status.health.last_failure = Some(chrono::Utc::now().timestamp_millis());
        status.health.consecutive_failures += 1;

        // Auto-degrade if too many failures
        if status.health.consecutive_failures >= 3 && status.state == ConnectionState::Connected {
            status.state = ConnectionState::Degraded;
            status.last_state_change = chrono::Utc::now().timestamp_millis();
        }
    }

    /// Update response time average
    async fn update_response_time(&self, provider: &str, response_time_ms: f64) {
        const MAX_SAMPLES: usize = 100;

        let mut response_times = self.response_times.write().await;
        let samples = response_times.entry(provider.to_string()).or_insert_with(Vec::new);

        samples.push(response_time_ms);
        if samples.len() > MAX_SAMPLES {
            samples.remove(0);
        }

        let avg = samples.iter().sum::<f64>() / samples.len() as f64;

        // Update in status
        drop(response_times);
        let mut statuses = self.statuses.write().await;
        if let Some(status) = statuses.get_mut(provider) {
            status.health.avg_response_ms = Some(avg);
        }
    }

    /// Record rate limit
    pub async fn record_rate_limit(&self, provider: &str, cooldown_ms: u64) {
        let event = {
            let mut statuses = self.statuses.write().await;
            let Some(status) = statuses.get_mut(provider) else {
                return;
            };

            let previous_state = if status.state != ConnectionState::RateLimited {
                Some(status.state)
            } else {
                None
            };

            status.state = ConnectionState::RateLimited;
            status.health.rate_limit_remaining_ms = Some(cooldown_ms);
            status.last_state_change = chrono::Utc::now().timestamp_millis();

            StatusEvent {
                provider: provider.to_string(),
                status: status.clone(),
                previous_state,
                timestamp: chrono::Utc::now().timestamp_millis(),
            }
        };

        let _ = self.event_tx.send(event);
    }

    /// Clear rate limit
    pub async fn clear_rate_limit(&self, provider: &str) {
        let event = {
            let mut statuses = self.statuses.write().await;
            let Some(status) = statuses.get_mut(provider) else {
                return;
            };

            if status.state == ConnectionState::RateLimited {
                status.state = ConnectionState::Connected;
                status.health.rate_limit_remaining_ms = None;
                status.last_state_change = chrono::Utc::now().timestamp_millis();

                Some(StatusEvent {
                    provider: provider.to_string(),
                    status: status.clone(),
                    previous_state: Some(ConnectionState::RateLimited),
                    timestamp: chrono::Utc::now().timestamp_millis(),
                })
            } else {
                None
            }
        };

        if let Some(event) = event {
            let _ = self.event_tx.send(event);
        }
    }

    /// Update queue statistics
    pub async fn update_queue_stats(&self, provider: &str, stats: QueueStats) {
        let mut statuses = self.statuses.write().await;
        if let Some(status) = statuses.get_mut(provider) {
            status.queue = stats;
            
            // Update rate limit info from queue
            if let Some(cooldown_ms) = status.queue.cooldown_remaining_ms {
                status.health.rate_limit_remaining_ms = Some(cooldown_ms);
            }
        }
    }

    /// Record a sent message
    pub async fn record_message_sent(&self, provider: &str) {
        let mut statuses = self.statuses.write().await;
        if let Some(status) = statuses.get_mut(provider) {
            status.health.messages_sent_hour += 1;
        }
    }

    /// Record a failed message
    pub async fn record_message_failed(&self, provider: &str) {
        let mut statuses = self.statuses.write().await;
        if let Some(status) = statuses.get_mut(provider) {
            status.health.messages_failed_hour += 1;
        }
    }

    /// Get status for a provider
    pub async fn get(&self, provider: &str) -> Option<ChannelStatus> {
        let statuses = self.statuses.read().await;
        statuses.get(provider).cloned()
    }

    /// Get status for all providers
    pub async fn all(&self) -> HashMap<String, ChannelStatus> {
        let statuses = self.statuses.read().await;
        statuses.clone()
    }

    /// Get a summary of all channel states
    pub async fn summary(&self) -> StatusSummary {
        let statuses = self.statuses.read().await;
        
        let mut summary = StatusSummary::default();
        for status in statuses.values() {
            summary.total += 1;
            if status.configured {
                summary.configured += 1;
            }
            if status.state.is_connected() {
                summary.connected += 1;
            }
            if status.state.is_healthy() {
                summary.healthy += 1;
            }
        }
        
        summary
    }

    /// Reset hourly statistics (call this every hour)
    pub async fn reset_hourly_stats(&self) {
        let mut statuses = self.statuses.write().await;
        for status in statuses.values_mut() {
            status.health.messages_sent_hour = 0;
            status.health.messages_failed_hour = 0;
        }
    }
}

impl Default for StatusManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Summary of all channel statuses
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StatusSummary {
    /// Total registered channels
    pub total: usize,
    /// Configured channels
    pub configured: usize,
    /// Connected channels
    pub connected: usize,
    /// Healthy channels
    pub healthy: usize,
}

/// Health check result
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    /// Provider checked
    pub provider: String,
    /// Whether the check passed
    pub healthy: bool,
    /// Response time in milliseconds
    pub response_time_ms: Option<f64>,
    /// Error message if unhealthy
    pub error: Option<String>,
}

/// Health checker that runs periodic probes
pub struct HealthChecker {
    /// Status manager to update
    status_manager: Arc<StatusManager>,
    /// Check interval
    interval: Duration,
    /// Whether running
    running: Arc<RwLock<bool>>,
}

impl HealthChecker {
    /// Create a new health checker
    pub fn new(status_manager: Arc<StatusManager>, interval: Duration) -> Self {
        Self {
            status_manager,
            interval,
            running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the health checker background task
    pub fn start<F, Fut>(&self, check_fn: F) -> tokio::task::JoinHandle<()>
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = HealthCheckResult> + Send,
    {
        let status_manager = self.status_manager.clone();
        let interval = self.interval;
        let running = self.running.clone();

        tokio::spawn(async move {
            *running.write().await = true;

            loop {
                if !*running.read().await {
                    break;
                }

                // Get all providers to check
                let providers: Vec<String> = status_manager
                    .all()
                    .await
                    .into_iter()
                    .filter(|(_, s)| s.configured && s.enabled)
                    .map(|(k, _)| k)
                    .collect();

                for provider in providers {
                    let result = check_fn(provider.clone()).await;

                    if result.healthy {
                        if let Some(response_time) = result.response_time_ms {
                            status_manager.record_health_check(&provider, response_time).await;
                        }
                        status_manager.set_state(&provider, ConnectionState::Connected, None).await;
                    } else {
                        status_manager.record_health_failure(&provider).await;
                        status_manager.set_state(
                            &provider,
                            ConnectionState::Degraded,
                            result.error,
                        ).await;
                    }
                }

                tokio::time::sleep(interval).await;
            }
        })
    }

    /// Stop the health checker
    pub async fn stop(&self) {
        *self.running.write().await = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_status_manager() {
        let manager = StatusManager::new();
        
        manager.register("telegram", "Telegram", true, true).await;
        manager.set_state("telegram", ConnectionState::Connected, None).await;
        
        let status = manager.get("telegram").await.unwrap();
        assert_eq!(status.state, ConnectionState::Connected);
        assert!(status.configured);
    }

    #[tokio::test]
    async fn test_health_metrics() {
        let manager = StatusManager::new();
        manager.register("test", "Test", true, true).await;
        
        manager.record_health_check("test", 50.0).await;
        manager.record_health_check("test", 100.0).await;
        
        let status = manager.get("test").await.unwrap();
        assert!(status.health.avg_response_ms.is_some());
        assert_eq!(status.health.consecutive_failures, 0);
    }

    #[tokio::test]
    async fn test_rate_limit_tracking() {
        let manager = StatusManager::new();
        manager.register("test", "Test", true, true).await;
        manager.set_state("test", ConnectionState::Connected, None).await;
        
        manager.record_rate_limit("test", 5000).await;
        
        let status = manager.get("test").await.unwrap();
        assert_eq!(status.state, ConnectionState::RateLimited);
        assert_eq!(status.health.rate_limit_remaining_ms, Some(5000));
    }
}
