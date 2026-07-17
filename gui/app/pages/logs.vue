<template>
  <div class="h-full flex flex-col relative overflow-hidden">
    <!-- Header -->
    <div class="relative z-10 px-4 sm:px-6 py-4 border-b border-white/[0.04]">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-xl sm:text-2xl font-bold text-white/90">Logs</h1>
          <p class="text-xs sm:text-sm text-white/30 mt-0.5">Real-time output and diagnostics — embedded backend and daemon</p>
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
          <!-- Which sources are feeding this view. The GUI always logs itself, so
               'embedded' is always live; 'daemon' only when one is attached. -->
          <span class="text-cyan-300/60">&#x25cf; embedded</span>
          <span :class="daemonAttached ? 'text-violet-300/60' : 'text-white/25'">
            {{ daemonAttached ? '\u25cf daemon' : '\u25cb daemon (not attached)' }}
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
            <p class="text-white/30">No logs yet. Embedded and daemon logs will appear here.</p>
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
            'inline-block w-[4.5rem] text-center text-[10px] font-bold uppercase tracking-wide',
            'ml-2 px-1 py-px rounded border select-none',
            sourceOf(log) === 'daemon'
              ? 'text-violet-300/70 border-violet-400/20 bg-violet-400/[0.06]'
              : 'text-cyan-300/70 border-cyan-400/20 bg-cyan-400/[0.06]'
          ]">
            {{ sourceOf(log) }}
          </span>
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
import { computed, ref, watch, onMounted, onUnmounted, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ChevronDown, Trash2, Circle } from 'lucide-vue-next'

type LogSource = 'embedded' | 'daemon'

interface LogEntry {
  timestamp: string
  level: 'error' | 'warn' | 'info' | 'debug'
  target: string
  message: string
  // Absent only if an older daemon omitted it; rendered as 'daemon' in that case.
  source?: LogSource
}

/** How often the view re-reads the buffers while Live. */
const POLL_INTERVAL_MS = 1000
/** Matches MAX_LOG_LINES in the Rust command — asking for more returns no more. */
const LOG_LINE_LIMIT = 2000

const logs = ref<LogEntry[]>([])
const logsContainer = ref<HTMLElement | null>(null)
const autoScroll = ref(true)
const liveMode = ref(true)
const isLoading = ref(false)
const lastUpdate = ref<Date | null>(null)

// Lines at or before this timestamp were cleared by the user and must not come
// back on the next poll. Clearing hides history; it does not stop the tail.
const clearedBefore = ref<string | null>(null)

let pollTimer: ReturnType<typeof setInterval> | null = null

const { status: backendStatus } = useBackend()

// A daemon only contributes lines when the GUI is actually attached to one.
const daemonAttached = computed(() => backendStatus.value?.connected === true)

function sourceOf(log: LogEntry): LogSource {
  return log.source ?? 'daemon'
}

async function refreshLogs() {
  try {
    const fetched = await invoke<LogEntry[]>('get_daemon_logs', { limit: LOG_LINE_LIMIT })
    const cutoff = clearedBefore.value
    logs.value = cutoff ? fetched.filter(l => l.timestamp > cutoff) : fetched
    lastUpdate.value = new Date()
  } catch (e) {
    console.error('Failed to load logs:', e)
  }
}

onMounted(async () => {
  isLoading.value = true
  try {
    await refreshLogs()
    await nextTick()
    scrollToBottom()
  } finally {
    isLoading.value = false
  }

  // The backend has no log push channel, so Live is a poll of the merged
  // embedded + daemon buffers rather than a subscription.
  pollTimer = setInterval(async () => {
    if (!liveMode.value) return
    const wasAtBottom = autoScroll.value
    await refreshLogs()
    if (wasAtBottom) nextTick(() => scrollToBottom())
  }, POLL_INTERVAL_MS)
})

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer)
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
  // Remember how far we cleared, otherwise the next poll re-fetches the same
  // history straight back into the view.
  const newest = logs.value[logs.value.length - 1]
  if (newest) clearedBefore.value = newest.timestamp
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
