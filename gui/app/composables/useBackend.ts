import { invoke } from '@tauri-apps/api/core'
import { computed, onMounted, onUnmounted, readonly, ref } from 'vue'
import { describeBackend, type BackendStatusLike } from '~/lib/backendLabels'

export interface BackendStatus extends BackendStatusLike {
  mode: 'daemon' | 'disconnected' | 'embedded'
  connected: boolean
  daemon_url: string | null
  daemon_state: string
  version: string
}

// Global reactive state (shared across callers)
const status = ref<BackendStatus | null>(null)
const initialized = ref(false)
const initializing = ref(false)
/**
 * The version the connected daemon reports about ITSELF.
 *
 * `status.version` is the GUI binary's own compile-time version — it describes
 * the process asking, not the one answering, so it agrees with the daemon only
 * when both came from the same build. Null means nothing is connected to ask.
 */
const daemonVersion = ref<string | null>(null)
let pollHandle: ReturnType<typeof setInterval> | null = null
let subscribers = 0

const POLL_MS = 2000

/** The version request in flight, if any. */
let versionAsk: Promise<void> | null = null
/** Bumped on every disconnect, so an answer from the daemon before it is dropped. */
let connectionEpoch = 0

/**
 * Ask the connected daemon for its version, without making anything wait.
 *
 * Fetched once per connection rather than on every poll: the version of a
 * running process cannot change under it. Never awaited by a status read: a
 * daemon too old to know `system.version` answers only after the client's
 * request timeout (300 s), and a status read that waited on it froze the
 * footer, the splash's release and the layout's first load for as long.
 */
function askDaemonVersion() {
  if (versionAsk !== null || daemonVersion.value !== null) return
  const epoch = connectionEpoch
  versionAsk = invoke<string | null>('get_daemon_version')
    .catch(() => null)
    .then((version) => {
      if (epoch === connectionEpoch) daemonVersion.value = version
    })
    .finally(() => {
      versionAsk = null
    })
}

async function refresh(): Promise<BackendStatus | null> {
  try {
    status.value = await invoke<BackendStatus>('get_backend_status')
    // Cleared on disconnect so a reconnect re-asks — the daemon that comes
    // back may be a different build, which is the whole reason for showing it.
    if (status.value?.connected) {
      askDaemonVersion()
    } else {
      connectionEpoch += 1
      daemonVersion.value = null
    }
    return status.value
  } catch (e) {
    console.error('Failed to get backend status:', e)
    return null
  }
}

/** The polls' read in flight, if any. */
let polling: Promise<BackendStatus | null> | null = null

/**
 * A polled status read: one at a time, across every poll. get_backend_status
 * waits on the app state's and the daemon manager's locks, so a read can take
 * as long as they are held. An interval that did not wait stacked another read on every
 * tick behind it. A tick that finds a read in flight gets that read.
 *
 * Only for polls: a caller that needs a read begun after something it did
 * (a restart) calls `refresh`.
 */
function poll(): Promise<BackendStatus | null> {
  if (polling === null) {
    polling = refresh().finally(() => {
      polling = null
    })
  }
  return polling
}

function ensurePolling() {
  if (pollHandle !== null) return
  pollHandle = setInterval(() => {
    void poll()
  }, POLL_MS)
}

function releasePolling() {
  if (subscribers > 0) return
  if (pollHandle !== null) {
    clearInterval(pollHandle)
    pollHandle = null
  }
}

export function useBackend() {
  /**
   * Initialize the backend — attach to the daemon control plane (P16: daemon-only).
   *
   * Runs again whenever it is called while disconnected. That is what the
   * Retry button relies on: `init_backend` restarts a sidecar that exited. It
   * used to return early once the first init had finished, so Retry did
   * nothing at all.
   */
  async function init(): Promise<'daemon' | 'disconnected'> {
    const connected = status.value?.mode === 'daemon' && status.value?.connected === true
    // One init at a time; the second caller gets the answer as it stands.
    if (initializing.value) return connected ? 'daemon' : 'disconnected'
    if (initialized.value && connected) return 'daemon'

    initializing.value = true

    try {
      const mode = await invoke<string>('init_backend')
      await refresh()
      initialized.value = true
      console.log(`Backend initialized: ${mode} mode`)
      if (mode === 'daemon' || status.value?.connected) return 'daemon'
      return 'disconnected'
    } catch (e) {
      console.error('Failed to initialize backend:', e)
      // P16: no embedded fallback — surface an honest offline state.
      status.value = {
        mode: 'disconnected',
        connected: false,
        daemon_url: 'ws://127.0.0.1:5149',
        daemon_state: 'not_started',
        version: 'unknown',
      }
      daemonVersion.value = null
      initialized.value = true
      return 'disconnected'
    } finally {
      initializing.value = false
      ensurePolling()
    }
  }

  // "Checking…" only until the first status arrives. The first init can last
  // as long as the daemon's boot, and the polled status is the one thing
  // that can say "Still starting · 2m 05s" in the meantime.
  const label = computed(() =>
    describeBackend(status.value, initializing.value && !initialized.value && status.value === null),
  )

  /** True when attached to a live daemon. */
  const isDaemon = computed(() => status.value?.mode === 'daemon' && status.value?.connected === true)

  /** @deprecated P16 removed embedded mode — always false in production. */
  const isEmbedded = computed(() => status.value?.mode === 'embedded')

  const isOnline = computed(() => label.value.online)

  onMounted(() => {
    subscribers += 1
    ensurePolling()
  })

  onUnmounted(() => {
    subscribers = Math.max(0, subscribers - 1)
    releasePolling()
  })

  return {
    status: readonly(status),
    daemonVersion: readonly(daemonVersion),
    initialized: readonly(initialized),
    initializing: readonly(initializing),
    isDaemon,
    isEmbedded,
    isOnline,
    label,
    init,
    refresh,
    poll,
  }
}
