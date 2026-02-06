# 05 — Tool System

## Feature Description

Nanna's tool system enables the AI to interact with the world — reading files, executing commands, searching the web, managing memory. It has three layers:

1. **Built-in tools**: Core capabilities shipped with Nanna (read_file, write_file, list_dir, exec, web_fetch, web_search, echo, remember, recall, reflect)
2. **User-authored tools**: Runtime-created tools via JavaScript/TypeScript scripts with the `nanna-scripting` engine
3. **Skills**: Workspace-local tools stored in a `skills/` directory as manifest+script pairs

Tools are registered in a central `ToolRegistry` and exposed to the LLM as function definitions. The agent loop executes tool calls in parallel and feeds results back to the model.

### Tool Aliases
Claude Code compatibility layer: `read` → `read_file`, `write` → `write_file`, `bash` → `exec`, `glob` → `list_dir`

## Current Implementation

### Architecture

```
ToolRegistry (nanna-tools)
├── Built-in tools (registered at startup)
│   ├── read_file, write_file, list_dir
│   ├── exec
│   ├── web_fetch, web_search
│   ├── echo
│   └── remember, recall, reflect (via MemoryServiceAdapter)
├── User tools (via UserToolManager)
│   └── UserToolWrapper → ScriptEngine (nanna-scripting/boa)
├── Meta-tools
│   ├── CreateToolTool (agent can create new tools)
│   └── ListUserToolsTool (agent can list custom tools)
└── Skills (workspace-local, loaded from skills/ directory)
```

### UserToolManager (tool_authoring.rs)
- Stores tools as `{name}.json` files in `~/.local/share/nanna/user_tools/`
- In-memory cache via `RwLock<HashMap<String, UserToolMeta>>`
- `load_all()`: Reads all JSON files from disk, registers with tool registry
- `create_tool()`: Validates, saves to disk, registers wrapper
- `update_tool()`: Mutates in-memory + saves to disk
- `delete_tool()`: Removes from disk + in-memory map
- `test_tool()`: Executes script with sample input without registering

### UserToolWrapper (tool_authoring.rs ~280)
- Implements `Tool` trait
- `definition()`: Builds `ToolDefinition` from metadata, parses parameters from JSON schema
- `execute()`: Creates `ScriptedTool` from source code, runs via `ScriptEngine`
- `timeout_secs()`: Returns 30 seconds (hardcoded)

### Skills System (lib.rs ~6487-6728)
- Directory-based: each skill is a folder with `manifest.yaml` + script file
- `list_skills()`: Scans skills directory, reads manifests
- `create_skill()`: Validates name (`[a-z0-9_-]`), creates directory + files
- `update_skill()`: Overwrites manifest/script
- `delete_skill()`: `remove_dir_all` on skill directory
- `test_skill()`: Executes script with sample input
- Skills path: workspace `.nanna/skills/` or global `~/.local/share/nanna/skills/`

### Tauri Commands
- `list_user_tools_cmd`, `get_user_tool`, `create_user_tool`, `update_user_tool`, `delete_user_tool`, `test_user_tool`
- `list_tools`, `get_tool` (all registered tools)
- `list_skills`, `create_skill`, `update_skill`, `delete_skill`, `test_skill`

## Issues & Bugs

### Critical
1. **Path traversal via tool names** (tool_authoring.rs): `save_tool` joins the tools directory with `{name}.json` without sanitizing the name. A name like `../../etc/cron.d/evil` writes outside the tools directory. User tools have NO name validation (skills validate with `[a-z0-9_-]` but tools don't).

2. **Deleted tools remain callable**: `delete_user_tool` and `delete_skill` remove from disk and in-memory maps but never unregister from `ToolRegistry`. No `ToolRegistry::unregister()` method exists. Deleted tools remain callable until app restart.

3. **Disabled tools still execute**: Setting `enabled: false` via `update_user_tool` updates the metadata but the tool remains in the registry. The `UserToolWrapper` doesn't check the `enabled` flag before execution.

### Moderate
4. **Blocking I/O on async runtime**: All file operations (`std::fs::read_dir`, `write`, `remove_file`, `create_dir_all`) are synchronous inside `async fn`s. Blocks the Tokio executor thread. Should use `tokio::fs` or `spawn_blocking`.

5. **In-memory/disk inconsistency**: `update_tool` mutates the in-memory HashMap entry before calling `save_tool`. If the write fails, memory and disk diverge. Should clone → mutate → save → swap.

6. **Silent registration failures**: `create_user_tool` uses `if let Ok(...)` around `create_tool_impl`, swallowing registration errors. Tool is saved to disk but never registered. User sees success for a non-functional tool.

7. **No duplicate name check**: `UserToolManager::create_tool` silently overwrites existing tools with the same name. No confirmation or error.

8. **`definition()` re-parses schema every call**: `UserToolWrapper::definition()` calls `parse_params_from_schema` on every invocation instead of caching parsed parameters at construction time.

9. **`parse_params_from_schema` drops non-string enums**: `"enum": [1, 2, 3]` silently produces empty enum values. Only string enums are handled.

### Minor
10. **Daemon/embedded feature parity**: `create_user_tool` in daemon mode drops `language`, `parameters`, and `enabled` fields — they're not sent in the daemon request.

11. **Skills have no daemon routing**: All skill commands always read local filesystem, even in daemon mode. Skills created on the GUI machine aren't visible to the daemon.

12. **No agent-callable update/delete tools**: `CreateToolTool` exists but there's no `UpdateToolTool` or `DeleteToolTool`. Agent can create tools but cannot fix or remove them.

13. **`remove_dir_all` in `delete_skill`**: No symlink check, no soft-delete. If `skills_path` is misconfigured, this could delete arbitrary directories.

14. **Inconsistent null handling**: `UserToolWrapper::execute` converts `Null` → `""`, but `test_tool` converts `Null` → `"(no output)"`.

## Improvement Suggestions

### High Priority
- **Sanitize tool names**: Enforce `^[a-z][a-z0-9_]{0,63}$` in `UserToolManager::create_tool` and `CreateToolTool`. Same validation as skills.
- **Implement `ToolRegistry::unregister(name)`**: Wire into `delete_user_tool`, `delete_skill`, and the `enabled: false` path. Without this, delete is fundamentally broken.
- **Switch to `tokio::fs`**: All file operations in async contexts should be non-blocking.
- **Clone-then-swap in `update_tool`**: Maintain disk/memory consistency on write failure.

### Medium Priority
- **Propagate registration errors**: In `create_user_tool`, return the error instead of swallowing it.
- **Cache parsed parameters**: Parse at `UserToolWrapper` construction, not on every `definition()` call.
- **Add `UpdateToolTool` and `DeleteToolTool`**: Let the agent manage its own tools.
- **Duplicate name detection**: Return error on name collision unless explicitly overwriting.
- **Check `enabled` flag in `execute()`**: Early return with error if tool is disabled.

### Future
- **Tool versioning**: Track tool versions, allow rollback to previous versions.
- **Tool marketplace**: Share tools between Nanna instances or publish to a community registry.
- **Tool sandboxing**: Run user scripts in a more restricted environment (currently limited by `ToolPermissions` but the scripting engine has broad access).
- **Tool dependencies**: Allow tools to declare dependencies on other tools or npm packages.
- **Tool analytics**: Track usage frequency, success rate, average execution time per tool.
- **Python tool support**: Currently only JavaScript/TypeScript via Boa engine. Python support would dramatically expand the tool ecosystem.
