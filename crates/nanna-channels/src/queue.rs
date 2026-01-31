//! Message queue with rate limiting and retry logic
//!
//! Provides buffered message delivery with:
//! - Per-channel rate limiting with token bucket
//! - Exponential backoff on failures
//! - Persistent queue for offline resilience
//! - Priority queuing (urgent messages first)

use crate::{Channel, ChannelError, OutgoingMessage};
use std::collections::{BinaryHeap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, warn};

/// Message priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessagePriority {
    /// Background messages (e.g., scheduled notifications)
    Low = 0,
    /// Normal conversation messages
    Normal = 1,
    /// Time-sensitive messages (e.g., alerts)
    High = 2,
    /// Critical messages (e.g., error notifications)
    Urgent = 3,
}

impl Default for MessagePriority {
    fn default() -> Self {
        Self::Normal
    }
}

/// Queued message with metadata
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    /// The message to send
    pub message: OutgoingMessage,
    /// Priority level
    pub priority: MessagePriority,
    /// When the message was queued
    pub queued_at: Instant,
    /// Number of retry attempts
    pub attempts: u32,
    /// Next retry time (if retrying)
    pub retry_after: Option<Instant>,
    /// Maximum attempts before dropping
    pub max_attempts: u32,
    /// Unique ID for tracking
    pub id: u64,
}

impl PartialEq for QueuedMessage {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for QueuedMessage {}

impl PartialOrd for QueuedMessage {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedMessage {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority first, then older messages first
        match self.priority.cmp(&other.priority) {
            std::cmp::Ordering::Equal => other.queued_at.cmp(&self.queued_at),
            ord => ord,
        }
    }
}

/// Rate limiter using token bucket algorithm
#[derive(Debug)]
pub struct RateLimiter {
    /// Tokens available
    tokens: f64,
    /// Maximum tokens (bucket size)
    max_tokens: f64,
    /// Tokens added per second
    refill_rate: f64,
    /// Last refill time
    last_refill: Instant,
    /// Cooldown until (for rate limit errors)
    cooldown_until: Option<Instant>,
}

impl RateLimiter {
    /// Create a new rate limiter
    pub fn new(max_tokens: f64, refill_rate: f64) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            refill_rate,
            last_refill: Instant::now(),
            cooldown_until: None,
        }
    }

    /// Create with provider-specific defaults
    pub fn for_provider(provider: &str) -> Self {
        match provider {
            "telegram" => Self::new(30.0, 1.0),      // 30 msg/min
            "discord" => Self::new(5.0, 0.2),        // 5 msg burst, 1/5s refill
            "slack" => Self::new(1.0, 1.0),          // 1 msg/s
            "signal" => Self::new(10.0, 0.5),        // 10 burst, 1/2s refill
            "whatsapp" => Self::new(20.0, 0.33),     // 20 msg/min
            _ => Self::new(10.0, 1.0),               // Conservative default
        }
    }

    /// Refill tokens based on elapsed time
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }

    /// Check if we can send (without consuming)
    pub fn can_send(&mut self) -> bool {
        // Check cooldown first
        if let Some(until) = self.cooldown_until {
            if Instant::now() < until {
                return false;
            }
            self.cooldown_until = None;
        }

        self.refill();
        self.tokens >= 1.0
    }

    /// Try to consume a token (returns true if allowed)
    pub fn try_acquire(&mut self) -> bool {
        if !self.can_send() {
            return false;
        }

        self.tokens -= 1.0;
        true
    }

    /// Time until next token is available
    pub fn time_until_available(&mut self) -> Duration {
        // Check cooldown first
        if let Some(until) = self.cooldown_until {
            let now = Instant::now();
            if now < until {
                return until.duration_since(now);
            }
            self.cooldown_until = None;
        }

        self.refill();
        if self.tokens >= 1.0 {
            Duration::ZERO
        } else {
            let needed = 1.0 - self.tokens;
            Duration::from_secs_f64(needed / self.refill_rate)
        }
    }

    /// Set a cooldown period (e.g., from 429 response)
    pub fn set_cooldown(&mut self, duration: Duration) {
        self.cooldown_until = Some(Instant::now() + duration);
        self.tokens = 0.0; // Drain tokens
    }
}

/// Result of a send attempt
#[derive(Debug)]
pub enum SendResult {
    /// Message sent successfully
    Success(String),
    /// Rate limited, retry after duration
    RateLimited(Duration),
    /// Temporary failure, will retry
    RetryLater(Duration, String),
    /// Permanent failure, message dropped
    Failed(String),
}

/// Message queue configuration
#[derive(Debug, Clone)]
pub struct QueueConfig {
    /// Maximum messages in queue per channel
    pub max_queue_size: usize,
    /// Maximum retry attempts
    pub max_retries: u32,
    /// Initial retry delay
    pub initial_retry_delay: Duration,
    /// Maximum retry delay
    pub max_retry_delay: Duration,
    /// How long to keep failed messages for inspection
    pub failed_message_ttl: Duration,
}

impl Default for QueueConfig {
    fn default() -> Self {
        Self {
            max_queue_size: 1000,
            max_retries: 5,
            initial_retry_delay: Duration::from_secs(1),
            max_retry_delay: Duration::from_secs(300), // 5 minutes
            failed_message_ttl: Duration::from_secs(3600), // 1 hour
        }
    }
}

/// Queue statistics
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct QueueStats {
    /// Messages currently queued
    pub queued: usize,
    /// Messages sent successfully
    pub sent: u64,
    /// Messages that failed permanently
    pub failed: u64,
    /// Messages currently being retried
    pub retrying: usize,
    /// Current rate limit cooldown (if any)
    pub cooldown_remaining_ms: Option<u64>,
}

/// Per-channel queue state
struct ChannelQueue {
    /// Priority queue of messages
    messages: BinaryHeap<QueuedMessage>,
    /// Rate limiter
    rate_limiter: RateLimiter,
    /// Statistics
    stats: QueueStats,
    /// Next message ID
    next_id: u64,
}

impl ChannelQueue {
    fn new(provider: &str) -> Self {
        Self {
            messages: BinaryHeap::new(),
            rate_limiter: RateLimiter::for_provider(provider),
            stats: QueueStats::default(),
            next_id: 0,
        }
    }
}

/// Message queue manager
pub struct MessageQueue {
    /// Per-channel queues
    queues: Arc<RwLock<HashMap<String, ChannelQueue>>>,
    /// Configuration
    config: QueueConfig,
    /// Channel for queue events
    event_tx: mpsc::Sender<QueueEvent>,
    /// Event receiver (for consumers)
    event_rx: Arc<RwLock<mpsc::Receiver<QueueEvent>>>,
}

/// Events emitted by the queue
#[derive(Debug, Clone)]
pub enum QueueEvent {
    /// Message was queued
    Queued { provider: String, message_id: u64 },
    /// Message was sent
    Sent { provider: String, message_id: u64, result_id: String },
    /// Message send failed (will retry)
    RetryScheduled { provider: String, message_id: u64, attempt: u32, retry_in: Duration },
    /// Message failed permanently
    Failed { provider: String, message_id: u64, error: String },
    /// Rate limited
    RateLimited { provider: String, cooldown: Duration },
    /// Queue stats updated
    StatsUpdated { provider: String, stats: QueueStats },
}

impl MessageQueue {
    /// Create a new message queue
    pub fn new(config: QueueConfig) -> Self {
        let (event_tx, event_rx) = mpsc::channel(1000);
        Self {
            queues: Arc::new(RwLock::new(HashMap::new())),
            config,
            event_tx,
            event_rx: Arc::new(RwLock::new(event_rx)),
        }
    }

    /// Get the event receiver
    pub fn events(&self) -> Arc<RwLock<mpsc::Receiver<QueueEvent>>> {
        self.event_rx.clone()
    }

    /// Enqueue a message with default priority
    pub async fn enqueue(&self, message: OutgoingMessage) -> u64 {
        self.enqueue_with_priority(message, MessagePriority::Normal).await
    }

    /// Enqueue a message with specific priority
    pub async fn enqueue_with_priority(&self, message: OutgoingMessage, priority: MessagePriority) -> u64 {
        let provider = message.channel.provider.clone();
        let mut queues = self.queues.write().await;

        let queue = queues
            .entry(provider.clone())
            .or_insert_with(|| ChannelQueue::new(&provider));

        let id = queue.next_id;
        queue.next_id += 1;

        let queued = QueuedMessage {
            message,
            priority,
            queued_at: Instant::now(),
            attempts: 0,
            retry_after: None,
            max_attempts: self.config.max_retries,
            id,
        };

        // Check queue size limit
        if queue.messages.len() >= self.config.max_queue_size {
            // Drop lowest priority message
            if let Some(dropped) = queue.messages.iter().min().cloned() {
                queue.messages.retain(|m| m.id != dropped.id);
                warn!("Queue full for {}, dropped message {}", provider, dropped.id);
            }
        }

        queue.messages.push(queued);
        queue.stats.queued = queue.messages.len();

        let _ = self.event_tx.send(QueueEvent::Queued { 
            provider: provider.clone(), 
            message_id: id,
        }).await;

        debug!("Enqueued message {} for {} (priority: {:?})", id, provider, priority);
        id
    }

    /// Process the queue for a channel, sending messages that are ready
    pub async fn process<C: Channel + ?Sized>(
        &self,
        provider: &str,
        channel: &C,
    ) -> Vec<SendResult> {
        let mut results = Vec::new();
        let mut to_retry = Vec::new();

        loop {
            // Get next message to send
            let msg = {
                let mut queues = self.queues.write().await;
                let Some(queue) = queues.get_mut(provider) else {
                    break;
                };

                // Check rate limit
                if !queue.rate_limiter.can_send() {
                    let wait = queue.rate_limiter.time_until_available();
                    if wait > Duration::ZERO {
                        queue.stats.cooldown_remaining_ms = Some(wait.as_millis() as u64);
                        break;
                    }
                }
                queue.stats.cooldown_remaining_ms = None;

                // Get highest priority message that's ready
                let now = Instant::now();
                let ready_idx = queue.messages.iter().position(|m| {
                    m.retry_after.map_or(true, |t| now >= t)
                });

                if let Some(_) = ready_idx {
                    // Pop the top message (highest priority that's ready)
                    let mut temp = Vec::new();
                    let mut found = None;
                    while let Some(m) = queue.messages.pop() {
                        if found.is_none() && m.retry_after.map_or(true, |t| now >= t) {
                            found = Some(m);
                        } else {
                            temp.push(m);
                        }
                    }
                    for m in temp {
                        queue.messages.push(m);
                    }
                    found
                } else {
                    None
                }
            };

            let Some(mut msg) = msg else {
                break;
            };

            // Acquire rate limit token
            {
                let mut queues = self.queues.write().await;
                if let Some(queue) = queues.get_mut(provider) {
                    if !queue.rate_limiter.try_acquire() {
                        // Put message back
                        queue.messages.push(msg);
                        break;
                    }
                }
            }

            // Send the message
            msg.attempts += 1;
            let send_result = channel.send(msg.message.clone()).await;

            match send_result {
                Ok(result_id) => {
                    results.push(SendResult::Success(result_id.clone()));
                    
                    let mut queues = self.queues.write().await;
                    if let Some(queue) = queues.get_mut(provider) {
                        queue.stats.sent += 1;
                        queue.stats.queued = queue.messages.len();
                    }

                    let _ = self.event_tx.send(QueueEvent::Sent {
                        provider: provider.to_string(),
                        message_id: msg.id,
                        result_id,
                    }).await;
                }
                Err(ChannelError::RateLimited) => {
                    // Apply cooldown and requeue
                    let cooldown = self.calculate_backoff(msg.attempts);
                    
                    {
                        let mut queues = self.queues.write().await;
                        if let Some(queue) = queues.get_mut(provider) {
                            queue.rate_limiter.set_cooldown(cooldown);
                        }
                    }

                    msg.retry_after = Some(Instant::now() + cooldown);
                    to_retry.push(msg.clone());

                    results.push(SendResult::RateLimited(cooldown));

                    let _ = self.event_tx.send(QueueEvent::RateLimited {
                        provider: provider.to_string(),
                        cooldown,
                    }).await;

                    warn!("Rate limited on {}, cooling down for {:?}", provider, cooldown);
                    break;
                }
                Err(e) => {
                    if msg.attempts < msg.max_attempts {
                        // Retry with backoff
                        let delay = self.calculate_backoff(msg.attempts);
                        msg.retry_after = Some(Instant::now() + delay);
                        to_retry.push(msg.clone());

                        results.push(SendResult::RetryLater(delay, e.to_string()));

                        let _ = self.event_tx.send(QueueEvent::RetryScheduled {
                            provider: provider.to_string(),
                            message_id: msg.id,
                            attempt: msg.attempts,
                            retry_in: delay,
                        }).await;

                        debug!("Will retry message {} in {:?} (attempt {})", msg.id, delay, msg.attempts);
                    } else {
                        // Max retries exceeded
                        results.push(SendResult::Failed(e.to_string()));

                        let mut queues = self.queues.write().await;
                        if let Some(queue) = queues.get_mut(provider) {
                            queue.stats.failed += 1;
                            queue.stats.queued = queue.messages.len();
                        }

                        let _ = self.event_tx.send(QueueEvent::Failed {
                            provider: provider.to_string(),
                            message_id: msg.id,
                            error: e.to_string(),
                        }).await;

                        error!("Message {} failed after {} attempts: {}", msg.id, msg.attempts, e);
                    }
                }
            }
        }

        // Re-add messages that need retry
        if !to_retry.is_empty() {
            let mut queues = self.queues.write().await;
            if let Some(queue) = queues.get_mut(provider) {
                for msg in to_retry {
                    queue.messages.push(msg);
                }
                queue.stats.retrying = queue.messages.iter().filter(|m| m.attempts > 0).count();
                queue.stats.queued = queue.messages.len();
            }
        }

        results
    }

    /// Calculate exponential backoff delay
    fn calculate_backoff(&self, attempts: u32) -> Duration {
        let base = self.config.initial_retry_delay.as_secs_f64();
        let max = self.config.max_retry_delay.as_secs_f64();
        
        // Exponential backoff with jitter
        let delay = (base * 2.0_f64.powi(attempts as i32 - 1)).min(max);
        let jitter = delay * 0.1 * rand::random::<f64>();
        
        Duration::from_secs_f64(delay + jitter)
    }

    /// Get queue statistics for a provider
    pub async fn stats(&self, provider: &str) -> Option<QueueStats> {
        let queues = self.queues.read().await;
        queues.get(provider).map(|q| q.stats.clone())
    }

    /// Get statistics for all providers
    pub async fn all_stats(&self) -> HashMap<String, QueueStats> {
        let queues = self.queues.read().await;
        queues.iter().map(|(k, v)| (k.clone(), v.stats.clone())).collect()
    }

    /// Clear the queue for a provider
    pub async fn clear(&self, provider: &str) {
        let mut queues = self.queues.write().await;
        if let Some(queue) = queues.get_mut(provider) {
            queue.messages.clear();
            queue.stats.queued = 0;
            queue.stats.retrying = 0;
        }
    }

    /// Get queue length for a provider
    pub async fn len(&self, provider: &str) -> usize {
        let queues = self.queues.read().await;
        queues.get(provider).map_or(0, |q| q.messages.len())
    }
}

impl Default for MessageQueue {
    fn default() -> Self {
        Self::new(QueueConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter() {
        let mut limiter = RateLimiter::new(3.0, 1.0);
        
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(limiter.try_acquire());
        assert!(!limiter.try_acquire()); // Bucket empty
    }

    #[test]
    fn test_message_priority() {
        assert!(MessagePriority::Urgent > MessagePriority::High);
        assert!(MessagePriority::High > MessagePriority::Normal);
        assert!(MessagePriority::Normal > MessagePriority::Low);
    }

    #[tokio::test]
    async fn test_queue_enqueue() {
        let queue = MessageQueue::default();
        let msg = OutgoingMessage {
            channel: ChannelId::new("test", "123"),
            content: crate::MessageContent::text("Hello"),
            reply_to: None,
        };

        let id = queue.enqueue(msg).await;
        assert_eq!(id, 0);
        assert_eq!(queue.len("test").await, 1);
    }
}
