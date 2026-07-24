//! User-tool and skill authoring commands.
//!
//! User tools live in the daemon (it owns the registry + `user_tools` dir), so
//! CRUD forwards over IPC. Skills are files under the active workspace's
//! `skills/` directory and are edited directly on disk here; the daemon loads
//! them from its `tools_dir` at startup.

#[allow(clippy::wildcard_imports)]
use crate::*;

// =============================================================================
// User tool metadata (self-contained; mirrors the daemon's user-tool JSON)
// =============================================================================

const fn default_true() -> bool {
    true
}
fn default_language() -> String {
    "typescript".to_string()
}

/// Permissions block for a user tool (matches the daemon's serialized shape).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserToolPermissions {
    #[serde(default)]
    pub net: Vec<String>,
    #[serde(default)]
    pub read: Vec<String>,
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub env: bool,
    #[serde(default)]
    pub run: bool,
}

/// User-created tool metadata as surfaced to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserToolMeta {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub source: String,
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
    #[serde(default)]
    pub permissions: UserToolPermissions,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Parse the daemon's user-tool list (`{tools: [...]}`) into `UserToolMeta`.
fn parse_user_tools(result: &serde_json::Value) -> Result<Vec<UserToolMeta>, String> {
    serde_json::from_value(result.get("tools").cloned().unwrap_or(serde_json::json!([])))
        .map_err(|e| format!("Failed to parse daemon response: {e}"))
}

#[tauri::command]
pub async fn list_user_tools_cmd(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<UserToolMeta>, String> {
    let state_guard = state.read().await;
    let result = state_guard.backend.tool_list_user().await?;
    parse_user_tools(&result)
}

#[tauri::command]
pub async fn get_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<Option<UserToolMeta>, String> {
    let state_guard = state.read().await;
    let result = state_guard.backend.tool_list_user().await?;
    let tools = parse_user_tools(&result)?;
    Ok(tools.into_iter().find(|t| t.name == name))
}

#[tauri::command]
pub async fn get_tool_source(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.tool_get_source(&name).await
}

#[tauri::command]
pub async fn create_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    description: String,
    source: String,
    language: Option<String>,
    parameters: Option<serde_json::Value>,
) -> Result<UserToolMeta, String> {
    let _ = (language, parameters); // daemon derives language/params from source
    let state_guard = state.read().await;
    // Daemon tool_create uses (name, description, code, needs_shell).
    let result = state_guard.backend.tool_create(&name, &description, &source, None).await?;
    let tool = result.get("tool").cloned().unwrap_or(result);
    serde_json::from_value(tool).map_err(|e| format!("Failed to parse daemon response: {e}"))
}

#[tauri::command]
pub async fn update_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    description: Option<String>,
    source: Option<String>,
    parameters: Option<serde_json::Value>,
    enabled: Option<bool>,
) -> Result<UserToolMeta, String> {
    let _ = (parameters, enabled); // not exposed over the daemon tool_update action
    let state_guard = state.read().await;
    let result = state_guard
        .backend
        .tool_update(&name, description.as_deref(), source.as_deref(), None)
        .await?;
    let tool = result.get("tool").cloned().unwrap_or(result);
    serde_json::from_value(tool).map_err(|e| format!("Failed to parse daemon response: {e}"))
}

#[tauri::command]
pub async fn delete_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;
    state_guard.backend.tool_delete(&name).await?;
    Ok(())
}

#[tauri::command]
pub async fn test_user_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    source: String,
    input: std::collections::HashMap<String, serde_json::Value>,
) -> Result<String, String> {
    let state_guard = state.read().await;
    let input_value = serde_json::to_value(&input).map_err(|e| format!("Failed to serialize input: {e}"))?;
    let result = state_guard.backend.tool_test(&source, input_value).await?;
    result
        .get("output")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "Invalid response from daemon".to_string())
}

// =============================================================================
// Tool Listing Commands (all registered tools)
// =============================================================================

/// List all registered tools.
#[tauri::command]
pub async fn list_tools(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<Vec<ToolInfo>, String> {
    let state_guard = state.read().await;
    let result = state_guard.backend.tool_list().await?;
    let tools = result
        .get("tools")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    Some(ToolInfo {
                        name: t.get("name")?.as_str()?.to_string(),
                        description: t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        enabled: t.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                    })
                })
                .collect()
        })
        .ok_or("Failed to fetch tools from daemon")?;
    Ok(tools)
}

/// Get details of a specific tool.
#[tauri::command]
pub async fn get_tool(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<serde_json::Value, String> {
    let state_guard = state.read().await;
    state_guard.backend.daemon_request(serde_json::json!({
        "type": "tool",
        "action": "get",
        "name": name,
    })).await
}

// =============================================================================
// Skill Directory Commands (workspace-based tools, edited on disk)
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

/// Resolve the skills directory: the active workspace's `skills/`, else a
/// per-user data directory fallback.
pub(crate) async fn get_skills_path(state: &AppState) -> std::path::PathBuf {
    {
        let registry = state.workspaces.read().await;
        if let Some(ws) = registry.active() {
            return ws.path.join("skills");
        }
    }
    nanna_config::project_dirs()
        .map(|p| p.data_dir().join("skills"))
        .unwrap_or_else(|| std::path::PathBuf::from("skills"))
}

/// List all skills in the workspace `skills/` directory.
#[tauri::command]
pub async fn list_skills(
    state: State<'_, Arc<RwLock<AppState>>>,
) -> Result<SkillListResult, String> {
    let state_guard = state.read().await;
    let skills_path = get_skills_path(&state_guard).await;

    if !skills_path.exists() {
        if let Err(e) = std::fs::create_dir_all(&skills_path) {
            warn!("Failed to create skills directory: {e}");
        }
    }

    let discovered = nanna_tools::skills::discover_skills(&skills_path);

    let mut skills = Vec::new();
    for skill in discovered {
        let (skill_type, language) = match &skill.source {
            nanna_tools::skills::SkillSource::Script(p) => {
                let lang = p
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| if e == "ts" { "typescript" } else { "javascript" })
                    .unwrap_or("javascript");
                ("script".to_string(), Some(lang.to_string()))
            }
            nanna_tools::skills::SkillSource::Manifest(_) => ("manifest".to_string(), None),
        };

        let code_path = match &skill.source {
            nanna_tools::skills::SkillSource::Script(p)
            | nanna_tools::skills::SkillSource::Manifest(p) => p.clone(),
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

/// Create a new skill in the workspace.
#[tauri::command]
pub async fn create_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    skill_type: String,
    code: String,
) -> Result<SkillInfo, String> {
    let state_guard = state.read().await;

    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return Err("Skill name must be lowercase alphanumeric with underscores or hyphens".to_string());
    }

    let skills_path = get_skills_path(&state_guard).await;
    let skill_dir = skills_path.join(&name);
    if skill_dir.exists() {
        return Err(format!("Skill '{name}' already exists"));
    }
    std::fs::create_dir_all(&skill_dir).map_err(|e| format!("Failed to create skill directory: {e}"))?;

    let (filename, language) = match skill_type.as_str() {
        "manifest" => ("tool.yaml", None),
        "script" => ("tool.ts", Some("typescript".to_string())),
        _ => return Err(format!("Unknown skill type: {skill_type}")),
    };

    let code_path = skill_dir.join(filename);
    std::fs::write(&code_path, &code).map_err(|e| format!("Failed to write skill code: {e}"))?;

    info!("Created new skill: {name} at {}", skill_dir.display());

    Ok(SkillInfo {
        name,
        skill_type,
        language,
        path: skill_dir.display().to_string(),
        code: Some(code),
    })
}

/// Update an existing skill's code.
#[tauri::command]
pub async fn update_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
    code: String,
) -> Result<SkillInfo, String> {
    let state_guard = state.read().await;
    let skills_path = get_skills_path(&state_guard).await;

    let skill_dir = skills_path.join(&name);
    if !skill_dir.exists() {
        return Err(format!("Skill '{name}' not found"));
    }

    let code_files = ["tool.ts", "tool.js", "tool.yaml", "tool.yml"];
    let code_path = code_files
        .iter()
        .map(|f| skill_dir.join(f))
        .find(|p| p.exists())
        .ok_or_else(|| format!("No tool file found in skill '{name}'"))?;

    std::fs::write(&code_path, &code).map_err(|e| format!("Failed to update skill code: {e}"))?;

    let (skill_type, language) = match code_path.extension().and_then(|e| e.to_str()) {
        Some("yaml" | "yml") => ("manifest".to_string(), None),
        Some("ts") => ("script".to_string(), Some("typescript".to_string())),
        Some("js") => ("script".to_string(), Some("javascript".to_string())),
        _ => ("unknown".to_string(), None),
    };

    info!("Updated skill: {name}");

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
/// Hardens the delete path against symlink escapes: the skill name is sanitized
/// so `$skills_path/<name>` cannot resolve outside the skills root, and
/// symlinked skill directories (or symlink children) are refused.
#[tauri::command]
pub async fn delete_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    name: String,
) -> Result<(), String> {
    let state_guard = state.read().await;

    let name = name.trim();
    if name.is_empty() {
        return Err("Skill name must be non-empty".into());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!(
            "Invalid skill name '{name}': path separators and '..' are not allowed"
        ));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
        return Err(format!(
            "Invalid skill name '{name}': only alphanumeric, '-', '_', '.' are allowed"
        ));
    }

    let skills_path = get_skills_path(&state_guard).await;
    let skills_root = if skills_path.exists() {
        std::fs::canonicalize(&skills_path)
            .map_err(|e| format!("Failed to resolve skills directory: {e}"))?
    } else {
        return Err(format!("Skills directory {skills_path:?} does not exist"));
    };

    let skill_dir = skills_root.join(name);
    if !skill_dir.exists() {
        return Err(format!("Skill '{name}' not found"));
    }

    let meta = std::fs::symlink_metadata(&skill_dir)
        .map_err(|e| format!("Failed to stat skill '{name}': {e}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!("Refusing to delete skill '{name}': path is a symlink (escape risk)"));
    }
    if !meta.is_dir() {
        return Err(format!("Skill path '{name}' is not a directory"));
    }

    let canonical = std::fs::canonicalize(&skill_dir)
        .map_err(|e| format!("Failed to resolve skill '{name}': {e}"))?;
    if !canonical.starts_with(&skills_root) {
        return Err(format!(
            "Refusing to delete skill '{name}': resolved path escapes skills directory"
        ));
    }

    for entry in std::fs::read_dir(&canonical).map_err(|e| format!("Failed to read skill '{name}': {e}"))? {
        let entry = entry.map_err(|e| format!("Failed to read skill entry: {e}"))?;
        let ft = entry.file_type().map_err(|e| format!("Failed to stat skill entry: {e}"))?;
        if ft.is_symlink() {
            return Err(format!(
                "Refusing to delete skill '{name}': contains symlink child '{:?}'",
                entry.file_name()
            ));
        }
    }

    std::fs::remove_dir_all(&canonical).map_err(|e| format!("Failed to delete skill: {e}"))?;
    info!("Deleted skill: {name}");
    Ok(())
}

/// Test a skill with sample input.
#[tauri::command]
pub async fn test_skill(
    state: State<'_, Arc<RwLock<AppState>>>,
    code: String,
    skill_type: String,
    input: std::collections::HashMap<String, serde_json::Value>,
) -> Result<String, String> {
    match skill_type.as_str() {
        "script" => {
            // Run the script through the daemon's tool sandbox.
            let state_guard = state.read().await;
            let input_value =
                serde_json::to_value(&input).map_err(|e| format!("Failed to serialize input: {e}"))?;
            let result = state_guard.backend.tool_test(&code, input_value).await?;
            result
                .get("output")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .ok_or_else(|| "Invalid response from daemon".to_string())
        }
        "manifest" => match serde_yaml::from_str::<serde_json::Value>(&code) {
            Ok(_) => Ok("Manifest YAML is valid".to_string()),
            Err(e) => Err(format!("Invalid YAML: {e}")),
        },
        _ => Err(format!("Unknown skill type: {skill_type}")),
    }
}
