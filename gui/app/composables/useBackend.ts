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
let pollHandle: ReturnType<typeof setInterval> | null = null
let subscribers = 0

const POLL_MS = 2000

async function refresh(): Promise<BackendStatus | null> {
  try {
    status.value = await invoke<BackendStatus>('get_backend_status')
    return status.value
  } catch (e) {
    console.error('Failed to get backend status:', e)
    return null
  }
}

function ensurePolling() {
  if (pollHandle !== null) return
  pollHandle = setInterval(() => {
    void refresh()
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
   */
  async function init(): Promise<'daemon' | 'disconnected'> {
    if (initialized.value || initializing.value) {
      const mode = status.value?.mode
      if (mode === 'daemon' && status.value?.connected) return 'daemon'
      return 'disconnected'
    }

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
      initialized.value = true
      return 'disconnected'
    } finally {
      initializing.value = false
      ensurePolling()
    }
  }

  const label = computed(() => describeBackend(status.value, initializing.value && !initialized.value))

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
    initialized: readonly(initialized),
    initializing: readonly(initializing),
    isDaemon,
    isEmbedded,
    isOnline,
    label,
    init,
    refresh,
  }
}
