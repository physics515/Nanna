//! Database migrations

/// List of migrations to apply in order
pub const MIGRATIONS: &[(&str, &str)] = &[
    ("001_initial", MIGRATION_001),
    ("002_memories", MIGRATION_002),
    ("003_config", MIGRATION_003),
    ("004_workspaces", MIGRATION_004),
    ("005_job_runs", MIGRATION_005),
    ("006_model_stats", MIGRATION_006),
    ("007_tool_stats", MIGRATION_007),
    ("008_workspace_registry", MIGRATION_008),
    ("009_memory_fsrs", MIGRATION_009),
    ("010_checkpoints", MIGRATION_010),
    ("011_tasks", MIGRATION_011),
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

const MIGRATION_005: &str = r"
-- Job run history for tracking cron executions
CREATE TABLE IF NOT EXISTS job_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id TEXT NOT NULL,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    success INTEGER NOT NULL DEFAULT 0,
    output TEXT,
    error TEXT,
    duration_ms INTEGER,
    FOREIGN KEY (job_id) REFERENCES cron_jobs(job_id)
);

CREATE INDEX IF NOT EXISTS idx_job_runs_job ON job_runs(job_id);
CREATE INDEX IF NOT EXISTS idx_job_runs_started ON job_runs(started_at);

-- Add timezone support to cron jobs
ALTER TABLE cron_jobs ADD COLUMN timezone TEXT DEFAULT 'UTC';

-- Add target channel/session for cron results
ALTER TABLE cron_jobs ADD COLUMN target_channel TEXT;
ALTER TABLE cron_jobs ADD COLUMN target_session TEXT;
";

const MIGRATION_006: &str = r"
-- Model performance statistics (aggregated per model)
CREATE TABLE IF NOT EXISTS model_stats (
    model TEXT PRIMARY KEY,
    total_requests INTEGER NOT NULL DEFAULT 0,
    successful_requests INTEGER NOT NULL DEFAULT 0,
    failed_requests INTEGER NOT NULL DEFAULT 0,
    total_input_tokens INTEGER NOT NULL DEFAULT 0,
    total_output_tokens INTEGER NOT NULL DEFAULT 0,
    total_cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    total_cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_success_epoch_ms INTEGER NOT NULL DEFAULT 0,
    last_failure_epoch_ms INTEGER NOT NULL DEFAULT 0,
    tier_successes_simple INTEGER NOT NULL DEFAULT 0,
    tier_successes_medium INTEGER NOT NULL DEFAULT 0,
    tier_successes_complex INTEGER NOT NULL DEFAULT 0,
    tier_failures_simple INTEGER NOT NULL DEFAULT 0,
    tier_failures_medium INTEGER NOT NULL DEFAULT 0,
    tier_failures_complex INTEGER NOT NULL DEFAULT 0,
    escalations INTEGER NOT NULL DEFAULT 0,
    -- Recent latencies/throughput stored as JSON arrays (ring buffer)
    latencies_ms_json TEXT NOT NULL DEFAULT '[]',
    throughput_tps_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Per-request model observations (detailed log for analysis)
CREATE TABLE IF NOT EXISTS model_request_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    model TEXT NOT NULL,
    success INTEGER NOT NULL,
    latency_ms INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    tier TEXT,
    escalated INTEGER NOT NULL DEFAULT 0,
    session_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_model_request_log_model ON model_request_log(model);
CREATE INDEX IF NOT EXISTS idx_model_request_log_created ON model_request_log(created_at);
";

const MIGRATION_007: &str = r"
-- Tool performance statistics (aggregated per tool)
CREATE TABLE IF NOT EXISTS tool_stats (
    tool_name TEXT PRIMARY KEY,
    call_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    total_duration_ms INTEGER NOT NULL DEFAULT 0,
    last_called_epoch_ms INTEGER NOT NULL DEFAULT 0,
    -- Recent latencies stored as JSON array (ring buffer)
    latencies_ms_json TEXT NOT NULL DEFAULT '[]',
    -- Recent output sizes stored as JSON array (ring buffer)
    output_sizes_json TEXT NOT NULL DEFAULT '[]',
    -- Common errors as JSON: [{ message, count }]
    errors_json TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Per-invocation tool call log (time-series data for graphs)
CREATE TABLE IF NOT EXISTS tool_call_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tool_name TEXT NOT NULL,
    success INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    output_size INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    session_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_tool_call_log_tool ON tool_call_log(tool_name);
CREATE INDEX IF NOT EXISTS idx_tool_call_log_created ON tool_call_log(created_at);
CREATE INDEX IF NOT EXISTS idx_tool_call_log_tool_created ON tool_call_log(tool_name, created_at);

-- Hourly aggregated tool stats (for dashboard graphs over time)
CREATE TABLE IF NOT EXISTS tool_stats_hourly (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tool_name TEXT NOT NULL,
    hour TEXT NOT NULL,  -- ISO hour: '2026-03-11T15:00:00'
    call_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    total_duration_ms INTEGER NOT NULL DEFAULT 0,
    avg_duration_ms INTEGER NOT NULL DEFAULT 0,
    p95_duration_ms INTEGER NOT NULL DEFAULT 0,
    UNIQUE(tool_name, hour)
);

CREATE INDEX IF NOT EXISTS idx_tool_stats_hourly_hour ON tool_stats_hourly(hour);
CREATE INDEX IF NOT EXISTS idx_tool_stats_hourly_tool ON tool_stats_hourly(tool_name, hour);

-- Daily aggregated tool stats (for longer-term trends)
CREATE TABLE IF NOT EXISTS tool_stats_daily (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    tool_name TEXT NOT NULL,
    day TEXT NOT NULL,  -- ISO date: '2026-03-11'
    call_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    total_duration_ms INTEGER NOT NULL DEFAULT 0,
    avg_duration_ms INTEGER NOT NULL DEFAULT 0,
    p95_duration_ms INTEGER NOT NULL DEFAULT 0,
    UNIQUE(tool_name, day)
);

CREATE INDEX IF NOT EXISTS idx_tool_stats_daily_day ON tool_stats_daily(day);
CREATE INDEX IF NOT EXISTS idx_tool_stats_daily_tool ON tool_stats_daily(tool_name, day);
";

const MIGRATION_008: &str = r"
-- Workspace registry: persists registered workspaces across restarts
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    path TEXT NOT NULL UNIQUE,
    active INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_accessed TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_workspaces_path ON workspaces(path);
";

const MIGRATION_009: &str = r"
-- Add FSRS cognitive state columns and workspace scope to memories
ALTER TABLE memories ADD COLUMN workspace_id TEXT;
ALTER TABLE memories ADD COLUMN fsrs_stability REAL NOT NULL DEFAULT 1.0;
ALTER TABLE memories ADD COLUMN fsrs_difficulty REAL NOT NULL DEFAULT 5.0;
ALTER TABLE memories ADD COLUMN fsrs_last_access INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN fsrs_access_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE memories ADD COLUMN fsrs_importance REAL NOT NULL DEFAULT 1.0;
ALTER TABLE memories ADD COLUMN fsrs_storage_strength REAL NOT NULL DEFAULT 0.1;
ALTER TABLE memories ADD COLUMN fsrs_generation INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_memories_workspace ON memories(workspace_id);
";

const MIGRATION_010: &str = r"
-- Checkpoints for crash recovery (replaces checkpoint-{id}.json files)
CREATE TABLE IF NOT EXISTS checkpoints (
    session_id TEXT PRIMARY KEY,
    data TEXT NOT NULL,  -- JSON checkpoint payload
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
";

const MIGRATION_011: &str = r"
-- Agent-grade task store (P15): hierarchy, dependencies, acceptance criteria.
-- No triggers (the migration runner splits statements on semicolons) --
-- parent auto-completion, dependency cycle checks, and the activity log are
-- enforced in TaskRepository.
CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    parent_id INTEGER,
    scope TEXT NOT NULL DEFAULT 'session',
    scope_id TEXT,
    project TEXT,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    priority INTEGER NOT NULL DEFAULT 3,
    labels TEXT NOT NULL DEFAULT '[]',
    tool_scope TEXT NOT NULL DEFAULT '[]',
    due_at TEXT,
    recurrence TEXT,
    depends_on TEXT NOT NULL DEFAULT '[]',
    acceptance TEXT,
    assignee TEXT,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_tasks_scope ON tasks(scope, scope_id);
CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_id);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);

-- Append-only working notes: where a sub-agent leaves findings for its parent.
CREATE TABLE IF NOT EXISTS task_notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    author TEXT,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_task_notes_task ON task_notes(task_id);

-- Activity log: every transition with actor + timestamp (drift post-mortems).
CREATE TABLE IF NOT EXISTS task_activity (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL,
    actor TEXT,
    action TEXT NOT NULL,
    detail TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_task_activity_task ON task_activity(task_id);
";
