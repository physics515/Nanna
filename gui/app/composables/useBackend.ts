import { invoke } from '@tauri-apps/api/core'

export interface BackendStatus {
  mode: 'daemon' | 'embedded'
  connected: boolean
  daemon_url: string | null
  version: string
}

// Global reactive state
const status = ref<BackendStatus | null>(null)
const initialized = ref(false)
const initializing = ref(false)

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
        version: 'unknown',
      }
      initialized.value = true
      return 'embedded'
    } finally {
      initializing.value = false
    }
  }
  
  /**
   * Refresh the backend status
   */
  async function refresh(): Promise<BackendStatus | null> {
    try {
      status.value = await invoke<BackendStatus>('get_backend_status')
      return status.value
    } catch (e) {
      console.error('Failed to get backend status:', e)
      return null
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
