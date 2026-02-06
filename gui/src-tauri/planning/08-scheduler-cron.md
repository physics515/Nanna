# 08 — Scheduler & Cron System

## Feature Description

Nanna's scheduler enables autonomous operation through timed tasks. It supports cron-expression-based scheduling for three categories of work:

1. **Heartbeat tasks**: Periodic "check-in" where Nanna autonomously reviews context and takes action (default: every 30 minutes)
2. **Consolidation tasks**: Automatic memory consolidation/dreaming (default: every hour)
3. **Custom cron jobs**: User-defined scheduled tasks with natural language instructions

Tasks execute through the agent loop — the scheduler sends a prompt to Nanna, who processes it like any other message, with full tool access. Results can be routed to channels (Telegram, Discord, etc.).

### Cron Expression Format
Standard cron: `minute hour day-of-month month day-of-week`
Examples: `*/30 * * * *` (every 30 min), `0 9 * * 1-5` (weekdays at 9am)

## Current Implementation

### Data Structures (lib.rs ~6898)

```rust
struct CronJobInfo {
    id: String,
    name: String,
    schedule: String,          // Cron expression
    task: String,              // Natural language instruction
    enabled: bool,
    last_run: Option<String>,  // ISO timestamp
    next_run: Option<String>,  // ISO timestamp
    created_at: String,
    run_count: u64,
}

struct JobRunInfo {
    id: String,
    job_id: String,
    started_at: String,
    completed_at: Option<String>,
    status: String,            // "success", "failed", "running"
    result: Option<String>,    // Output text
    error: Option<String>,
}
```

### Commands
- `list_cron_jobs` — Lists all scheduled jobs with next run times
- `create_cron_job` — Creates a new job with cron expression and task prompt
- `update_cron_job` — Modifies schedule, task, or enabled state
- `set_cron_job_enabled` — Toggle job on/off
- `delete_cron_job` — Remove a job
- `delete_cron_jobs_by_name` — Remove jobs matching a name pattern
- `run_cron_job_now` — Execute a job immediately (bypasses schedule)
- `get_cron_job_history` — Retrieves past runs for a job
- `validate_cron_expression` — Checks if a cron string is valid

### Scheduler Integration (lib.rs ~7147, setup_state)
- Scheduler initialized at startup from `nanna_core::Scheduler`
- Heartbeat task added if `heartbeat_enabled` and no existing heartbeat job
- Consolidation task added with deduplication check (prevents duplicate "Memory Consolidation" jobs)
- Runtime toggles: `set_scheduler_enabled`, `set_heartbeat_enabled`, `set_heartbeat_interval`

### Task Execution Flow
1. Scheduler fires at cron time
2. Task prompt sent through agent loop (same as user message)
3. Agent has full tool access during execution
4. Result optionally routed to configured channel via `route_to_channel`
5. Run recorded in job history

### Channel Routing (lib.rs ~49)
`route_to_channel` function:
- Takes channel ID and message content
- Looks up channel configuration
- Sends via appropriate channel adapter (Telegram, Discord, etc.)
- Used by scheduler for delivering task results

## Issues & Bugs

### Critical
1. **No concurrent execution guard**: Nothing prevents the same cron job from running multiple times simultaneously. If a job takes longer than its interval (e.g., 30-min heartbeat takes 45 minutes), a second instance starts while the first is still running. Need a "running" lock per job.

2. **Task execution uses the chat session context**: When the scheduler runs a task, it goes through the agent loop which may use the currently active session's context. Scheduled tasks should have their own isolated sessions to avoid polluting user conversations.

### Moderate
3. **Heartbeat deduplication is name-based**: Checks for existing job named "Heartbeat" or "Memory Consolidation". If a user renames these jobs, duplicates will be created on next restart. Should use a `job_type` field instead of name matching.

4. **No job timeout**: Scheduled tasks have no wall-clock timeout. A stuck agent loop blocks the scheduler indefinitely. Should have a configurable timeout (default: 10 minutes).

5. **`run_cron_job_now` bypasses enabled check**: A disabled job can still be run manually. This might be intentional (useful for testing), but should be documented or have an explicit `force` parameter.

6. **No error retry for failed jobs**: If a job fails, it's recorded as failed but not retried. No configurable retry policy (retry count, backoff).

7. **Cron expression validation is separate from creation**: `validate_cron_expression` exists as a standalone command but `create_cron_job` also validates internally. Redundant validation, but more importantly, the validation in `create_cron_job` might have different behavior than the standalone validator.

### Minor
8. **Job history has no retention policy**: History grows indefinitely. No automatic cleanup of old run records.

9. **No timezone configuration per job**: All jobs presumably use the system timezone. No per-job timezone override for users in different timezones or wanting UTC-based scheduling.

10. **`delete_cron_jobs_by_name` is a broad operation**: Deletes all jobs matching a name. No confirmation, no preview of what will be deleted. Dangerous if names are similar.

11. **Scheduler state not persisted across restarts**: Jobs are re-created from config on startup, but custom user-created jobs may not survive restart (depends on whether they're saved to storage).

## Improvement Suggestions

### High Priority
- **Job execution lock**: Add a per-job mutex or "running" flag. Skip execution if the previous run hasn't completed. Log a warning.
- **Isolated scheduler sessions**: Create a dedicated session per scheduled job. Don't pollute user conversation history.
- **Job timeout**: Wrap task execution in `tokio::time::timeout`. Default 10 minutes, configurable per job.
- **Job type field**: Add `job_type: enum { Heartbeat, Consolidation, Custom }` instead of name-based deduplication.

### Medium Priority
- **Retry policy**: Add `max_retries` and `retry_delay` to job config. Retry failed jobs with exponential backoff.
- **History retention**: Auto-delete job history older than N days (configurable, default 30 days).
- **Per-job timezone**: Allow timezone override per job. Store as IANA timezone string.
- **Job persistence**: Ensure custom jobs survive app restart by saving to SQLite.

### Future
- **Job dependencies**: Allow jobs to depend on other jobs (run B after A completes successfully).
- **Job templates**: Pre-built job templates (daily standup summary, weekly report, code review reminder).
- **Natural language scheduling**: "Run this every morning at 9" → parse to cron expression.
- **Job dashboard**: Visual timeline of past and upcoming runs. Success/failure rates. Average execution time.
- **Conditional execution**: Jobs that only run if a condition is met (e.g., "only if there are new git commits").
- **Job output aggregation**: Collect outputs from multiple runs and summarize (e.g., weekly digest of daily heartbeats).