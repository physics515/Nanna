<template>
  <div :class="['backend-status', modeClass]" :title="label.tooltip">
    <div class="flex items-center gap-2 px-3 py-1.5 rounded-lg min-h-8">
      <span class="status-indicator" aria-hidden="true">{{ icon }}</span>
      <div class="flex flex-col min-w-0">
        <span class="text-xs font-mono font-semibold truncate">{{ label.short }}</span>
        <span v-if="showDetail && label.detail" class="text-xs opacity-80 truncate">{{ label.detail }}</span>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { describeBackend, type BackendStatusLike } from '~/lib/backendLabels'

const props = defineProps<{
  showDetail?: boolean
}>()

const status = ref<BackendStatusLike | null>(null)
const loading = ref(true)
let pollInterval: number | null = null

const label = computed(() => describeBackend(status.value, loading.value))

const modeClass = computed(() => {
  switch (label.value.tone) {
    case 'ok': return 'status-daemon'
    case 'loading': return 'status-loading'
    case 'warn':
    case 'info': return 'status-warn'
    case 'error':
    default: return 'status-disconnected'
  }
})

const icon = computed(() => {
  switch (label.value.tone) {
    case 'loading': return '⏳'
    case 'ok': return '🔗'
    case 'warn':
    case 'info': return '↻'
    case 'error': return '🔌'
    default: return '❓'
  }
})

async function fetchStatus() {
  try {
    const data = await invoke<BackendStatusLike>('get_backend_status')
    status.value = data
  } catch (error) {
    console.error('Failed to fetch backend status:', error)
    status.value = null
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchStatus()
  pollInterval = window.setInterval(fetchStatus, 5000)
})

onUnmounted(() => {
  if (pollInterval !== null) clearInterval(pollInterval)
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

.status-warn {
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
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.8; transform: scale(1.05); }
}
</style>
