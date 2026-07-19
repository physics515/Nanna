//! User-authored skills (tools) system
//!
//! Supports two tiers of tool authoring:
//! 1. **Scripted (Boa/Deno)** - JS/TS tools with sandboxing (requires `scripting` feature)
//! 2. **Executable (Manifest)** - Python/shell/binary via tool.yaml
//!
//! # Directory Structure
//!
//! ```text
//! workspace/
//! └── skills/
//!     ├── weather/
//!     │   └── tool.ts          # Scripted (self-describing)
//!     ├── pdf-rotate/
//!     │   ├── tool.yaml        # Manifest
//!     │   └── tool.py          # Python impl
//!     └── quick-hash/
//!         ├── tool.yaml
//!         └── tool.sh          # Shell script
//! ```

mod manifest;
mod executable;
mod discovery;
pub mod defaults;

#[cfg(feature = "scripting")]
mod scripted;

pub use manifest::{SkillManifest, ExecutionMethod};
pub use executable::ExecutableTool;
pub use discovery::{discover_skills, SkillSource, DiscoveredSkill};

#[cfg(feature = "scripting")]
pub use scripted::ScriptedToolWrapper;

use crate::{Tool, ToolError};
#[cfg(feature = "scripting")]
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// Load a skill from a directory, returning a boxed Tool
pub async fn load_skill(skill_dir: &Path) -> Result<Arc<dyn Tool>, ToolError> {
    let source = discovery::detect_skill_source(skill_dir)?;

    match source {
        #[cfg(feature = "scripting")]
        SkillSource::Script(path) => {
            let tool = scripted::ScriptedToolWrapper::from_file(&path).await?;
            Ok(Arc::new(tool))
        }
        #[cfg(not(feature = "scripting"))]
        SkillSource::Script(_) => {
            Err(ToolError::ExecutionFailed(
                "Scripted tools require the 'scripting' feature".to_string()
            ))
        }
        SkillSource::Manifest(path) => {
            let tool = executable::ExecutableTool::from_manifest(&path)?;
            Ok(Arc::new(tool))
        }
    }
}

/// Load a skill from a directory with service functions attached.
///
/// `registry` is the weak handle scripted tools read their working directory
/// and session id from at execute time — without it, `Nanna.workdir()` is
/// null and relative paths in file/exec tools silently resolve to the HOME
/// directory instead of the active workspace.
#[cfg(feature = "scripting")]
pub async fn load_skill_with_services(
    skill_dir: &Path,
    services: &HashMap<String, nanna_scripting::ServiceFn>,
    registry: Option<std::sync::Weak<crate::ToolRegistry>>,
) -> Result<Arc<dyn Tool>, ToolError> {
    let source = discovery::detect_skill_source(skill_dir)?;

    match source {
        SkillSource::Script(path) => {
            let mut tool = scripted::ScriptedToolWrapper::from_file(&path).await?
                .with_services(services.clone());
            if let Some(registry) = registry {
                tool = tool.with_registry(registry);
            }
            Ok(Arc::new(tool))
        }
        SkillSource::Manifest(path) => {
            let tool = executable::ExecutableTool::from_manifest(&path)?;
            Ok(Arc::new(tool))
        }
    }
}

/// Load all skills from a skills directory
pub async fn load_skills_from_dir(skills_dir: &Path) -> Vec<Result<Arc<dyn Tool>, ToolError>> {
    let discovered = discover_skills(skills_dir);
    let mut results = Vec::with_capacity(discovered.len());

    for skill in discovered {
        results.push(load_skill(&skill.path).await);
    }

    results
}
