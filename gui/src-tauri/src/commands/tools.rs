//! User tool and skill authoring commands.

#[allow(clippy::wildcard_imports)]
use crate::*;

// =============================================================================
// User Tool Authoring Commands
// =============================================================================

#[tauri::command]
pub async fn list_user_tools_cmd(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<tool_authoring::UserToolMeta>, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.tool_list_user().await?;
        return serde_json::from_value(result.get("tools").cloned().unwrap_or(serde_json::json!([])))
            .map_err(|e| format!("Failed to parse daemon response: {}", e));
    }

    // Embedded mode
    Ok(state_guard.user_tools.list_tools().await)
}

#[tauri::command]
pub async fn get_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<Option<tool_authoring::UserToolMeta>, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode (get via tool_list_user and filter)
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.tool_list_user().await?;
        let tools: Vec<tool_authoring::UserToolMeta> = serde_json::from_value(
            result.get("tools").cloned().unwrap_or(serde_json::json!([]))
        ).map_err(|e| format!("Failed to parse daemon response: {}", e))?;
        return Ok(tools.into_iter().find(|t| t.name == name));
    }

    // Embedded mode
    Ok(state_guard.user_tools.get_tool(&name).await)
}

#[tauri::command]
pub async fn get_tool_source(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        return state_guard.backend.tool_get_source(&name).await;
    }

    // Embedded mode: not supported (no tools_dir available)
    Err("Tool source not available in embedded mode".to_string())
}

#[tauri::command]
pub async fn create_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    description: String,
    source: String,
    language: Option<String>,
    parameters: Option<serde_json::Value>,
) -> Result<tool_authoring::UserToolMeta, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        // Daemon tool_create uses (name, description, code, needs_shell)
        let result = state_guard.backend.tool_create(&name, &description, &source, None).await?;
        return serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse daemon response: {}", e));
    }

    // Embedded mode: Create the tool locally
    let meta = state_guard.user_tools.create_tool(
        name.clone(),
        description,
        source,
        language,
        parameters,
        None,
    ).await?;

    // Register it with the tool registry
    if let Ok(tool_impl) = state_guard.user_tools.create_tool_impl(&meta) {
        state_guard.tools.register_boxed(tool_impl).await;
        info!("Registered new user tool: {}", name);
    }

    Ok(meta)
}

#[tauri::command]
pub async fn update_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    description: Option<String>,
    source: Option<String>,
    parameters: Option<serde_json::Value>,
    enabled: Option<bool>,
) -> Result<tool_authoring::UserToolMeta, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        let result = state_guard.backend.tool_update(
            &name,
            description.as_deref(),
            source.as_deref(),
            None, // needs_shell not exposed in GUI
        ).await?;
        return serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse daemon response: {}", e));
    }

    // Embedded mode
    let meta = state_guard.user_tools.update_tool(
        &name,
        description,
        source,
        parameters,
        None,
        enabled,
    ).await?;

    // Re-register if enabled
    if meta.enabled {
        if let Ok(tool_impl) = state_guard.user_tools.create_tool_impl(&meta) {
            state_guard.tools.register_boxed(tool_impl).await;
            info!("Re-registered updated user tool: {}", name);
        }
    }

    Ok(meta)
}

#[tauri::command]
pub async fn delete_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        state_guard.backend.tool_delete(&name).await?;
        return Ok(());
    }

    // Embedded mode
    state_guard.user_tools.delete_tool(&name).await
}

#[tauri::command]
pub async fn test_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    source: String,
    input: std::collections::HashMap<String, serde_json::Value>,
) -> Result<String, String> {
    let state_guard = state.read().await;

    // Route through daemon if in daemon mode
    if state_guard.backend.is_daemon_mode().await {
        let input_value = serde_json::to_value(&input)
            .map_err(|e| format!("Failed to serialize input: {}", e))?;
        let result = state_guard.backend.tool_test(&source, input_value).await?;
        return result.get("output")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Invalid response from daemon".to_string());
    }

    // Embedded mode
    state_guard.user_tools.test_tool(&source, input).await
}

// =============================================================================
// Tool Listing Commands (all registered tools)
// =============================================================================

/// List all registered tools
#[tauri::command]
pub async fn list_tools(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ToolInfo>, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.tool_list().await {
            if let Some(tools_array) = result.get("tools").and_then(|v| v.as_array()) {
                let tools: Vec<ToolInfo> = tools_array.iter().filter_map(|t| {
                    Some(ToolInfo {
                        name: t.get("name")?.as_str()?.to_string(),
                        description: t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        enabled: t.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                    })
                }).collect();
                return Ok(tools);
            }
        }
        return Err("Failed to fetch tools from daemon".to_string());
    }

    // Embedded mode: get from tool registry
    let definitions = state_guard.tools.definitions().await;
    let tools: Vec<ToolInfo> = definitions.into_iter()
        .map(|t| ToolInfo {
            name: t.name,
            description: t.description,
            enabled: true,
        })
        .collect();

    Ok(tools)
}

/// Get details of a specific tool
#[tauri::command]
pub async fn get_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;

    // Route through daemon if available
    if state_guard.backend.is_daemon_mode().await {
        if let Ok(result) = state_guard.backend.daemon_request(serde_json::json!({
            "type": "tool",
            "action": "get",
            "name": name
        })).await {
            return Ok(result);
        }
        return Err("Failed to fetch tool from daemon".to_string());
    }

    // Embedded mode
    let definitions = state_guard.tools.definitions().await;
    if let Some(tool) = definitions.into_iter().find(|t| t.name == name) {
        Ok(serde_json::json!({
            "tool": {
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            }
        }))
    } else {
        Err(format!("Tool not found: {}", name))
    }
}

// =============================================================================
// Skill Directory Commands (workspace-based tools)
// =============================================================================

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillInfo {
    name: String,
    #[serde(rename = "type")]
    skill_type: String,
    language: Option<String>,
    path: String,
    code: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SkillListResult {
    skills: Vec<SkillInfo>,
    path: String,
}

/// Helper to get the skills directory path
pub(crate) async fn get_skills_path(state: &AppState) -> std::path::PathBuf {
    // Check for active workspace
    let registry = state.workspaces.read().await;
    if let Some(ws) = registry.active() {
        // Use the workspace path (not .nanna folder, but workspace root)
        return ws.path.join("skills");
    }
    drop(registry);

    // Fallback to config-based path
    directories::ProjectDirs::from("com", "clawd", "Nanna")
        .map(|p| p.data_dir().join("skills"))
        .unwrap_or_else(|| std::path::PathBuf::from("skills"))
}

/// List all skills in the workspace skills/ directory
#[tauri::command]
pub async fn list_skills(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<SkillListResult, String> {
    let state_guard = state.read().await;

    // Get skills directory from workspace or config
    let skills_path = get_skills_path(&state_guard).await;

    // Ensure directory exists
    if !skills_path.exists() {
        if let Err(e) = std::fs::create_dir_all(&skills_path) {
            warn!("Failed to create skills directory: {}", e);
        }
    }

    // Discover skills
    let discovered = nanna_tools::skills::discover_skills(&skills_path);

    let mut skills = Vec::new();
    for skill in discovered {
        let (skill_type, language) = match &skill.source {
            nanna_tools::skills::SkillSource::Script(p) => {
                let lang = p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| if e == "ts" { "typescript" } else { "javascript" })
                    .unwrap_or("javascript");
                ("script".to_string(), Some(lang.to_string()))
            }
            nanna_tools::skills::SkillSource::Manifest(_) => {
                ("manifest".to_string(), None)
            }
        };

        // Read the code
        let code_path = match &skill.source {
            nanna_tools::skills::SkillSource::Script(p) => p.clone(),
            nanna_tools::skills::SkillSource::Manifest(p) => p.clone(),
        };
        let code = std::fs::read_to_string(&code_path).ok();

        skills.push(SkillInfo {
            name: skill.name,
            skill_type,
            language,
            path: skill.path.display().to_string(),
            code,
        });
    }

    Ok(SkillListResult {
        skills,
        path: skills_path.display().to_string(),
    })
}

/// Create a new skill in the workspace
#[tauri::command]
pub async fn create_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    skill_type: String,
    code: String,
) -> Result<SkillInfo, String> {
    let state_guard = state.read().await;

    // Validate name (lowercase, underscores, alphanumeric)
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return Err("Skill name must be lowercase alphanumeric with underscores or hyphens".to_string());
    }

    // Get skills directory
    let skills_path = get_skills_path(&state_guard).await;

    // Create skill directory
    let skill_dir = skills_path.join(&name);
    if skill_dir.exists() {
        return Err(format!("Skill '{}' already exists", name));
    }
    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("Failed to create skill directory: {}", e))?;

    // Determine file name and language
    let (filename, language) = match skill_type.as_str() {
        "manifest" => ("tool.yaml", None),
        "script" => ("tool.ts", Some("typescript".to_string())),
        _ => return Err(format!("Unknown skill type: {}", skill_type)),
    };

    // Write the code file
    let code_path = skill_dir.join(filename);
    std::fs::write(&code_path, &code)
        .map_err(|e| format!("Failed to write skill code: {}", e))?;

    info!("Created new skill: {} at {}", name, skill_dir.display());

    Ok(SkillInfo {
        name,
        skill_type,
        language,
        path: skill_dir.display().to_string(),
        code: Some(code),
    })
}

/// Update an existing skill's code
#[tauri::command]
pub async fn update_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    code: String,
) -> Result<SkillInfo, String> {
    let state_guard = state.read().await;

    // Get skills directory
    let skills_path = get_skills_path(&state_guard).await;

    let skill_dir = skills_path.join(&name);
    if !skill_dir.exists() {
        return Err(format!("Skill '{}' not found", name));
    }

    // Find the code file
    let code_files = ["tool.ts", "tool.js", "tool.yaml", "tool.yml"];
    let code_path = code_files.iter()
        .map(|f| skill_dir.join(f))
        .find(|p| p.exists())
        .ok_or_else(|| format!("No tool file found in skill '{}'", name))?;

    // Write the updated code
    std::fs::write(&code_path, &code)
        .map_err(|e| format!("Failed to update skill code: {}", e))?;

    // Determine type from extension
    let (skill_type, language) = match code_path.extension().and_then(|e| e.to_str()) {
        Some("yaml") | Some("yml") => ("manifest".to_string(), None),
        Some("ts") => ("script".to_string(), Some("typescript".to_string())),
        Some("js") => ("script".to_string(), Some("javascript".to_string())),
        _ => ("unknown".to_string(), None),
    };

    info!("Updated skill: {}", name);

    Ok(SkillInfo {
        name,
        skill_type,
        language,
        path: skill_dir.display().to_string(),
        code: Some(code),
    })
}

/// Delete a skill.
///
/// Hardens the delete path against symlink escapes: the skill name is
/// sanitized so `$skills_path/<name>` cannot resolve outside the skills root.
/// Symlinked skill directories (or symlink children inside them) are refused
/// rather than followed with `remove_dir_all`.
#[tauri::command]
pub async fn delete_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    // Reject empty, path-separator-bearing, or parent-traversal names.
    let name = name.trim();
    if name.is_empty() {
        return Err("Skill name must be non-empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!(
            "Invalid skill name '{}': path separators and '..' are not allowed",
            name
        ));
    }
    // Keep the name to a conservative character class so it cannot smuggle
    // platform-specific path tricks (e.g. device names).
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(format!(
            "Invalid skill name '{}': only alphanumeric, '-', '_', '.' are allowed",
            name
        ));
    }

    let skills_path = get_skills_path(&state_guard).await;
    // Canonicalize the root when it already exists so later containment
    // checks are against the real path (not a caller's symlink-as-root).
    let skills_root = if skills_path.exists() {
        std::fs::canonicalize(&skills_path)
            .map_err(|e| format!("Failed to resolve skills directory: {}", e))?
    } else {
        return Err(format!("Skills directory {:?} does not exist", skills_path));
    };

    let skill_dir = skills_root.join(name);
    if !skill_dir.exists() {
        return Err(format!("Skill '{}' not found", name));
    }

    // Reject if the skill path itself is a symlink (avoid escaping via a
    // pre-existing link under the skills root).
    let meta = std::fs::symlink_metadata(&skill_dir)
        .map_err(|e| format!("Failed to stat skill '{}': {}", name, e))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "Refusing to delete skill '{}': path is a symlink (escape risk)",
            name
        ));
    }
    if !meta.is_dir() {
        return Err(format!("Skill path '{}' is not a directory", name));
    }

    // Containment check after canonicalize (defends against junction /
    // reparse races on Windows and symlink races on Unix between join and
    // delete).
    let canonical = std::fs::canonicalize(&skill_dir)
        .map_err(|e| format!("Failed to resolve skill '{}': {}", name, e))?;
    if !canonical.starts_with(&skills_root) {
        return Err(format!(
            "Refusing to delete skill '{}': resolved path escapes skills directory",
            name
        ));
    }

    // Refuse if any immediate child is a symlink. Soft-delete would be safer
    // long-term; for now a hard refuse keeps `remove_dir_all` off untrusted
    // trees.
    for entry in std::fs::read_dir(&canonical)
        .map_err(|e| format!("Failed to read skill '{}': {}", name, e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read skill entry: {}", e))?;
        let ft = entry
            .file_type()
            .map_err(|e| format!("Failed to stat skill entry: {}", e))?;
        if ft.is_symlink() {
            return Err(format!(
                "Refusing to delete skill '{}': contains symlink child '{:?}'",
                name,
                entry.file_name()
            ));
        }
    }

    std::fs::remove_dir_all(&canonical).map_err(|e| format!("Failed to delete skill: {}", e))?;

    info!("Deleted skill: {}", name);
    Ok(())
}

/// Test a skill with sample input
#[tauri::command]
pub async fn test_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    code: String,
    skill_type: String,
    input: std::collections::HashMap<String, serde_json::Value>,
) -> Result<String, String> {
    let state = state.read().await;

    match skill_type.as_str() {
        "script" => {
            // Use user_tools test for scripts
            state.user_tools.test_tool(&code, input).await
        }
        "manifest" => {
            // For manifest tools, we'd need to parse and execute
            // For now, just validate the YAML
            match serde_yaml::from_str::<serde_json::Value>(&code) {
                Ok(_) => Ok("Manifest YAML is valid".to_string()),
                Err(e) => Err(format!("Invalid YAML: {}", e)),
            }
        }
        _ => Err(format!("Unknown skill type: {}", skill_type)),
    }
}
