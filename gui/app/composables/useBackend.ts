import { invoke } from '@tauri-apps/api/core'
import { computed, onMounted, onUnmounted, readonly, ref } from 'vue'

export interface BackendStatus {
  mode: 'daemon' | 'embedded'
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
   * Initialize the backend - attempts daemon connection, falls back to embedded
   */
  async function init(): Promise<'daemon' | 'embedded'> {
    if (initialized.value || initializing.value) {
      return status.value?.mode || 'embedded'
    }

    initializing.value = true

    try {
      const mode = await invoke<string>('init_backend')
      await refresh()
      initialized.value = true
      console.log(`Backend initialized: ${mode} mode`)
      return mode as 'daemon' | 'embedded'
    } catch (e) {
      console.error('Failed to initialize backend:', e)
      // Default to embedded mode on error
      status.value = {
        mode: 'embedded',
        connected: false,
        daemon_url: null,
        daemon_state: 'not_started',
        version: 'unknown',
      }
      initialized.value = true
      return 'embedded'
    } finally {
      initializing.value = false
      ensurePolling()
    }
  }

  /**
   * Check if connected to daemon
   */
  const isDaemon = computed(() => status.value?.mode === 'daemon')

  /**
   * Check if running embedded
   */
  const isEmbedded = computed(() => status.value?.mode === 'embedded')

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
    init,
    refresh,
  }
}
