#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Nanna Daemon - Headless background service
//!
//! The daemon owns all state and exposes it via IPC to channel clients (GUI, CLI, etc.).
//! Every channel (including GUI) is a full control plane client with access to:
//! - Session management
//! - Memory browsing/editing
//! - Configuration
//! - Tool management
//! - Scheduler/cron

// `ActivityClock` now lives in `nanna-memory`, beside the dreaming code that
// reads it, so the control plane and `DreamingService` share **one** clock
// instead of each keeping a private notion of "last activity". Re-exported
// below because the daemon is where it is stamped.
pub mod agent_service;
pub mod channels;
pub mod control;
pub mod dream_summarizer;
pub mod embedding_router;
pub mod health;
pub mod ipc;
pub mod llm_router;
// `log_buffer` now lives in `nanna-core` so pure daemon clients (the GUI) can
// capture their own lines without linking this crate. Re-exported here so
// `crate::log_buffer::…` and `nanna_daemon::log_buffer::…` paths keep resolving.
pub use nanna_core::log_buffer;
pub mod log_file;
pub mod memory_adapter;
pub mod memory_persistence;
pub mod persistence;
pub mod protocol;
pub mod server;
pub mod service;
pub mod session;
pub mod tasks;
pub mod user_tools;
pub mod webhook;

#[cfg(windows)]
pub mod windows_service;

pub use ipc::{IpcServer, IpcServerConfig, ConnectionId, DEFAULT_IPC_PORT};
pub use protocol::{Request, Response, Event, Action, SessionAction, MemoryAction, ConfigAction};
pub use server::{DaemonServer, DaemonConfig};
pub use session::{Session, SessionManager, SessionId};
pub use control::ControlPlane;
pub use nanna_memory::ActivityClock;
pub use health::{PidFile, PidFileError, HealthServer, HealthState, DEFAULT_HEALTH_PORT};
pub use webhook::{WebhookServer, WebhookConfig, WebhookEvent, WebhookMessage, DEFAULT_WEBHOOK_PORT};
pub use log_buffer::{LogBuffer, LogEntry, LogSource};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum DaemonError {
    #[error("IPC error: {0}")]
    Ipc(String),
    
    #[error("Session error: {0}")]
    Session(String),
    
    #[error("Control error: {0}")]
    Control(String),
    
    #[error("Config error: {0}")]
    Config(String),
    
    #[error("Storage error: {0}")]
    Storage(String),
    
    #[error("Already running")]
    AlreadyRunning,
    
    #[error("Not running")]
    NotRunning,
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
