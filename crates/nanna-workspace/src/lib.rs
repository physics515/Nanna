#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! Workspace management for Nanna
//!
//! Provides directory-based agent context, including:
//! - Workspace detection and auto-discovery
//! - Context files (AGENTS.md, SOUL.md, USER.md, TOOLS.md, MEMORY.md)
//! - Per-workspace memory folders
//! - System prompt injection
//! - Workspace templates

mod discovery;
mod files;
mod manager;
mod templates;

pub use discovery::{discover_workspace, find_workspace_root, get_markers, is_workspace, WorkspaceMarker};
pub use files::{WorkspaceFile, WorkspaceFiles};
pub use manager::{Workspace, WorkspaceConfig, WorkspaceManager};
pub use templates::{create_from_template, list_templates, WorkspaceTemplate};

use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkspaceError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Workspace not found at {0}")]
    NotFound(PathBuf),
    #[error("Invalid workspace: {0}")]
    Invalid(String),
    #[error("Template not found: {0}")]
    TemplateNotFound(String),
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),
    #[error("Parse error: {0}")]
    Parse(String),
}

/// Well-known workspace file names
pub const AGENTS_FILE: &str = "AGENTS.md";
pub const SOUL_FILE: &str = "SOUL.md";
pub const USER_FILE: &str = "USER.md";
pub const TOOLS_FILE: &str = "TOOLS.md";
pub const MEMORY_FILE: &str = "MEMORY.md";
pub const IDENTITY_FILE: &str = "IDENTITY.md";
pub const HEARTBEAT_FILE: &str = "HEARTBEAT.md";
pub const BOOTSTRAP_FILE: &str = "BOOTSTRAP.md";

/// Memory subfolder name
pub const MEMORY_FOLDER: &str = "memory";

/// Workspace marker directory
pub const WORKSPACE_MARKER_DIR: &str = ".nanna";
