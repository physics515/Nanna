<template>
  <div class="h-full flex flex-col bg-nanna-bg-deep">
    <!-- Header -->
    <div class="px-6 py-4 border-b border-white/[0.04] bg-nanna-bg-surface">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-2xl font-bold text-nanna-text">Daemon Logs</h1>
          <p class="text-sm text-nanna-text-muted mt-1">Real-time daemon output and diagnostics</p>
        </div>
        <div class="flex items-center gap-2">
          <button
            @click="toggleAutoScroll"
            :class="[
              'px-3 py-2 rounded-lg text-sm font-medium transition-colors',
              autoScroll
                ? 'bg-nanna-accent/20 text-nanna-accent'
                : 'bg-nanna-bg-elevated text-nanna-text-muted hover:text-nanna-text'
            ]"
          >
            <ChevronDown class="w-4 h-4 inline mr-1" />
            {{ autoScroll ? 'Auto-scroll' : 'Manual' }}
          </button>
          <button
            @click="clearLogs"
            class="px-3 py-2 rounded-lg text-sm font-medium bg-nanna-bg-elevated text-nanna-text-muted hover:text-nanna-text hover:bg-white/[0.06]/80 transition-colors"
          >
            <Trash2 class="w-4 h-4 inline mr-1" />
            Clear
          </button>
          <button
            @click="toggleLiveMode"
            :class="[
              'px-3 py-2 rounded-lg text-sm font-medium transition-colors',
              liveMode
                ? 'bg-nanna-success/20 text-nanna-success'
                : 'bg-nanna-bg-elevated text-nanna-text-muted hover:text-nanna-text'
            ]"
          >
            <Circle class="w-3 h-3 inline mr-1" :class="liveMode ? 'fill-current' : ''" />
            {{ liveMode ? 'Live' : 'Paused' }}
          </button>
        </div>
      </div>
    </div>

    <!-- Log viewer -->
    <div class="flex-1 overflow-hidden flex flex-col">
      <!-- Status bar -->
      <div class="px-6 py-2 bg-nanna-bg-elevated/30 border-b border-white/[0.04] text-xs text-nanna-text-muted flex items-center justify-between">
        <div class="flex items-center gap-4">
          <span>Total lines: {{ logs.length }}</span>
          <span v-if="backendStatus">
            Status: <span :class="backendStatus.connected ? 'text-nanna-success' : 'text-nanna-warning'">
              {{ backendStatus.connected ? 'Connected' : 'Disconnected' }}
            </span>
          </span>
          <span v-if="lastUpdate">Last update: {{ formatTime(lastUpdate) }}</span>
        </div>
        <div v-if="isLoading" class="flex items-center gap-2">
          <div class="w-2 h-2 bg-nanna-accent rounded-full animate-pulse"></div>
          <span>Loading...</span>
        </div>
      </div>

      <!-- Logs container -->
      <div 
        ref="logsContainer"
        class="flex-1 overflow-y-auto font-mono text-sm bg-nanna-bg-deep p-4 space-y-0"
      >
        <div v-if="logs.length === 0" class="text-nanna-text-muted text-center py-8">
          No logs yet. Daemon logs will appear here.
        </div>

        <div
          v-for="(log, idx) in logs"
          :key="idx"
          :class="[
            'py-1 px-2 rounded hover:bg-white/[0.06]/15 transition-colors group',
            log.level === 'error' ? 'text-nanna-error' :
            log.level === 'warn' ? 'text-nanna-warning' :
            log.level === 'info' ? 'text-nanna-success' :
            'text-nanna-text-dim'
          ]"
        >
          <span class="text-nanna-text-dim text-xs select-none">{{ log.timestamp }}</span>
          <span :class="[
            'inline-block w-8 text-xs font-bold ml-2 select-none',
            log.level === 'error' ? 'text-nanna-error' :
            log.level === 'warn' ? 'text-nanna-warning' :
            log.level === 'info' ? 'text-nanna-success' :
            'text-nanna-text-muted'
          ]">
            {{ log.level.toUpperCase().padEnd(5) }}
          </span>
          <span class="text-nanna-text-muted text-xs select-none">[{{ log.target }}]</span>
          <span class="ml-2 break-words">{{ log.message }}</span>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from 'vue'
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
    // Load initial logs
    const initialLogs = await invoke<LogEntry[]>('get_daemon_logs', { limit: 1000 })
    logs.value = initialLogs
    lastUpdate.value = new Date()
    
    // Scroll to bottom after initial load
    await nextTick()
    scrollToBottom()
  } catch (e) {
    console.error('Failed to load logs:', e)
  } finally {
    isLoading.value = false
  }

  // Listen for new log events
  unlistenLogs = await listen<LogEntry>('daemon-log', (event) => {
    if (liveMode.value) {
      logs.value.push(event.payload)
      lastUpdate.value = new Date()
      
      // Keep only last 10000 logs in memory
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
