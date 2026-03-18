<template>
  <div
    class="tool-card"
    :class="statusClass"
    :style="{ background: splatterBg }"
    @mouseenter="onEnter"
    @mouseleave="onLeave"
  >
    <!-- Collapsed header (always visible) -->
    <button class="tool-header" @click="expanded = !expanded">
      <span class="tool-icon">{{ toolIcon }}</span>
      <span class="tool-name">{{ toolCall.name }}</span>
      <span v-if="inputSummary" class="tool-input-summary">{{ inputSummary }}</span>
      <span class="tool-spacer" />
      <span v-if="status === 'started'" class="tool-running-dot" />
      <span v-if="toolCall.duration_ms" class="tool-duration">{{ formatDuration(toolCall.duration_ms) }}</span>
      <span :class="['tool-status', `tool-status--${status}`]">
        {{ status === 'started' ? '⟳' : status === 'completed' ? '✓' : '✗' }}
      </span>
      <svg class="tool-chevron" :class="{ 'tool-chevron--open': expanded }" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 2l3 3-3 3" />
      </svg>
    </button>

    <!-- Expanded detail -->
    <Transition name="tool-expand">
      <div v-if="expanded" class="tool-detail">
        <!-- Input -->
        <div class="tool-section">
          <div class="tool-section-label">📥 Input</div>
          <pre class="tool-code">{{ formatJson(toolCall.input) }}</pre>
        </div>
        <!-- Output -->
        <div v-if="toolCall.output || status === 'started'" class="tool-section">
          <div class="tool-section-label">📤 Output</div>
          <pre v-if="toolCall.output" class="tool-code" :class="{ 'tool-code--error': status === 'error' }">{{ truncateOutput(toolCall.output) }}</pre>
          <div v-else class="tool-code tool-code--waiting">Waiting for result...</div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import { useSplatter } from '~/composables/useSplatter'

interface ToolCallInfo {
  id: string
  name: string
  input: any
  output: string
  success: boolean
  duration_ms: number
}

const props = defineProps<{
  toolCall: ToolCallInfo
  status: 'started' | 'completed' | 'error'
}>()

const expanded = ref(false)

// Splatter mesh colors based on status
const statusColors = computed<[string, string, string]>(() => {
  switch (props.status) {
    case 'started': return ['251,191,36', '234,179,8', '245,158,11'] // amber
    case 'error': return ['239,68,68', '220,38,38', '248,113,113'] // red
    default: return ['139,92,246', '99,102,241', '167,139,250'] // violet/indigo
  }
})

const { splatterBg, onEnter, onLeave } = useSplatter({
  colors: statusColors,
  opacityRanges: [[0.06, 0.08], [0.04, 0.06], [0.02, 0.04]],
  sizes: ['70%', '65%', '55%'],
})

const toolIcons: Record<string, string> = {
  read_file: '📄', write_file: '✏️', list_dir: '📁', explore: '🗂️',
  exec: '⚡', bash: '⚡',
  web_fetch: '🌐', web_search: '🔍', web_search_batch: '🔍',
  browser_action: '🖥️', browser_screenshot: '📸', browser_extract: '🖥️', browser_evaluate: '🖥️',
  remember: '🧠', recall: '🧠', reflect: '🧠', recall_messages: '🧠',
  discover_tools: '🔧', create_tool: '🔧',
  remind: '⏰', cancel_reminder: '⏰', list_reminders: '⏰',
  todo: '✅', task: '📋',
  code_outline: '📊', code_search: '🔎', project_structure: '🏗️',
  read_pdf: '📕', ocr: '👁️', analyze_image: '🖼️', describe_image: '🖼️',
  text_to_speech: '🔊', transcribe: '🎙️',
  screenshot: '📸', status: 'ℹ️', echo: '💬', wonder: '💭',
}

const toolIcon = computed(() => toolIcons[props.toolCall.name] || '🔧')

const inputSummary = computed(() => {
  const input = props.toolCall.input
  if (!input || typeof input !== 'object') return ''
  // Show the most relevant parameter as a brief summary
  const path = input.file_path || input.filePath || input.path || input.command || input.query || input.url
  if (path) {
    const s = String(path)
    return s.length > 60 ? '…' + s.slice(-55) : s
  }
  return ''
})

const statusClass = computed(() => ({
  'tool-card--running': props.status === 'started',
  'tool-card--error': props.status === 'error',
}))

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60000).toFixed(1)}m`
}

function formatJson(obj: any): string {
  try { return JSON.stringify(obj, null, 2) } catch { return String(obj) }
}

function truncateOutput(output: string): string {
  return output.length > 2000 ? output.substring(0, 2000) + '\n... (truncated)' : output
}
</script>

<style scoped>
.tool-card {
  position: relative;
  border-radius: 0.5rem;
  overflow: hidden;
  transition: box-shadow 0.15s ease;
}
.tool-card:hover {
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.15);
}

.tool-header {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 100%;
  padding: 5px 10px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 12px;
  color: #e2e8f0;
  outline: none;
  text-align: left;
}

.tool-icon {
  font-size: 13px;
  flex-shrink: 0;
}

.tool-name {
  font-family: var(--font-mono, monospace);
  font-weight: 600;
  color: #c4b5fd;
  flex-shrink: 0;
}

.tool-input-summary {
  color: rgba(148, 163, 184, 0.6);
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}

.tool-spacer {
  flex: 1;
}

.tool-running-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #fbbf24;
  animation: tool-blink 1s infinite;
  flex-shrink: 0;
}

@keyframes tool-blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

.tool-duration {
  font-family: var(--font-mono, monospace);
  font-size: 10px;
  color: rgba(148, 163, 184, 0.5);
  flex-shrink: 0;
}

.tool-status {
  font-size: 11px;
  flex-shrink: 0;
}
.tool-status--completed { color: #34d399; }
.tool-status--error { color: #fb7185; }
.tool-status--started { color: #fbbf24; animation: tool-spin 1s linear infinite; }

@keyframes tool-spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.tool-chevron {
  width: 8px;
  height: 8px;
  color: rgba(148, 163, 184, 0.4);
  flex-shrink: 0;
  transition: transform 0.15s ease;
}
.tool-chevron--open {
  transform: rotate(90deg);
}

/* Expanded detail */
.tool-detail {
  padding: 0 10px 8px;
}

.tool-section {
  margin-top: 6px;
}

.tool-section-label {
  font-size: 10px;
  color: rgba(148, 163, 184, 0.5);
  margin-bottom: 2px;
}

.tool-code {
  padding: 6px 8px;
  background: rgba(0, 0, 0, 0.2);
  border-radius: 4px;
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  color: #cbd5e1;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 200px;
  overflow-y: auto;
}

.tool-code--error {
  color: #fca5a5;
}

.tool-code--waiting {
  color: rgba(148, 163, 184, 0.5);
  font-style: italic;
}

/* Expand transition */
.tool-expand-enter-active,
.tool-expand-leave-active {
  transition: all 0.15s ease;
  overflow: hidden;
}
.tool-expand-enter-from,
.tool-expand-leave-to {
  opacity: 0;
  max-height: 0;
}
.tool-expand-enter-to,
.tool-expand-leave-from {
  opacity: 1;
  max-height: 500px;
}
</style>
