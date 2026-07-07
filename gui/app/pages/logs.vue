<template>
  <div class="h-full flex flex-col relative overflow-hidden">
    <!-- Header -->
    <div class="relative z-10 px-4 sm:px-6 py-4 border-b border-white/[0.04]">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-xl sm:text-2xl font-bold text-white/90">Daemon Logs</h1>
          <p class="text-xs sm:text-sm text-white/30 mt-0.5">Real-time daemon output and diagnostics</p>
        </div>
        <div class="flex items-center gap-2">
          <!-- Auto-scroll button -->
          <UiGlassButton pill size="xs" :color="autoScroll ? 'accent' : 'default'" @click="toggleAutoScroll">
            <ChevronDown class="w-3.5 h-3.5" />
            {{ autoScroll ? 'Auto-scroll' : 'Manual' }}
          </UiGlassButton>
          <!-- Clear button -->
          <UiGlassButton pill size="xs" @click="clearLogs">
            <Trash2 class="w-3.5 h-3.5" />
            Clear
          </UiGlassButton>
          <!-- Live mode button -->
          <UiGlassButton pill size="xs" :color="liveMode ? 'accent' : 'default'" @click="toggleLiveMode">
            <Circle class="w-2.5 h-2.5" :class="liveMode ? 'fill-current' : ''" />
            {{ liveMode ? 'Live' : 'Paused' }}
          </UiGlassButton>
        </div>
      </div>
    </div>

    <!-- Log viewer -->
    <div class="relative z-10 flex-1 overflow-hidden flex flex-col">
      <!-- Status bar -->
      <div class="px-4 sm:px-6 py-2 border-b border-white/[0.04] text-xs text-white/30 flex items-center justify-between">
        <div class="flex items-center gap-4">
          <span>{{ logs.length }} lines</span>
          <span v-if="backendStatus">
            <span :class="backendStatus.connected ? 'text-emerald-400/60' : 'text-amber-400/60'">
              {{ backendStatus.connected ? '\u25cf Connected' : '\u25cb Disconnected' }}
            </span>
          </span>
          <span v-if="lastUpdate">{{ formatTime(lastUpdate) }}</span>
        </div>
        <div v-if="isLoading" class="flex items-center gap-2">
          <div class="w-1.5 h-1.5 bg-cyan-400/60 rounded-full animate-pulse"></div>
          <span>Loading...</span>
        </div>
      </div>

      <!-- Logs container -->
      <div
        ref="logsContainer"
        class="flex-1 overflow-y-auto font-mono text-sm p-4 space-y-0"
      >
        <div v-if="logs.length === 0" class="flex items-center justify-center min-h-[300px]">
          <div class="text-center">
            <div class="text-4xl mb-3">&#x1f4cb;</div>
            <p class="text-white/30">No logs yet. Daemon logs will appear here.</p>
          </div>
        </div>

        <div
          v-for="(log, idx) in logs"
          :key="idx"
          class="py-1 px-2 rounded transition-colors group cursor-default"
          :class="[
            'hover:bg-white/[0.03]',
            log.level === 'error' ? 'text-red-400/80' :
            log.level === 'warn' ? 'text-amber-400/80' :
            log.level === 'info' ? 'text-emerald-400/70' :
            'text-white/40'
          ]"
        >
          <span class="text-white/20 text-xs select-none">{{ log.timestamp }}</span>
          <span :class="[
            'inline-block w-8 text-xs font-bold ml-2 select-none',
            log.level === 'error' ? 'text-red-400/80' :
            log.level === 'warn' ? 'text-amber-400/80' :
            log.level === 'info' ? 'text-emerald-400/60' :
            'text-white/30'
          ]">
            {{ log.level.toUpperCase().padEnd(5) }}
          </span>
          <span class="text-white/20 text-xs select-none">[{{ log.target }}]</span>
          <span class="ml-2 break-words">{{ log.message }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { ChevronDown, Trash2, Circle } from 'lucide-vue-next'

interface LogEntry {
  timestamp: string
  level: 'error' | 'warn' | 'info' | 'debug'
  target: string
  message: string
}

const logs = ref<LogEntry[]>([])
const logsContainer = ref<HTMLElement | null>(null)
const autoScroll = ref(true)
const liveMode = ref(true)
const isLoading = ref(false)
const lastUpdate = ref<Date | null>(null)

let unlistenLogs: UnlistenFn | null = null

const { status: backendStatus } = useBackend()

onMounted(async () => {
  isLoading.value = true
  try {
    const initialLogs = await invoke<LogEntry[]>('get_daemon_logs', { limit: 1000 })
    logs.value = initialLogs
    lastUpdate.value = new Date()
    await nextTick()
    scrollToBottom()
  } catch (e) {
    console.error('Failed to load logs:', e)
  } finally {
    isLoading.value = false
  }

  unlistenLogs = await listen<LogEntry>('daemon-log', (event) => {
    if (liveMode.value) {
      logs.value.push(event.payload)
      lastUpdate.value = new Date()

      if (logs.value.length > 10000) {
        logs.value = logs.value.slice(-10000)
      }

      if (autoScroll.value) {
        nextTick(() => scrollToBottom())
      }
    }
  })
})

onUnmounted(() => {
  if (unlistenLogs) {
    unlistenLogs()
  }
})

watch(autoScroll, async () => {
  if (autoScroll.value) {
    await nextTick()
    scrollToBottom()
  }
})

function scrollToBottom() {
  if (logsContainer.value) {
    logsContainer.value.scrollTop = logsContainer.value.scrollHeight
  }
}

function toggleAutoScroll() {
  autoScroll.value = !autoScroll.value
}

function toggleLiveMode() {
  liveMode.value = !liveMode.value
}

function clearLogs() {
  logs.value = []
}

function formatTime(date: Date): string {
  return date.toLocaleTimeString('en-US', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false
  })
}
</script>
