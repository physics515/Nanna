//! Skill manifest parsing (tool.yaml)

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use crate::{ToolError, OutputTarget};

/// Serde-friendly output target field for YAML manifests.
/// Accepts "memory" (default) or "context".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputTargetField {
    #[default]
    Memory,
    Context,
}

impl From<&OutputTargetField> for OutputTarget {
    fn from(field: &OutputTargetField) -> Self {
        match field {
            OutputTargetField::Memory => OutputTarget::Memory,
            OutputTargetField::Context => OutputTarget::Context,
        }
    }
}

/// Skill manifest loaded from tool.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    /// Tool name (required)
    pub name: String,

    /// Tool description (required)
    pub description: String,

    /// JSON Schema for parameters (optional)
    #[serde(default)]
    pub parameters: Option<Value>,

    /// Execution method
    #[serde(flatten)]
    pub execution: ExecutionMethod,

    /// Timeout in seconds (default: 30)
    #[serde(default = "default_timeout")]
    pub timeout: u64,

    /// Working directory (relative to skill dir, default: skill dir)
    #[serde(default)]
    pub workdir: Option<PathBuf>,

    /// Environment variables to set
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,

    /// Output routing: "memory" (default) or "context"
    #[serde(default)]
    pub output: OutputTargetField,
}

fn default_timeout() -> u64 {
    30
}

/// How to execute the tool
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMethod {
    /// Python script: `python {script} --json '{params}'`
    Python(PathBuf),
    
    /// Shell script: `bash {script} '{params}'` or `sh {script} '{params}'`
    Shell(PathBuf),
    
    /// Direct command with parameter substitution
    /// e.g., `./tool.exe --file {{file}} --count {{count}}`
    Command(String),
    
    /// Binary executable with JSON input on stdin
    Binary(PathBuf),
}

impl SkillManifest {
    /// Load manifest from a tool.yaml file
    pub fn from_file(path: &Path) -> Result<Self, ToolError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| ToolError::Io(e))?;
        
        let manifest: Self = serde_yaml::from_str(&content)
            .map_err(|e| ToolError::InvalidParams(format!("Invalid manifest: {e}")))?;
        
        Ok(manifest)
    }
    
    /// Get the script/binary path, resolved relative to the manifest directory
    pub fn resolve_executable(&self, manifest_dir: &Path) -> PathBuf {
        match &self.execution {
            ExecutionMethod::Python(p) => manifest_dir.join(p),
            ExecutionMethod::Shell(p) => manifest_dir.join(p),
            ExecutionMethod::Binary(p) => manifest_dir.join(p),
            ExecutionMethod::Command(_) => manifest_dir.to_path_buf(),
        }
    }
    
    /// Get the working directory, resolved relative to the manifest directory
    pub fn resolve_workdir(&self, manifest_dir: &Path) -> PathBuf {
        match &self.workdir {
            Some(wd) => manifest_dir.join(wd),
            None => manifest_dir.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_python_manifest() {
        let yaml = r#"
name: pdf_rotate
description: Rotate PDF pages by specified degrees
parameters:
  type: object
  properties:
    file:
      type: string
      description: Path to PDF file
    degrees:
      type: integer
      enum: [90, 180, 270]
  required: [file, degrees]
python: tool.py
timeout: 60
"#;
        let manifest: SkillManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(manifest.name, "pdf_rotate");
        assert_eq!(manifest.timeout, 60);
        assert!(matches!(manifest.execution, ExecutionMethod::Python(_)));
    }

    #[test]
    fn test_parse_command_manifest() {
        let yaml = r#"
name: image_resize
description: Resize an image
command: magick convert {{input}} -resize {{size}} {{output}}
"#;
        let manifest: SkillManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(manifest.name, "image_resize");
        assert!(matches!(manifest.execution, ExecutionMethod::Command(_)));
    }

    #[test]
    fn test_parse_shell_manifest() {
        let yaml = r#"
name: quick_hash
description: Hash a file
shell: hash.sh
"#;
        let manifest: SkillManifest = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(manifest.execution, ExecutionMethod::Shell(_)));
    }

    #[test]
    fn test_parse_output_context() {
        let yaml = r#"
name: my_lookup
description: Look something up
output: context
shell: lookup.sh
"#;
        let manifest: SkillManifest = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(manifest.output, OutputTargetField::Context));
        assert_eq!(OutputTarget::from(&manifest.output), OutputTarget::Context);
    }

    #[test]
    fn test_parse_output_defaults_to_memory() {
        let yaml = r#"
name: runner
description: Run a thing
shell: run.sh
"#;
        let manifest: SkillManifest = serde_yaml::from_str(yaml).unwrap();
        assert!(matches!(manifest.output, OutputTargetField::Memory));
        assert_eq!(OutputTarget::from(&manifest.output), OutputTarget::Memory);
    }
}
