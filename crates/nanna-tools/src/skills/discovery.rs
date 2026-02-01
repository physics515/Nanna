//! Skill discovery - scan directories for user-authored tools

use crate::ToolError;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Source type of a discovered skill
#[derive(Debug, Clone)]
pub enum SkillSource {
    /// JavaScript/TypeScript script (tool.js or tool.ts)
    Script(PathBuf),
    /// Manifest-based executable (tool.yaml)
    Manifest(PathBuf),
}

/// A discovered skill before loading
#[derive(Debug, Clone)]
pub struct DiscoveredSkill {
    /// Skill directory path
    pub path: PathBuf,
    /// Skill name (directory name)
    pub name: String,
    /// Source type
    pub source: SkillSource,
}

/// Discover all skills in a directory
///
/// Looks for subdirectories containing either:
/// - `tool.ts` or `tool.js` (scripted)
/// - `tool.yaml` or `tool.yml` (manifest-based)
pub fn discover_skills(skills_dir: &Path) -> Vec<DiscoveredSkill> {
    let mut skills = Vec::new();
    
    if !skills_dir.exists() || !skills_dir.is_dir() {
        debug!(?skills_dir, "Skills directory does not exist");
        return skills;
    }
    
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(e) => {
            warn!(?skills_dir, error = %e, "Failed to read skills directory");
            return skills;
        }
    };
    
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        
        if !path.is_dir() {
            continue;
        }
        
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        
        // Skip hidden directories
        if name.starts_with('.') {
            continue;
        }
        
        // Detect skill source type
        if let Ok(source) = detect_skill_source(&path) {
            info!(name = %name, source = ?source, "Discovered skill");
            skills.push(DiscoveredSkill { path, name, source });
        } else {
            debug!(name = %name, "Directory is not a valid skill (no tool.ts/js/yaml)");
        }
    }
    
    skills
}

/// Detect the source type of a skill directory
pub fn detect_skill_source(skill_dir: &Path) -> Result<SkillSource, ToolError> {
    // Check for scripted tools first (higher priority)
    let ts_path = skill_dir.join("tool.ts");
    if ts_path.exists() {
        return Ok(SkillSource::Script(ts_path));
    }
    
    let js_path = skill_dir.join("tool.js");
    if js_path.exists() {
        return Ok(SkillSource::Script(js_path));
    }
    
    // Check for manifest-based tools
    let yaml_path = skill_dir.join("tool.yaml");
    if yaml_path.exists() {
        return Ok(SkillSource::Manifest(yaml_path));
    }
    
    let yml_path = skill_dir.join("tool.yml");
    if yml_path.exists() {
        return Ok(SkillSource::Manifest(yml_path));
    }
    
    Err(ToolError::NotFound(format!(
        "No tool.ts, tool.js, or tool.yaml found in {}",
        skill_dir.display()
    )))
}

/// Check if a directory is a valid skill
pub fn is_skill_dir(path: &Path) -> bool {
    path.is_dir() && detect_skill_source(path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_discover_skills() {
        let dir = tempdir().unwrap();
        let skills_dir = dir.path().join("skills");
        fs::create_dir(&skills_dir).unwrap();
        
        // Create a manifest-based skill
        let pdf_skill = skills_dir.join("pdf-rotate");
        fs::create_dir(&pdf_skill).unwrap();
        fs::write(pdf_skill.join("tool.yaml"), "name: pdf_rotate\ndescription: test\nshell: tool.sh").unwrap();
        
        // Create a scripted skill
        let weather_skill = skills_dir.join("weather");
        fs::create_dir(&weather_skill).unwrap();
        fs::write(weather_skill.join("tool.ts"), "export default { name: 'weather' }").unwrap();
        
        // Create a non-skill directory
        let random_dir = skills_dir.join("not-a-skill");
        fs::create_dir(&random_dir).unwrap();
        fs::write(random_dir.join("README.md"), "nothing here").unwrap();
        
        let discovered = discover_skills(&skills_dir);
        assert_eq!(discovered.len(), 2);
        
        let names: Vec<_> = discovered.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"pdf-rotate"));
        assert!(names.contains(&"weather"));
    }

    #[test]
    fn test_detect_skill_source() {
        let dir = tempdir().unwrap();
        
        // Test TypeScript detection
        let ts_skill = dir.path().join("ts-skill");
        fs::create_dir(&ts_skill).unwrap();
        fs::write(ts_skill.join("tool.ts"), "").unwrap();
        assert!(matches!(detect_skill_source(&ts_skill), Ok(SkillSource::Script(_))));
        
        // Test manifest detection
        let yaml_skill = dir.path().join("yaml-skill");
        fs::create_dir(&yaml_skill).unwrap();
        fs::write(yaml_skill.join("tool.yaml"), "").unwrap();
        assert!(matches!(detect_skill_source(&yaml_skill), Ok(SkillSource::Manifest(_))));
        
        // Test not found
        let empty_skill = dir.path().join("empty-skill");
        fs::create_dir(&empty_skill).unwrap();
        assert!(detect_skill_source(&empty_skill).is_err());
    }
}
