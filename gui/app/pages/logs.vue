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
          <!-- Auto-scroll button (glass pill) -->
          <button
            @click="toggleAutoScroll"
            class="relative overflow-hidden px-3 py-1.5 rounded-full text-xs font-medium transition-all"
            :class="autoScroll ? 'text-cyan-300/90' : 'text-white/40 hover:text-white/60'"
            :style="autoScroll ? { ...btnGlassStyle, background: activeBtnBg } : btnGlassStyle"
            @mouseenter="() => { btnGlassEnter(); if (autoScroll) activeBtnEnter() }"
            @mouseleave="() => { btnGlassLeave(); activeBtnLeave() }"
          >
            <span v-if="autoScroll" class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
            <span class="relative z-10 flex items-center gap-1">
              <ChevronDown class="w-3.5 h-3.5" />
              {{ autoScroll ? 'Auto-scroll' : 'Manual' }}
            </span>
          </button>
          <!-- Clear button (glass pill) -->
          <button
            @click="clearLogs"
            class="relative overflow-hidden px-3 py-1.5 rounded-full text-xs font-medium text-white/40 hover:text-white/60 transition-all"
            :style="btnGlassStyle"
            @mouseenter="btnGlassEnter"
            @mouseleave="btnGlassLeave"
          >
            <span class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
            <span class="relative z-10 flex items-center gap-1">
              <Trash2 class="w-3.5 h-3.5" />
              Clear
            </span>
          </button>
          <!-- Live mode button (glass pill) -->
          <button
            @click="toggleLiveMode"
            class="relative overflow-hidden px-3 py-1.5 rounded-full text-xs font-medium transition-all"
            :class="liveMode ? 'text-emerald-300/90' : 'text-white/40 hover:text-white/60'"
            :style="liveMode ? { ...btnGlassStyle, background: liveBtnBg } : btnGlassStyle"
            @mouseenter="() => { btnGlassEnter(); if (liveMode) liveBtnEnter() }"
            @mouseleave="() => { btnGlassLeave(); liveBtnLeave() }"
          >
            <span v-if="liveMode" class="absolute inset-0 z-0" :style="{ background: btnMeshBg }"></span>
            <span class="relative z-10 flex items-center gap-1">
              <Circle class="w-2.5 h-2.5" :class="liveMode ? 'fill-current' : ''" />
              {{ liveMode ? 'Live' : 'Paused' }}
            </span>
          </button>
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
import { useSplatter } from '~/composables/useSplatter'
import { useGroundGlass } from '~/composables/useGroundGlass'

// Ground glass for all buttons (pill style)
const { meshBg: btnMeshBg, containerStyle: btnGlassStyle, onEnter: btnGlassEnter, onLeave: btnGlassLeave } = useGroundGlass({
  opacity: 2.0,
  sizes: ['55%', '50%', '45%'],
  lerpSpeed: 0.008,
  interval: 2000,
  blur: 8,
})

// Splatter for active auto-scroll button
const { splatterBg: activeBtnBg, onEnter: activeBtnEnter, onLeave: activeBtnLeave } = useSplatter({
  colors: ['34,211,238', '56,189,248', '20,184,220'],
  opacityRanges: [[0.08, 0.12], [0.06, 0.08], [0.04, 0.06]],
  sizes: ['65%', '60%', '50%'],
})

// Splatter for active live mode button
const { splatterBg: liveBtnBg, onEnter: liveBtnEnter, onLeave: liveBtnLeave } = useSplatter({
  colors: ['34,197,94', '22,163,74', '74,222,128'],
  opacityRanges: [[0.08, 0.12], [0.06, 0.08], [0.04, 0.06]],
  sizes: ['65%', '60%', '50%'],
})

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
