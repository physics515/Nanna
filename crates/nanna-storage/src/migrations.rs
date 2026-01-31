//! Database migrations

/// List of migrations to apply in order
pub const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", MIGRATION_001),
    ("002_memories", MIGRATION_002),
    ("003_config", MIGRATION_003),
    ("004_workspaces", MIGRATION_004),
];

const MIGRATION_001: &str = r"
-- Sessions table
CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL UNIQUE,
    channel TEXT NOT NULL,
    user_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT -- JSON
);

CREATE INDEX IF NOT EXISTS idx_sessions_channel ON sessions(channel);
CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);

-- Messages table
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    role TEXT NOT NULL, -- 'user', 'assistant', 'system', 'tool'
    content TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'text', -- 'text', 'tool_use', 'tool_result'
    tool_use_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    tokens_in INTEGER,
    tokens_out INTEGER,
    metadata TEXT, -- JSON
    FOREIGN KEY (session_id) REFERENCES sessions(session_id)
);

CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id);
CREATE INDEX IF NOT EXISTS idx_messages_created ON messages(created_at);
";

const MIGRATION_002: &str = r"
-- Vector memories table (for semantic search)
CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    memory_id TEXT NOT NULL UNIQUE,
    content TEXT NOT NULL,
    embedding BLOB, -- f32 vector as bytes
    embedding_model TEXT,
    session_id TEXT, -- optional, for session-specific memories
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT -- JSON
);

CREATE INDEX IF NOT EXISTS idx_memories_session ON memories(session_id);

-- Memory tags for filtering
CREATE TABLE IF NOT EXISTS memory_tags (
    memory_id TEXT NOT NULL,
    tag TEXT NOT NULL,
    PRIMARY KEY (memory_id, tag),
    FOREIGN KEY (memory_id) REFERENCES memories(memory_id)
);

CREATE INDEX IF NOT EXISTS idx_memory_tags_tag ON memory_tags(tag);
";

const MIGRATION_003: &str = r"
-- Key-value config storage
CREATE TABLE IF NOT EXISTS config (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Scheduled tasks / cron jobs
CREATE TABLE IF NOT EXISTS cron_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL UNIQUE,
    schedule TEXT NOT NULL, -- cron expression
    task TEXT NOT NULL, -- JSON task definition
    enabled INTEGER NOT NULL DEFAULT 1,
    last_run TEXT,
    next_run TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT -- JSON
);

CREATE INDEX IF NOT EXISTS idx_cron_next ON cron_jobs(next_run) WHERE enabled = 1;
";

const MIGRATION_004: &str = r"
-- Add workspace support to sessions
-- workspace_id is optional (NULL = global/no workspace)
ALTER TABLE sessions ADD COLUMN workspace_id TEXT;
ALTER TABLE sessions ADD COLUMN name TEXT;

CREATE INDEX IF NOT EXISTS idx_sessions_workspace ON sessions(workspace_id);

-- Workspace memory links (memory can belong to multiple workspaces)
CREATE TABLE IF NOT EXISTS workspace_memories (
    workspace_id TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    PRIMARY KEY (workspace_id, memory_id),
    FOREIGN KEY (memory_id) REFERENCES memories(memory_id)
);

CREATE INDEX IF NOT EXISTS idx_workspace_memories_workspace ON workspace_memories(workspace_id);
";
