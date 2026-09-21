/**
 * Calm, truthful labels for daemon connection state.
 * Post-P16 the GUI is daemon-only — no embedded mode in production.
 * Keep words specific: "Daemon not reachable on 5149" beats "Disconnected".
 */

export type BackendMode = 'daemon' | 'disconnected' | 'embedded'

/** What went wrong, as `BackendStatus.last_error.kind` names it. */
export type DaemonFailureKind =
  | 'sidecar_unresolved'
  | 'spawn_failed'
  | 'exited_during_boot'
  | 'exited_after_ready'
  | 'health_check_failed'
  | 'restarts_exhausted'

/** The most recent reason the daemon failed to start or stay up. */
export interface DaemonFailure {
  kind: DaemonFailureKind | string
  /** Human-readable; for an exit during boot, the daemon's own last error line when it printed one. */
  message: string
  exit_code: number | null
  signal: number | null
  at_ms: number
}

export interface BackendStatusLike {
  mode?: BackendMode | string | null
  connected?: boolean | null
  daemon_url?: string | null
  daemon_state?: string | null
  version?: string | null
  /** Seconds the sidecar has been starting; set only while daemon_state is 'starting'. */
  starting_for_s?: number | null
  /** Not connected, and the app will try the daemon again shortly. */
  retrying?: boolean | null
  /**
   * An init is running right now. It is what tells "stopped, and about to
   * start" from "stopped, and nothing will start it".
   */
  init_in_progress?: boolean | null
  /** Why the daemon last failed to start or stay up; cleared once one is ready. */
  last_error?: DaemonFailure | null
}

/**
 * When a boot counts as slow and the label becomes "Still starting". This is
 * the same point at which the daemon manager logs its first "still starting"
 * line (`SLOW_START_NOTICE` in daemon_manager.rs). A normal boot opens the
 * port within a few seconds.
 */
export const SLOW_START_S = 30

/** Compact elapsed time: "42s", "2m 05s", "1h 03m". */
export function formatElapsed(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds))
  if (s < 60) return s + 's'
  const pad = (n: number) => String(n).padStart(2, '0')
  const m = Math.floor(s / 60)
  if (m < 60) return m + 'm ' + pad(s % 60) + 's'
  return Math.floor(m / 60) + 'h ' + pad(m % 60) + 'm'
}

export type ConnectionTone = 'ok' | 'warn' | 'error' | 'info' | 'loading'

export interface BackendLabel {
  /** Short badge text (status bar / BackendStatus). */
  short: string
  /** One-line detail under the badge. */
  detail: string
  /** Longer tooltip / title. */
  tooltip: string
  /** Visual tone for dots/badges. */
  tone: ConnectionTone
  /** True when the control plane can serve requests. */
  online: boolean
}

function endpointHint(url: string | null | undefined): string {
  if (!url) return '5149'
  return url.replace(/^wss?:\/\//, '')
}

/**
 * Map raw backend status into calm UI copy.
 * Prefer daemon_state when disconnected (stopped/crashed/starting/reconnecting),
 * and say so when the app is still retrying. A daemon that answers later is
 * attached without a restart.
 */
export function describeBackend(
  status: BackendStatusLike | null | undefined,
  loading = false,
): BackendLabel {
  if (loading) {
    return {
      short: 'Checking…',
      detail: '',
      tooltip: 'Checking daemon connection',
      tone: 'loading',
      online: false,
    }
  }

  if (!status) {
    return {
      short: 'Unknown',
      detail: 'Status unavailable',
      tooltip: 'Backend status unknown',
      tone: 'warn',
      online: false,
    }
  }

  const mode = String(status.mode ?? '').toLowerCase()
  const connected = status.connected === true
  const state = (status.daemon_state || '').toLowerCase()
  const endpoint = endpointHint(status.daemon_url)
  const retrying = status.retrying === true
  const retryNote = ' The app keeps trying and attaches as soon as a daemon answers.'

  // Production happy path: daemon + connected.
  if ((mode === 'daemon' || mode === '') && connected) {
    return {
      short: 'Daemon',
      detail: endpoint,
      tooltip: 'Attached to daemon at ' + endpoint,
      tone: 'ok',
      online: true,
    }
  }

  // Legacy label only — embedded mode was removed in P16.
  if (mode === 'embedded') {
    return {
      short: 'Legacy',
      detail: 'embedded mode retired',
      tooltip: 'Embedded mode was removed; attach a daemon on 5149.',
      tone: 'warn',
      online: false,
    }
  }

  // Reconnecting — not a hard disconnect for live log views.
  // Match only reconnect* / reconnecting; never bare "connect" (hits disconnected).
  // A running daemon with a retrying client is the same thing: the connection
  // dropped and the health check has not caught up yet.
  if (state.includes('reconnect') || (retrying && state === 'running')) {
    return {
      short: 'Reconnecting',
      detail: endpoint ? 'to ' + endpoint : 'daemon',
      tooltip: 'Reconnecting to daemon' + (endpoint ? ' at ' + endpoint : ''),
      tone: 'warn',
      online: false,
    }
  }

  // Exact match: 'not_started' (the frontend's own fallback when init itself
  // failed) contains "start" too, and it used to read as "Starting" for good.
  if (state === 'starting') {
    const secs = typeof status.starting_for_s === 'number' ? status.starting_for_s : null
    if (secs !== null && secs >= SLOW_START_S) {
      const elapsed = formatElapsed(secs)
      return {
        short: 'Still starting · ' + elapsed,
        detail: 'daemon booting for ' + elapsed,
        tooltip:
          'The daemon process is alive and still booting (' +
          elapsed +
          '). A cold model load, a migration or a slow provider can take minutes. ' +
          'The app attaches as soon as it opens ' +
          endpoint +
          '.',
        tone: 'info',
        online: false,
      }
    }
    return {
      short: 'Starting',
      detail: 'daemon sidecar',
      tooltip: 'Daemon sidecar is starting',
      tone: 'info',
      online: false,
    }
  }

  if (state.includes('crash')) {
    return {
      short: retrying ? 'Daemon crashed · retrying' : 'Daemon crashed',
      detail: endpoint ? 'last on ' + endpoint : '',
      tooltip:
        'Daemon crashed' +
        (endpoint ? ' (was ' + endpoint + ')' : '') +
        '. Check Logs, then restart.' +
        (retrying ? retryNote : ''),
      tone: 'error',
      online: false,
    }
  }

  if (state.includes('stop') || state === 'not_started' || state === '') {
    return {
      short: retrying ? 'Daemon offline · retrying' : 'Daemon offline',
      detail: endpoint ? 'not reachable on ' + endpoint : 'not reachable on 5149',
      tooltip:
        'Daemon not reachable on ' +
        (endpoint || '5149') +
        '. Start the daemon or wait for the sidecar.' +
        (retrying ? retryNote : ''),
      tone: 'error',
      online: false,
    }
  }

  return {
    short: 'Daemon offline',
    detail: state || (endpoint ? 'not reachable on ' + endpoint : 'not reachable'),
    tooltip: 'Daemon state: ' + (status.daemon_state || 'unknown'),
    tone: 'error',
    online: false,
  }
}

/** Status-bar compact label (Connected / specific offline reason). */
export function statusBarLabel(
  status: BackendStatusLike | null | undefined,
  apiKeySet: boolean,
): { text: string; tone: ConnectionTone } {
  const d = describeBackend(status)
  if (d.online) return { text: 'Connected', tone: 'ok' }
  if (!apiKeySet && (status == null || status.connected === false)) {
    // Prefer backend truth when we know daemon is down; else first-run hint.
    if (status && status.connected === false) {
      return { text: d.short, tone: d.tone }
    }
    return { text: 'No API key', tone: 'warn' }
  }
  return { text: d.short, tone: d.tone }
}
