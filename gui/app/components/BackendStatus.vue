<template>
  <div :class="['backend-status', modeClass]" :title="tooltip">
    <div class="flex items-center gap-2 px-3 py-1.5 rounded-lg backdrop-blur-sm">
      <span class="status-indicator">{{ icon }}</span>
      <div class="flex flex-col">
        <span class="text-xs font-mono font-semibold">{{ modeLabel }}</span>
        <span v-if="showDetail" class="text-xs opacity-70">{{ detail }}</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'

interface BackendStatusData {
  mode: 'daemon' | 'embedded'
  connected: boolean
  daemon_url: string | null
  daemon_state: string
  version: string
}

const props = defineProps<{
  showDetail?: boolean
}>()

const status = ref<BackendStatusData | null>(null)
const loading = ref(true)
let pollInterval: number | null = null

const modeClass = computed(() => {
  if (!status.value) return 'status-loading'
  if (status.value.mode === 'daemon' && status.value.connected) {
    return 'status-daemon'
  }
  if (status.value.mode === 'embedded') {
    return 'status-embedded'
  }
  return 'status-disconnected'
})

const icon = computed(() => {
  if (loading.value) return '⏳'
  if (!status.value) return '❓'
  if (status.value.mode === 'daemon' && status.value.connected) return '🔗'
  if (status.value.mode === 'embedded') return '📱'
  return '🔌'
})

const modeLabel = computed(() => {
  if (loading.value) return 'Loading...'
  if (!status.value) return 'Unknown'
  if (status.value.mode === 'daemon' && status.value.connected) {
    return 'DAEMON'
  }
  if (status.value.mode === 'embedded') {
    return 'EMBEDDED'
  }
  return 'DISCONNECTED'
})

const detail = computed(() => {
  if (!status.value) return ''
  if (status.value.mode === 'daemon' && status.value.daemon_url) {
    return status.value.daemon_url.replace('ws://', '')
  }
  if (status.value.mode === 'embedded') {
    return 'Direct mode'
  }
  return status.value.daemon_state
})

const tooltip = computed(() => {
  if (!status.value) return 'Backend status unknown'
  if (status.value.mode === 'daemon' && status.value.connected) {
    return `Connected to daemon at ${status.value.daemon_url}\nAll requests routed through background service`
  }
  if (status.value.mode === 'embedded') {
    return 'Running in embedded mode\nDirect LLM/tool access (daemon unavailable)'
  }
  return `Daemon state: ${status.value.daemon_state}`
})

async function fetchStatus() {
  try {
    const data = await invoke<BackendStatusData>('get_backend_status')
    status.value = data
    loading.value = false
  } catch (error) {
    console.error('Failed to fetch backend status:', error)
    loading.value = false
  }
}

onMounted(() => {
  fetchStatus()
  // Poll every 5 seconds
  pollInterval = window.setInterval(fetchStatus, 5000)
})

onUnmounted(() => {
  if (pollInterval !== null) {
    clearInterval(pollInterval)
  }
})
</script>

<style scoped>
.backend-status {
  font-family: 'JetBrains Mono', 'Fira Code', monospace;
  transition: all 0.3s ease;
}

.status-loading {
  background: linear-gradient(135deg, rgba(148, 163, 184, 0.2), rgba(148, 163, 184, 0.1));
  border: 1px solid rgba(148, 163, 184, 0.3);
  color: #94a3b8;
}

.status-daemon {
  background: linear-gradient(135deg, rgba(34, 197, 94, 0.25), rgba(34, 197, 94, 0.15));
  border: 1px solid rgba(34, 197, 94, 0.4);
  color: #4ade80;
  box-shadow: 0 0 10px rgba(34, 197, 94, 0.2);
}

.status-embedded {
  background: linear-gradient(135deg, rgba(234, 179, 8, 0.25), rgba(234, 179, 8, 0.15));
  border: 1px solid rgba(234, 179, 8, 0.4);
  color: #fbbf24;
  box-shadow: 0 0 10px rgba(234, 179, 8, 0.2);
}

.status-disconnected {
  background: linear-gradient(135deg, rgba(239, 68, 68, 0.25), rgba(239, 68, 68, 0.15));
  border: 1px solid rgba(239, 68, 68, 0.4);
  color: #f87171;
  box-shadow: 0 0 10px rgba(239, 68, 68, 0.2);
}

.status-indicator {
  font-size: 1rem;
  line-height: 1;
}

.status-daemon .status-indicator {
  animation: pulse 2s ease-in-out infinite;
}

@keyframes pulse {
  0%, 100% {
    opacity: 1;
    transform: scale(1);
  }
  50% {
    opacity: 0.8;
    transform: scale(1.05);
  }
}
</style>
