/**
 * Calm, truthful labels for daemon connection state.
 * Post-P16 the GUI is daemon-only — no embedded mode in production.
 * Keep words specific: "Daemon not reachable on 5149" beats "Disconnected".
 */

export type BackendMode = 'daemon' | 'disconnected' | 'embedded'

export interface BackendStatusLike {
  mode?: BackendMode | string | null
  connected?: boolean | null
  daemon_url?: string | null
  daemon_state?: string | null
  version?: string | null
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
 * Prefer daemon_state when disconnected (stopped/crashed/starting/reconnecting).
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
  if (state.includes('reconnect')) {
    return {
      short: 'Reconnecting',
      detail: endpoint ? 'to ' + endpoint : 'daemon',
      tooltip: 'Reconnecting to daemon' + (endpoint ? ' at ' + endpoint : ''),
      tone: 'warn',
      online: false,
    }
  }

  if (state.includes('start') || state === 'starting') {
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
      short: 'Daemon crashed',
      detail: endpoint ? 'last on ' + endpoint : '',
      tooltip:
        'Daemon crashed' +
        (endpoint ? ' (was ' + endpoint + ')' : '') +
        '. Check Logs, then restart.',
      tone: 'error',
      online: false,
    }
  }

  if (state.includes('stop') || state === 'not_started' || state === '') {
    return {
      short: 'Daemon offline',
      detail: endpoint ? 'not reachable on ' + endpoint : 'not reachable on 5149',
      tooltip:
        'Daemon not reachable on ' +
        (endpoint || '5149') +
        '. Start the daemon or wait for the sidecar.',
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
