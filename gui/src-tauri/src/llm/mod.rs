//! LLM model-id parsing and client routing helpers.
//!
//! Truncation and summarization helpers used to live here for the GUI's
//! hand-rolled embedded agent loop; that loop now delegates to the daemon's
//! `AgentService` (see `crate::embedded`), which owns context budgeting and
//! tool-result summarization in `nanna-agent`.

pub mod routing;
