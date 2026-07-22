<template>
  <div class="h-full flex flex-col relative overflow-hidden">
    <!-- Header -->
    <div class="relative z-10 px-4 sm:px-6 py-4 border-b border-white/[0.04]">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-xl sm:text-2xl font-bold text-white/90">Logs</h1>
          <p class="text-xs sm:text-sm text-white/30 mt-0.5">Real-time output and diagnostics from the GUI and attached daemon</p>
        </div>
        <div class="flex items-center gap-2">
          <!-- Auto-scroll button -->
          <UiGlassButton pill size="xs" :color="autoScroll ? 'accent' : 'default'" @click="toggleAutoScroll">
            <ChevronDown class="w-3.5 h-3.5" />
            {{ autoScroll ? 'Auto-scroll' : 'Manual' }}
          </UiGlassButton>
          <!-- Copy all logs button -->
          <UiGlassButton
            pill
            size="xs"
            :disabled="logs.length === 0"
            :aria-label="copyLabel"
            :title="copyLabel"
            @click="copyAllLogs"
          >
            <Copy class="w-3.5 h-3.5" />
            {{ copyLabel }}
          </UiGlassButton>
          <!-- Clear button -->
          <UiGlassButton pill size="xs" aria-label="Clear" title="Clear" @click="clearLogs">
            <Trash2 class="w-3.5 h-3.5" />
            Clear
          </UiGlassButton>
          <!-- Live mode button -->
          <UiGlassButton
            pill
            size="xs"
            :color="liveMode ? 'accent' : 'default'"
            :aria-label="liveMode ? 'Live' : 'Paused'"
            :title="liveMode ? 'Live' : 'Paused'"
            @click="toggleLiveMode"
          >
            <Circle class="w-2.5 h-2.5" :class="liveMode ? 'fill-current' : ''" />
            {{ liveMode ? 'Live' : 'Paused' }}
          </UiGlassButton>
        </div>
      </div>
    </div>

    <div class="relative z-10 px-4 sm:px-6 py-2 border-b border-white/[0.04] flex flex-wrap gap-2">
      <input v-model="searchQuery" aria-label="Search logs" placeholder="Search logs" class="bg-white/[0.04] border border-white/[0.08] rounded px-2 py-1 text-xs" />
      <select v-model="levelFilter" aria-label="Filter by level" class="bg-[#292d3e] border border-white/[0.08] rounded px-2 py-1 text-xs">
        <option value="all">All levels</option><option value="debug">Debug</option><option value="info">Info</option><option value="warn">Warn</option><option value="error">Error</option>
      </select>
      <select v-model="sourceFilter" aria-label="Filter by source" class="bg-[#292d3e] border border-white/[0.08] rounded px-2 py-1 text-xs">
        <option value="all">All sources</option><option value="daemon">Daemon</option><option value="embedded">GUI</option>
      </select>
    </div>

    <!-- Log viewer -->
    <div class="relative z-10 flex-1 overflow-hidden flex flex-col">
      <!-- Status bar -->
      <div class="px-4 sm:px-6 py-2 border-b border-white/[0.04] text-xs text-white/30 flex items-center justify-between">
        <div class="flex items-center gap-4">
          <span>{{ filteredLogs.length }} of {{ logs.length }} lines</span>
          <!-- Which sources are feeding this view. The GUI always logs itself, so
               'embedded' is always live; 'daemon' only when one is attached. -->
          <span class="text-cyan-300/70">&#x25cf; gui</span>
          <span :class="daemonAttached ? 'text-violet-300/70' : 'text-white/35'" :title="daemonAttached ? 'Daemon attached — live daemon logs' : 'Daemon not attached; GUI logs only'">
            {{ daemonAttached ? '\u25cf daemon' : '\u25cb daemon offline' }}
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
        v-if="filteredLogs.length === 0"
        class="flex-1 overflow-y-auto font-mono text-sm p-4 min-h-0"
      >
        <PageState
          :state="isLoading ? 'loading' : (!daemonAttached && logs.length === 0 ? 'offline' : 'empty')"
          :title="isLoading ? 'Loading logs…' : (!daemonAttached && logs.length === 0 ? 'Daemon offline' : 'No log lines')"
          :description="isLoading
            ? 'Reading the in-process buffers.'
            : (!daemonAttached && logs.length === 0
              ? 'GUI logs will appear here. Attach the daemon on 5149 for daemon-side lines.'
              : (searchQuery || levelFilter !== 'all' || sourceFilter !== 'all'
                ? 'Nothing matches the current filters.'
                : 'GUI and daemon log lines will appear here as they arrive.'))"
          :primary-action="!daemonAttached && logs.length === 0 && !isLoading ? 'Open Settings' : ''"
          @primary="navigateTo('/settings')"
        />
      </div>

      <VirtualList
        v-else-if="filteredLogs.length > 100"
        ref="logsContainer"
        :items="filteredLogs"
        :item-height="32"
        :overscan="10"
        class="flex-1 font-mono text-sm p-4 min-h-0"
      >
        <template #default="{ item: log }">
          <div
            class="py-1 px-2 rounded transition-colors group cursor-default h-full"
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
            {{ sourceLabel(log) }}
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
        </template>
      </VirtualList>

      <div
        v-else
        ref="logsContainer"
        class="flex-1 overflow-y-auto font-mono text-sm p-4 space-y-0 min-h-0"
      >
        <div
          v-for="(log, idx) in filteredLogs"
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
            {{ sourceLabel(log) }}
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
import { filterLogs } from '~/lib/logFilters'
import { ChevronDown, Trash2, Circle, Copy } from 'lucide-vue-next'
const toastApi = useToast()
const { confirm } = useConfirm()

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
const logsContainer = ref<HTMLElement | { $el?: HTMLElement } | null>(null)
const autoScroll = ref(true)
function readLiveLogsPref(): boolean {
  try {
    const v = localStorage.getItem('nanna.logs.live')
    if (v === null) return true
    return v === '1' || v === 'true'
  } catch { return true }
}
const liveMode = ref(readLiveLogsPref())
const isLoading = ref(false)
const searchQuery = ref('')
const levelFilter = ref('all')
const sourceFilter = ref('all')
const filteredLogs = computed(() => filterLogs(logs.value, levelFilter.value, sourceFilter.value, searchQuery.value))
const lastUpdate = ref<Date | null>(null)
const copyLabel = ref('Copy all')
let copyLabelTimer: ReturnType<typeof setTimeout> | null = null

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

function sourceLabel(log: LogEntry): string {
  const s = sourceOf(log)
  return s === 'embedded' ? 'gui' : s
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

function onLogsLiveEvent(e: Event) {
  const detail = (e as CustomEvent<{ live?: boolean }>).detail
  if (detail && typeof detail.live === 'boolean') {
    liveMode.value = detail.live
  } else {
    liveMode.value = readLiveLogsPref()
  }
}

onMounted(async () => {
  window.addEventListener('nanna:logs-live', onLogsLiveEvent)
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
  window.removeEventListener('nanna:logs-live', onLogsLiveEvent)
  if (pollTimer) clearInterval(pollTimer)
  if (copyLabelTimer) clearTimeout(copyLabelTimer)
})

watch(autoScroll, async () => {
  if (autoScroll.value) {
    await nextTick()
    scrollToBottom()
  }
})

function scrollToBottom() {
  const el = logsContainer.value as any
  if (!el) return
  const node = el.$el ?? el
  if (node && typeof node.scrollTop === 'number') {
    node.scrollTop = node.scrollHeight
  }
}

function toggleAutoScroll() {
  autoScroll.value = !autoScroll.value
}

function toggleLiveMode() {
  liveMode.value = !liveMode.value
  try { localStorage.setItem('nanna.logs.live', liveMode.value ? '1' : '0') } catch { /* ignore */ }
}

async function clearLogs() {
  const ok = await confirm({
    title: 'Clear logs?',
    message: 'Clears the visible log buffer for this session. Daemon files on disk are untouched.',
    confirmLabel: 'Clear',
    danger: true,
  })
  if (!ok) return
  // Remember how far we cleared, otherwise the next poll re-fetches the same
  // history straight back into the view.
  const newest = logs.value[logs.value.length - 1]
  if (newest) clearedBefore.value = newest.timestamp
  logs.value = []
  toastApi.success('Logs cleared')
}

async function copyAllLogs() {
  if (logs.value.length === 0) return

  const text = logs.value.map((log) => {
    const level = log.level.toUpperCase().padEnd(5)
    return `${log.timestamp} [${sourceOf(log)}] ${level} [${log.target}] ${log.message}`
  }).join('\n')

  try {
    await navigator.clipboard.writeText(text)
    copyLabel.value = 'Copied'
    if (copyLabelTimer) clearTimeout(copyLabelTimer)
    copyLabelTimer = setTimeout(() => {
      copyLabel.value = 'Copy all'
      copyLabelTimer = null
    }, 1500)
    toastApi.success(`Copied ${logs.value.length} log lines`)
  } catch (e) {
    console.error('Failed to copy logs:', e)
    toastApi.error('Failed to copy logs')
  }
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
