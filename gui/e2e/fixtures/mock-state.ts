/**
 * Shared types + pure helpers for the hermetic Tauri/daemon mock used by
 * Playwright web E2E. The browser-side state machine lives in tauri-mock.ts
 * (serialized into the page); this module exposes the options shape + a
 * few pure utilities for unit tests.
 */

export type SessionInfo = {
  id: string
  name: string
  created_at: string
  updated_at: string
  message_count: number
  workspace_id: string | null
  workspace_name: string | null
}

export type ChatMessage = {
  role: 'user' | 'assistant' | 'system'
  content: string
  timestamp?: string
}

export type LogEntry = {
  timestamp: string
  level: 'error' | 'warn' | 'info' | 'debug'
  target: string
  message: string
  source?: 'embedded' | 'daemon'
}

/** Why the daemon last failed to start or stay up (`BackendStatus.last_error`). */
export type MockDaemonFailure = {
  kind: string
  message: string
  exit_code: number | null
  signal: number | null
  at_ms: number
}

/**
 * A boot that has not attached yet, instead of the attached default. The
 * backend starts disconnected with these `BackendStatus` fields, so the app
 * shows the startup splash until `mock.attach()` connects it. `init_backend`
 * answers only once a boot in progress ends, as the real one does.
 */
export type MockBoot = {
  /** Default 'starting'. */
  daemon_state?: 'starting' | 'running' | 'crashed' | 'stopped' | 'stopping'
  /** Default 0 while starting, else null. */
  starting_for_s?: number | null
  retrying?: boolean
  /** Default: true while daemon_state is 'starting'. */
  init_in_progress?: boolean
  last_error?: MockDaemonFailure | null
  /** What `get_boot_log` returns, oldest first. */
  log?: Array<{ stream: 'stdout' | 'stderr'; line: string }>
}

/**
 * Options accepted by `installTauriMock` / `mock.gotoWithMock`.
 * Serialized into the page init script — keep JSON-safe.
 */
export type MockOptions = {
  /** When false, first-run / no-key empty state. Default true. */
  apiKeySet?: boolean
  /**
   * When false, backend starts disconnected and stopped, so the page opens on
   * the startup splash ("The daemon isn't running") until `mock.attach()`.
   * Default true.
   */
  backendConnected?: boolean
  /** Start mid-boot (or failed) behind the startup splash; overrides backendConnected. */
  boot?: MockBoot
  /** Seed sessions (default: one Welcome session). Pass [] for empty. */
  sessions?: Array<Partial<SessionInfo> & { id?: string; name?: string }>
  /** Seed message histories keyed by session id. */
  messages?: Record<string, ChatMessage[]>
  /** Auto-stream a mock reply on send_message. Default true. */
  streamAuto?: boolean
  /** Chunks used by the mock LLM stream. */
  streamChunks?: string[]
  /** Spec aliases */
  noKey?: boolean
  disconnected?: boolean
}

/** Normalize aliases (noKey/disconnected) into the canonical options. */
export function normalizeMockOptions(options: MockOptions = {}): MockOptions {
  const out: MockOptions = { ...options }
  if (options.noKey) out.apiKeySet = false
  if (options.disconnected) out.backendConnected = false
  if (options.apiKeySet !== undefined) out.apiKeySet = options.apiKeySet
  if (options.backendConnected !== undefined) out.backendConnected = options.backendConnected
  return out
}

export type MockScenario = MockOptions
