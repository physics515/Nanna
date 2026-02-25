#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]
#![allow(clippy::module_name_repetitions)]

//! JavaScript/TypeScript scripting engine for Nanna
//!
//! Provides a unified interface for executing user-authored tools written in JS/TS.
//! Uses Boa (pure Rust) as the primary engine with Deno (V8) as a fallback.
//!
//! # Features
//!
//! - `boa` (default): Pure Rust JavaScript engine, lightweight (~5MB)
//! - `deno`: V8-based engine, full ECMAScript + TypeScript support (~30MB)
//! - `full`: Both engines with automatic fallback
//!
//! # Example
//!
//! ```ignore
//! use nanna_scripting::{ScriptEngine, ScriptedTool};
//!
//! let engine = ScriptEngine::new();
//!
//! let tool = ScriptedTool::new("greet", r#"
//!     export default {
//!         name: "greet",
//!         description: "Greet someone",
//!         execute({ name }) {
//!             return `Hello, ${name}!`;
//!         }
//!     }
//! "#);
//!
//! let result = engine.execute(&tool, json!({"name": "World"})).await?;
//! ```

mod engine;
mod tool;
mod bridge;

#[cfg(feature = "boa")]
mod boa_impl;

#[cfg(feature = "deno")]
mod deno_impl;

pub use engine::{ScriptEngine, EngineKind, ExecutionResult};
pub use tool::{ScriptedTool, ToolManifest, ToolPermissions, OutputTarget, extract_manifest};
pub use bridge::{NannaBridge, ServiceFn};

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ScriptError {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Execution error: {0}")]
    Execution(String),

    #[error("Timeout after {0}ms")]
    Timeout(u64),

    #[error("Permission denied: {0}")]
    Permission(String),

    #[error("TypeScript transpilation failed: {0}")]
    Transpile(String),

    #[error("Tool export invalid: {0}")]
    InvalidExport(String),

    #[error("Engine not available: {0}")]
    EngineNotAvailable(String),

    #[error("Bridge error: {0}")]
    Bridge(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ScriptError>;
