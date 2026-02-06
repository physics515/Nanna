#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]
#![allow(clippy::module_name_repetitions)]

//! Nanna Daemon - Headless background service
//!
//! The daemon owns all state and exposes it via IPC to channel clients (GUI, CLI, etc.).
//! Every channel (including GUI) is a full control plane client with access to:
//! - Session management
//! - Memory browsing/editing
//! - Configuration
//! - Tool management
//! - Scheduler/cron

pub mod agent_service;
pub mod channels;
pub mod control;
pub mod health;
pub mod ipc;
pub mod llm_router;
pub mod persistence;
pub mod protocol;
pub mod server;
pub mod service;
pub mod session;
pub mod user_tools;
pub mod webhook;

#[cfg(windows)]
pub mod windows_service;

pub use ipc::{IpcServer, IpcServerConfig, ConnectionId};
pub use protocol::{Request, Response, Event, Action, SessionAction, MemoryAction, ConfigAction};
pub use server::{DaemonServer, DaemonConfig};
pub use session::{Session, SessionManager, SessionId};
pub use control::ControlPlane;
pub use health::{PidFile, PidFileError, HealthServer, HealthState, DEFAULT_HEALTH_PORT};
pub use webhook::{WebhookServer, WebhookConfig, WebhookEvent, WebhookMessage, DEFAULT_WEBHOOK_PORT};

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
