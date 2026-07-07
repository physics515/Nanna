//! Supervisor patterns for agent lifecycle management
//!
//! Provides Erlang/OTP-inspired supervision:
//! - Restart policies (never, always, on-failure, exponential backoff)
//! - Health checks with configurable probes
//! - Supervision strategies (one-for-one, one-for-all, rest-for-one)
//! - Graceful shutdown and cleanup

use crate::{Agent, AgentConfig, AgentContext, AgentError, RunOptions};
use nanna_llm::LlmClient;
use nanna_tools::ToolRegistry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::time::{interval, timeout};
use tracing::{debug, error, info, warn};
/// Serde helper for Duration (serialize as seconds)
mod serde_duration {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(duration: &Duration, s: S) -> Result<S::Ok, S::Error> {
        duration.as_secs().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let secs = u64::deserialize(d)?;
        Ok(Duration::from_secs(secs))
    }
}

/// Restart policy for supervised agents
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    /// Never restart on failure
    Never,
    /// Always restart, regardless of exit reason
    #[default]
    Always,
    /// Restart only on abnormal termination (errors)
    OnFailure,
    /// Restart with exponential backoff
    ExponentialBackoff {
        /// Initial delay before first restart (seconds)
        #[serde(with = "serde_duration")]
        initial_delay: Duration,
        /// Maximum delay between restarts (seconds)
        #[serde(with = "serde_duration")]
        max_delay: Duration,
        /// Multiplier for each subsequent restart
        multiplier: f64,
        /// Maximum number of restarts before giving up
        max_restarts: u32,
    },
}

impl RestartPolicy {
    /// Create exponential backoff with sensible defaults
    #[must_use]
    pub fn exponential_backoff() -> Self {
        Self::ExponentialBackoff {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(300), // 5 minutes
            multiplier: 2.0,
            max_restarts: 10,
        }
    }
}

/// Health check configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheckConfig {
    /// Interval between health checks (seconds)
    #[serde(with = "serde_duration")]
    pub interval: Duration,
    /// Timeout for each health check (seconds)
    #[serde(with = "serde_duration")]
    pub timeout: Duration,
    /// Number of consecutive failures before marking unhealthy
    pub failure_threshold: u32,
    /// Number of consecutive successes to recover from unhealthy
    pub success_threshold: u32,
    /// Prompt to send for health check (agent responds to prove liveness)
    pub probe_prompt: String,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(60),
            timeout: Duration::from_secs(30),
            failure_threshold: 3,
            success_threshold: 1,
            probe_prompt: "Health check: respond with OK if operational.".to_string(),
        }
    }
}

/// Supervision strategy for handling child failures
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SupervisionStrategy {
    /// Restart only the failed child
    #[default]
    OneForOne,
    /// Restart all children if any one fails
    OneForAll,
    /// Restart the failed child and all children started after it
    RestForOne,
}

/// Current state of a supervised agent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent is starting up
    Starting,
    /// Agent is running and healthy
    Running,
    /// Agent is running but health checks are failing
    Unhealthy,
    /// Agent has stopped normally
    Stopped,
    /// Agent has failed and is awaiting restart
    Failed,
    /// Agent has been terminated and will not restart
    Terminated,
}

/// Statistics for a supervised agent
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentStats {
    /// Total number of restarts
    pub restart_count: u32,
    /// Consecutive restart failures (for backoff)
    pub consecutive_failures: u32,
    /// Last restart time
    pub last_restart: Option<i64>,
    /// Total uptime in seconds
    pub total_uptime_secs: u64,
    /// Current uptime start
    pub current_start: Option<i64>,
    /// Health check pass count
    pub health_checks_passed: u64,
    /// Health check fail count
    pub health_checks_failed: u64,
    /// Consecutive health check failures
    pub consecutive_health_failures: u32,
    /// Consecutive health check successes (counted only while Unhealthy, to gate recovery)
    pub consecutive_health_successes: u32,
}

/// Outcome of folding one health-check result into an agent's health state.
///
/// Pure data — the async caller applies these to the live handle and emits the
/// corresponding events, keeping the state-machine logic testable in isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HealthTransition {
    state: AgentState,
    consecutive_health_failures: u32,
    consecutive_health_successes: u32,
    /// The agent just recovered Unhealthy -> Running (emit `BecameHealthy`).
    recovered: bool,
    /// The agent just degraded Running -> Unhealthy (emit `BecameUnhealthy`).
    became_unhealthy: bool,
}

/// Pure health-check state machine: given the current state, the check outcome,
/// the running consecutive counters, and the thresholds, compute the next state
/// and counters. Recovery requires `success_threshold` *consecutive* successes
/// (not just one), and degradation requires `failure_threshold` consecutive
/// failures. Thresholds are floored at 1 so a zero-config never traps the agent.
fn apply_health_result(
    state: AgentState,
    passed: bool,
    consecutive_health_failures: u32,
    consecutive_health_successes: u32,
    failure_threshold: u32,
    success_threshold: u32,
) -> HealthTransition {
    if passed {
        // Count successes only while trying to recover from Unhealthy.
        let successes = if state == AgentState::Unhealthy {
            consecutive_health_successes.saturating_add(1)
        } else {
            0
        };
        let recovered = state == AgentState::Unhealthy && successes >= success_threshold.max(1);
        debug_assert!(!recovered || state == AgentState::Unhealthy);
        HealthTransition {
            state: if recovered {
                AgentState::Running
            } else {
                state
            },
            consecutive_health_failures: 0,
            consecutive_health_successes: if recovered { 0 } else { successes },
            recovered,
            became_unhealthy: false,
        }
    } else {
        let failures = consecutive_health_failures.saturating_add(1);
        let became_unhealthy = state == AgentState::Running && failures >= failure_threshold.max(1);
        debug_assert!(!became_unhealthy || state == AgentState::Running);
        HealthTransition {
            state: if became_unhealthy {
                AgentState::Unhealthy
            } else {
                state
            },
            consecutive_health_failures: failures,
            consecutive_health_successes: 0,
            recovered: false,
            became_unhealthy,
        }
    }
}

/// Configuration for a supervised agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisedAgentConfig {
    /// Unique identifier
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Agent configuration (not serialized - runtime only)
    #[serde(skip, default)]
    pub agent_config: AgentConfig,
    /// System prompt for the agent
    pub system_prompt: String,
    /// Restart policy
    pub restart_policy: RestartPolicy,
    /// Health check config (None = no health checks)
    pub health_check: Option<HealthCheckConfig>,
    /// Shutdown timeout before force-kill (in seconds, for serde compatibility)
    #[serde(with = "serde_duration")]
    pub shutdown_timeout: Duration,
    /// Priority (lower = started first, stopped last)
    pub priority: i32,
}

impl SupervisedAgentConfig {
    /// Create a new supervised agent config with defaults
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            agent_config: AgentConfig::default(),
            system_prompt: system_prompt.into(),
            restart_policy: RestartPolicy::default(),
            health_check: Some(HealthCheckConfig::default()),
            shutdown_timeout: Duration::from_secs(30),
            priority: 0,
        }
    }

    /// Set agent config
    #[must_use]
    pub fn with_agent_config(mut self, config: AgentConfig) -> Self {
        self.agent_config = config;
        self
    }

    /// Set restart policy
    #[must_use]
    pub fn with_restart_policy(mut self, policy: RestartPolicy) -> Self {
        self.restart_policy = policy;
        self
    }

    /// Set health check config
    #[must_use]
    pub fn with_health_check(mut self, config: Option<HealthCheckConfig>) -> Self {
        self.health_check = config;
        self
    }

    /// Disable health checks
    #[must_use]
    pub fn without_health_check(mut self) -> Self {
        self.health_check = None;
        self
    }

    /// Set priority
    #[must_use]
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// Handle to a running supervised agent
struct SupervisedAgentHandle {
    config: SupervisedAgentConfig,
    state: AgentState,
    stats: AgentStats,
    /// Channel to send shutdown signal
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Next restart delay (for exponential backoff)
    next_restart_delay: Duration,
}

/// Event emitted by the supervisor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorEvent {
    pub timestamp: i64,
    pub agent_id: String,
    pub event_type: SupervisorEventType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorEventType {
    Started,
    Stopped,
    Failed { error: String },
    Restarting { attempt: u32 },
    RestartGaveUp { attempts: u32 },
    HealthCheckPassed,
    HealthCheckFailed { reason: String },
    BecameUnhealthy,
    BecameHealthy,
    Terminated,
}

/// Supervisor for managing agent lifecycles
pub struct Supervisor {
    /// Supervision strategy
    strategy: SupervisionStrategy,
    /// Managed agents
    agents: Arc<RwLock<HashMap<String, SupervisedAgentHandle>>>,
    /// LLM client (shared across agents)
    llm: Arc<LlmClient>,
    /// Tool registry (shared)
    tools: Arc<ToolRegistry>,
    /// Event channel
    event_tx: mpsc::Sender<SupervisorEvent>,
    /// Shutdown signal
    shutdown_tx: Option<mpsc::Sender<()>>,
    /// Whether supervisor is running
    running: Arc<RwLock<bool>>,
}

impl Supervisor {
    /// Create a new supervisor.
    #[must_use]
    pub fn new(
        strategy: SupervisionStrategy,
        llm: Arc<LlmClient>,
        tools: Arc<ToolRegistry>,
    ) -> (Self, mpsc::Receiver<SupervisorEvent>) {
        let (event_tx, event_rx) = mpsc::channel(256);
        
        let supervisor = Self {
            strategy,
            agents: Arc::new(RwLock::new(HashMap::new())),
            llm,
            tools,
            event_tx,
            shutdown_tx: None,
            running: Arc::new(RwLock::new(false)),
        };
        
        (supervisor, event_rx)
    }

    /// Add a supervised agent.
    pub async fn add_agent(&self, config: SupervisedAgentConfig) {
        let handle = SupervisedAgentHandle {
            config: config.clone(),
            state: AgentState::Stopped,
            stats: AgentStats::default(),
            shutdown_tx: None,
            next_restart_delay: match &config.restart_policy {
                RestartPolicy::ExponentialBackoff { initial_delay, .. } => *initial_delay,
                _ => Duration::from_secs(1),
            },
        };
        
        self.agents.write().await.insert(config.id.clone(), handle);
        info!("Added supervised agent: {} ({})", config.name, config.id);
    }

    /// Remove a supervised agent (stops it first if running).
    pub async fn remove_agent(&self, agent_id: &str) -> bool {
        // Stop the agent first
        self.stop_agent(agent_id).await;
        
        let removed = self.agents.write().await.remove(agent_id).is_some();
        if removed {
            info!("Removed supervised agent: {}", agent_id);
        }
        removed
    }

    /// Start a specific agent.
    pub async fn start_agent(&self, agent_id: &str) -> Result<(), AgentError> {
        let mut agents = self.agents.write().await;
        let handle = agents.get_mut(agent_id)
            .ok_or_else(|| AgentError::Stopped)?;
        
        if handle.state == AgentState::Running || handle.state == AgentState::Starting {
            return Ok(()); // Already running
        }
        
        self.spawn_agent_task(handle).await;
        Ok(())
    }

    /// Stop a specific agent gracefully.
    pub async fn stop_agent(&self, agent_id: &str) {
        let mut agents = self.agents.write().await;
        if let Some(handle) = agents.get_mut(agent_id) {
            if let Some(tx) = handle.shutdown_tx.take() {
                let _ = tx.send(());
            }
            handle.state = AgentState::Stopped;
            
            // Update uptime
            if let Some(start) = handle.stats.current_start {
                let uptime = chrono_timestamp() - start;
                handle.stats.total_uptime_secs += u64::try_from(uptime).unwrap_or(0);
                handle.stats.current_start = None;
            }
            
            self.emit_event(agent_id, SupervisorEventType::Stopped).await;
        }
    }

    /// Start all agents (respecting priority order).
    pub async fn start_all(&self) {
        let mut agents: Vec<_> = {
            let agents = self.agents.read().await;
            agents.iter()
                .map(|(id, h)| (id.clone(), h.config.priority))
                .collect()
        };
        
        // Sort by priority (lower first)
        agents.sort_by_key(|(_, p)| *p);
        
        for (id, _) in agents {
            if let Err(e) = self.start_agent(&id).await {
                error!("Failed to start agent {}: {:?}", id, e);
            }
        }
    }

    /// Stop all agents (reverse priority order).
    pub async fn stop_all(&self) {
        let mut agents: Vec<_> = {
            let agents = self.agents.read().await;
            agents.iter()
                .map(|(id, h)| (id.clone(), h.config.priority))
                .collect()
        };
        
        // Sort by priority descending (higher first = stop first)
        agents.sort_by_key(|(_, p)| std::cmp::Reverse(*p));
        
        for (id, _) in agents {
            self.stop_agent(&id).await;
        }
    }

    /// Start the supervisor loop (health checks, restart handling).
    pub async fn run(&mut self) {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        self.shutdown_tx = Some(shutdown_tx);
        *self.running.write().await = true;
        
        let agents = self.agents.clone();
        let llm = self.llm.clone();
        let tools = self.tools.clone();
        let event_tx = self.event_tx.clone();
        let strategy = self.strategy;
        let running = self.running.clone();
        
        // Start all agents
        self.start_all().await;
        
        info!("Supervisor started with {:?} strategy", strategy);
        
        // Main supervisor loop
        tokio::spawn(async move {
            let mut check_interval = interval(Duration::from_secs(5));
            
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        info!("Supervisor shutting down");
                        *running.write().await = false;
                        break;
                    }
                    _ = check_interval.tick() => {
                        Self::check_agents(&agents, &llm, &tools, &event_tx, strategy).await;
                    }
                }
            }
        });
    }

    /// Shutdown the supervisor.
    pub async fn shutdown(&mut self) {
        // Signal shutdown
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(()).await;
        }
        
        // Stop all agents
        self.stop_all().await;
        
        info!("Supervisor shutdown complete");
    }

    /// Get the state of an agent.
    pub async fn get_agent_state(&self, agent_id: &str) -> Option<AgentState> {
        self.agents.read().await.get(agent_id).map(|h| h.state)
    }

    /// Get stats for an agent.
    pub async fn get_agent_stats(&self, agent_id: &str) -> Option<AgentStats> {
        self.agents.read().await.get(agent_id).map(|h| h.stats.clone())
    }

    /// List all supervised agents with their states.
    pub async fn list_agents(&self) -> Vec<(String, String, AgentState, AgentStats)> {
        self.agents.read().await
            .iter()
            .map(|(id, h)| (id.clone(), h.config.name.clone(), h.state, h.stats.clone()))
            .collect()
    }

    // ---- Internal methods ----

    async fn spawn_agent_task(&self, handle: &mut SupervisedAgentHandle) {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        handle.shutdown_tx = Some(shutdown_tx);
        handle.state = AgentState::Starting;
        handle.stats.current_start = Some(chrono_timestamp());
        
        let config = handle.config.clone();
        let _llm = self.llm.clone();
        let _tools = self.tools.clone();
        let agents = self.agents.clone();
        let event_tx = self.event_tx.clone();
        
        tokio::spawn(async move {
            Self::emit_event_static(&event_tx, &config.id, SupervisorEventType::Started).await;
            
            // Update state to running
            {
                let mut agents = agents.write().await;
                if let Some(h) = agents.get_mut(&config.id) {
                    h.state = AgentState::Running;
                }
            }
            
            // TODO: Run actual agent loop here using _llm and _tools
            // Wait for shutdown signal (in a real system, this would run the agent loop)
            let _ = shutdown_rx.await;
            
            debug!("Agent {} received shutdown signal", config.id);
        });
    }

    async fn check_agents(
        agents: &Arc<RwLock<HashMap<String, SupervisedAgentHandle>>>,
        llm: &Arc<LlmClient>,
        tools: &Arc<ToolRegistry>,
        event_tx: &mpsc::Sender<SupervisorEvent>,
        strategy: SupervisionStrategy,
    ) {
        let agent_ids: Vec<String> = {
            let agents = agents.read().await;
            agents.keys().cloned().collect()
        };
        
        for agent_id in agent_ids {
            let should_health_check = {
                let agents = agents.read().await;
                agents.get(&agent_id)
                    .is_some_and(|h| h.state == AgentState::Running && h.config.health_check.is_some())
            };
            
            if should_health_check {
                Self::perform_health_check(agents, llm, tools, event_tx, &agent_id).await;
            }
            
            // Check for agents that need restart
            Self::check_restart(agents, event_tx, &agent_id, strategy).await;
        }
    }

    async fn perform_health_check(
        agents: &Arc<RwLock<HashMap<String, SupervisedAgentHandle>>>,
        llm: &Arc<LlmClient>,
        tools: &Arc<ToolRegistry>,
        event_tx: &mpsc::Sender<SupervisorEvent>,
        agent_id: &str,
    ) {
        let (health_config, agent_config, system_prompt) = {
            let agents = agents.read().await;
            let Some(handle) = agents.get(agent_id) else { return };
            let Some(hc) = &handle.config.health_check else { return };
            (hc.clone(), handle.config.agent_config.clone(), handle.config.system_prompt.clone())
        };
        
        // Perform health check probe
        let context = AgentContext::new(&format!("{}-health", agent_id))
            .with_system_prompt(system_prompt);
        let agent = Agent::new(agent_config, llm.clone(), tools.clone())
            .with_context(context);
        
        let check_result = timeout(
            health_config.timeout,
            agent.run(&health_config.probe_prompt, RunOptions::default()),
        ).await;
        
        let passed = match check_result {
            Ok(Ok(response)) => response.text.to_lowercase().contains("ok"),
            Ok(Err(e)) => {
                warn!("Health check failed for {}: {:?}", agent_id, e);
                false
            }
            Err(_) => {
                warn!("Health check timed out for {}", agent_id);
                false
            }
        };
        
        // Fold the result into the health state machine while holding the lock,
        // then emit events after releasing it (no handle borrow across await).
        let transition = {
            let mut agents = agents.write().await;
            agents.get_mut(agent_id).map(|handle| {
                let (failure_threshold, success_threshold) = handle
                    .config
                    .health_check
                    .as_ref()
                    .map_or((3, 1), |hc| (hc.failure_threshold, hc.success_threshold));
                let t = apply_health_result(
                    handle.state,
                    passed,
                    handle.stats.consecutive_health_failures,
                    handle.stats.consecutive_health_successes,
                    failure_threshold,
                    success_threshold,
                );
                if passed {
                    handle.stats.health_checks_passed += 1;
                } else {
                    handle.stats.health_checks_failed += 1;
                }
                handle.state = t.state;
                handle.stats.consecutive_health_failures = t.consecutive_health_failures;
                handle.stats.consecutive_health_successes = t.consecutive_health_successes;
                t
            })
        };

        let Some(transition) = transition else { return };
        if transition.recovered {
            Self::emit_event_static(event_tx, agent_id, SupervisorEventType::BecameHealthy).await;
        }
        if transition.became_unhealthy {
            Self::emit_event_static(event_tx, agent_id, SupervisorEventType::BecameUnhealthy).await;
        }
        if passed {
            Self::emit_event_static(event_tx, agent_id, SupervisorEventType::HealthCheckPassed).await;
        } else {
            Self::emit_event_static(
                event_tx,
                agent_id,
                SupervisorEventType::HealthCheckFailed {
                    reason: "Probe failed or timed out".to_string(),
                },
            )
            .await;
        }
    }

    async fn check_restart(
        agents: &Arc<RwLock<HashMap<String, SupervisedAgentHandle>>>,
        event_tx: &mpsc::Sender<SupervisorEvent>,
        agent_id: &str,
        _strategy: SupervisionStrategy,
    ) {
        let should_restart = {
            let agents = agents.read().await;
            let Some(handle) = agents.get(agent_id) else { return };
            
            if handle.state != AgentState::Failed {
                return;
            }
            
            match &handle.config.restart_policy {
                RestartPolicy::Never => false,
                RestartPolicy::Always | RestartPolicy::OnFailure => true,
                RestartPolicy::ExponentialBackoff { max_restarts, .. } => {
                    handle.stats.restart_count < *max_restarts
                }
            }
        };
        
        if !should_restart {
            // Give up on restart
            let mut agents = agents.write().await;
            if let Some(handle) = agents.get_mut(agent_id) {
                if handle.state == AgentState::Failed {
                    let attempts = handle.stats.restart_count;
                    handle.state = AgentState::Terminated;
                    Self::emit_event_static(event_tx, agent_id, SupervisorEventType::RestartGaveUp { attempts }).await;
                }
            }
            return;
        }
        
        // Calculate delay for exponential backoff
        let delay = {
            let agents = agents.read().await;
            agents.get(agent_id).map_or(Duration::from_secs(1), |h| h.next_restart_delay)
        };
        
        // Wait before restart
        tokio::time::sleep(delay).await;
        
        // Perform restart
        let mut agents = agents.write().await;
        if let Some(handle) = agents.get_mut(agent_id) {
            handle.stats.restart_count += 1;
            handle.stats.last_restart = Some(chrono_timestamp());
            handle.stats.current_start = Some(chrono_timestamp());
            
            // Update backoff delay
            if let RestartPolicy::ExponentialBackoff { multiplier, max_delay, .. } = &handle.config.restart_policy {
                let new_delay = Duration::from_secs_f64(handle.next_restart_delay.as_secs_f64() * multiplier);
                handle.next_restart_delay = new_delay.min(*max_delay);
            }
            
            handle.state = AgentState::Starting;
            Self::emit_event_static(event_tx, agent_id, SupervisorEventType::Restarting {
                attempt: handle.stats.restart_count,
            }).await;
            
            // In production, would call spawn_agent_task here
            // For now, just transition to Running
            handle.state = AgentState::Running;
        }
    }

    async fn emit_event(&self, agent_id: &str, event_type: SupervisorEventType) {
        Self::emit_event_static(&self.event_tx, agent_id, event_type).await;
    }

    async fn emit_event_static(
        tx: &mpsc::Sender<SupervisorEvent>,
        agent_id: &str,
        event_type: SupervisorEventType,
    ) {
        let event = SupervisorEvent {
            timestamp: chrono_timestamp(),
            agent_id: agent_id.to_string(),
            event_type,
        };
        let _ = tx.send(event).await;
    }
}

fn chrono_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_restart_policy_defaults() {
        let policy = RestartPolicy::default();
        assert!(matches!(policy, RestartPolicy::Always));
    }

    #[test]
    fn test_recovery_needs_consecutive_successes() {
        // success_threshold = 2: one success is not enough to recover.
        let t1 = apply_health_result(AgentState::Unhealthy, true, 5, 0, 3, 2);
        assert_eq!(t1.state, AgentState::Unhealthy);
        assert!(!t1.recovered);
        assert_eq!(t1.consecutive_health_successes, 1);
        assert_eq!(t1.consecutive_health_failures, 0);

        // Second consecutive success crosses the threshold -> Running.
        let t2 = apply_health_result(
            AgentState::Unhealthy,
            true,
            0,
            t1.consecutive_health_successes,
            3,
            2,
        );
        assert_eq!(t2.state, AgentState::Running);
        assert!(t2.recovered);
        assert_eq!(t2.consecutive_health_successes, 0);
    }

    #[test]
    fn test_failure_resets_recovery_progress() {
        // A failure while recovering wipes the success streak (stays Unhealthy).
        let t = apply_health_result(AgentState::Unhealthy, false, 0, 1, 3, 2);
        assert_eq!(t.state, AgentState::Unhealthy);
        assert!(!t.recovered);
        assert!(!t.became_unhealthy);
        assert_eq!(t.consecutive_health_successes, 0);
        assert_eq!(t.consecutive_health_failures, 1);
    }

    #[test]
    fn test_degradation_needs_consecutive_failures() {
        // failure_threshold = 3: first two failures do not degrade a Running agent.
        let t1 = apply_health_result(AgentState::Running, false, 0, 0, 3, 1);
        assert_eq!(t1.state, AgentState::Running);
        assert!(!t1.became_unhealthy);
        let t2 = apply_health_result(
            AgentState::Running,
            false,
            t1.consecutive_health_failures,
            0,
            3,
            1,
        );
        assert_eq!(t2.state, AgentState::Running);
        // Third consecutive failure degrades to Unhealthy.
        let t3 = apply_health_result(
            AgentState::Running,
            false,
            t2.consecutive_health_failures,
            0,
            3,
            1,
        );
        assert_eq!(t3.state, AgentState::Unhealthy);
        assert!(t3.became_unhealthy);
    }

    #[test]
    fn test_pass_while_running_is_stable() {
        // A healthy agent stays Running and never counts recovery successes.
        let t = apply_health_result(AgentState::Running, true, 2, 0, 3, 2);
        assert_eq!(t.state, AgentState::Running);
        assert!(!t.recovered);
        assert_eq!(t.consecutive_health_failures, 0);
        assert_eq!(t.consecutive_health_successes, 0);
    }

    #[test]
    fn test_thresholds_floored_at_one() {
        // A zero success_threshold must not trap the agent: recover on first success.
        let t = apply_health_result(AgentState::Unhealthy, true, 0, 0, 0, 0);
        assert_eq!(t.state, AgentState::Running);
        assert!(t.recovered);
    }

    #[test]
    fn test_supervised_agent_config_builder() {
        let config = SupervisedAgentConfig::new("test", "Test Agent", "You are a test agent.")
            .with_restart_policy(RestartPolicy::OnFailure)
            .with_priority(10);
        
        assert_eq!(config.id, "test");
        assert_eq!(config.priority, 10);
        assert!(matches!(config.restart_policy, RestartPolicy::OnFailure));
    }

    #[tokio::test]
    async fn test_supervisor_add_agent() {
        let llm = Arc::new(LlmClient::anthropic("test"));
        let tools = Arc::new(ToolRegistry::new());
        let (supervisor, _rx) = Supervisor::new(SupervisionStrategy::OneForOne, llm, tools);
        
        let config = SupervisedAgentConfig::new("agent-1", "Agent One", "System prompt");
        supervisor.add_agent(config).await;
        
        let agents = supervisor.list_agents().await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].0, "agent-1");
    }
}
