# 06 — Workspace System

## Feature Description

Workspaces give Nanna project-specific context. Each workspace is a directory containing a `.nanna/` folder with identity files (SOUL.md, USER.md), guidelines (AGENTS.md, TOOLS.md), long-term memory (MEMORY.md), and daily memory logs. Sessions and memories can be scoped to a workspace, allowing Nanna to maintain separate knowledge bases per project.

### Workspace Structure
```
project-root/
└── .nanna/
    ├── SOUL.md      — Who the agent is in this workspace
    ├── USER.md      — About the human
    ├── AGENTS.md    — Workspace conventions and guidelines
    ├── TOOLS.md     — Local tool notes and preferences
    ├── MEMORY.md    — Curated long-term memory
    ├── skills/      — Workspace-local tools
    └── memory/
        ├── 2025-01-15.md  — Daily memory log
        └── 2025-01-16.md
```

### Context Injection
When a workspace is active, Nanna's system prompt includes the content of SOUL.md, USER.md, AGENTS.md, TOOLS.md, and MEMORY.md. This gives the agent project-specific personality, knowledge, and guidelines without explicit user instruction.

### Scoped Sessions & Memory
- Sessions created in a workspace context are tagged with the workspace ID
- Workspace sessions access global memories + that workspace's memories
- Global sessions access all memories across all workspaces

## Current Implementation

### WorkspaceInfo (lib.rs ~5155)
```rust
pub struct WorkspaceInfo {
    pub id: String,           // UUID
    pub name: String,         // Directory name
    pub path: String,         // Absolute path
    pub is_active: bool,      // Currently selected
    pub has_soul: bool,       // SOUL.md exists
    pub has_user: bool,       // USER.md exists
    pub has_agents: bool,     // AGENTS.md exists
    pub has_tools: bool,      // TOOLS.md exists
    pub has_memory: bool,     // MEMORY.md exists
}
```

### Workspace Commands
- `list_workspaces` — Lists all registered workspaces with metadata
- `open_workspace` — Opens a directory as workspace, registers in WorkspaceRegistry
- `set_active_workspace` — Sets the active workspace for context injection
- `clear_active_workspace` — Clears active workspace (back to global)
- `get_active_workspace` — Returns currently active workspace
- `get_workspace_context` — Returns concatenated context from all workspace files
- `reload_workspace` — Re-reads workspace files from disk
- `close_workspace` — Unregisters workspace
- `discover_workspaces_in_path` — Walks directory tree looking for `.nanna/` folders
- `find_workspace_root_from_path` — Walks up from a file to find the workspace root
- `save_workspace_file` — Writes content to a workspace file (SOUL.md, etc.)
- `append_workspace_memory` — Appends to today's daily memory file
- `get_workspace_recent_memory` — Reads recent daily memory entries
- `list_workspace_memory_files` — Lists all daily memory files
- `init_workspace` — Creates `.nanna/` folder with template files
- `read_workspace_file` — Reads a specific workspace file
- `check_workspace_validity` — Validates workspace structure

### Workspace Templates (lib.rs ~5515)
Default content for new workspaces:
- `SOUL_MD`: Identity template with personality, values, communication style
- `USER_MD`: User information template
- `AGENTS_MD`: Workspace conventions, project structure, workflow preferences
- `TOOLS_MD`: Local tool notes and preferences
- `MEMORY_MD`: Long-term memory curation template

### Context Injection Flow
1. Active workspace determined from session or global setting
2. `get_workspace_context` reads all `.md` files from `.nanna/`
3. Content concatenated and injected into system prompt
4. Recent daily memory entries appended
5. Total workspace context contributes to the 10k system prompt token budget

## Issues & Bugs

### Critical
1. **No workspace file size limits**: `get_workspace_context` reads and concatenates all workspace files without any size check. A large MEMORY.md or accumulated daily memory files could blow the system prompt token budget (10k tokens reserved). If workspace context exceeds the budget, it silently consumes tokens from the conversation history budget.

2. **Path traversal in `save_workspace_file`**: The `file` parameter is joined to the workspace path without validation. A value like `../../etc/passwd` could write outside the workspace directory. Need to validate that the resolved path is within the workspace.

### Moderate
3. **No workspace persistence**: The `WorkspaceRegistry` is in-memory. Registered workspaces are lost on app restart. Users must re-open workspaces each time. The active workspace setting persists in config, but the list of known workspaces does not.

4. **Daily memory files grow unbounded**: `append_workspace_memory` appends to `memory/YYYY-MM-DD.md` with no size limit. Over time, daily files accumulate and `get_workspace_recent_memory` reads the most recent N files, but there's no archival or rotation.

5. **No workspace conflict detection**: Two Nanna instances can open the same workspace simultaneously with no locking. Concurrent writes to workspace files could corrupt them.

6. **`discover_workspaces_in_path` has no depth limit**: Walks the entire directory tree, which could be extremely slow on large filesystems (e.g., scanning from `/` or a home directory with many nested projects).

7. **Template content is hardcoded**: Workspace templates are `const &str` in the source code. Users can't customize default templates for new workspaces.

8. **`check_workspace_validity` is informational only**: Returns a `WorkspaceValidityCheck` struct but doesn't enforce validity. Invalid workspaces can still be used.

### Minor
9. **Workspace name from directory name**: `WorkspaceInfo.name` is derived from the directory name via `file_name()`. Directories named with special characters or Unicode could produce unexpected names. No user-friendly rename capability.

10. **No workspace archival**: No way to archive/export a workspace's context and memories for backup or transfer to another machine.

11. **Memory files not included in workspace context by default**: Daily memory files in `memory/` are only included via `get_workspace_recent_memory`, not automatically in the main context injection. Users might expect all memory files to be part of the context.

## Improvement Suggestions

### High Priority
- **Workspace context budget**: Set a hard limit (e.g., 8k tokens) on total workspace context. Truncate or summarize if workspace files exceed the budget. Show a warning in the UI.
- **Path validation in `save_workspace_file`**: Canonicalize the resolved path and verify it starts with the workspace path. Reject traversal attempts.
- **Persist workspace list**: Save registered workspaces to config or a separate file. Restore on app startup.
- **Add depth limit to `discover_workspaces_in_path`**: Default max depth of 3-5 levels. Configurable.

### Medium Priority
- **Daily memory rotation**: Archive old daily memory files (e.g., consolidate weekly/monthly). Set a max file count for `get_workspace_recent_memory`.
- **Workspace locking**: Use a `.nanna/.lock` file to detect concurrent access. Warn or prevent.
- **Customizable templates**: Allow users to set default templates in config. Fall back to built-in templates.
- **Workspace rename**: Allow users to set a friendly name independent of the directory name.

### Future
- **Workspace sharing**: Export/import workspace context (`.nanna/` folder) as a portable archive.
- **Workspace inheritance**: Child workspaces that inherit from a parent (e.g., monorepo with shared SOUL.md but per-package AGENTS.md).
- **Auto-discovery on startup**: Scan common project directories for `.nanna/` folders and offer to register them.
- **Workspace health dashboard**: Show context token usage per file, memory file count, last modified dates.
- **Git integration**: Track `.nanna/` changes in git. Show diff of workspace context over time.
- **Workspace-specific model preferences**: Different workspaces could prefer different models (e.g., coding workspace uses Claude, writing workspace uses GPT-4).
