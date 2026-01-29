<template>
  <div :class="['tool-call-card', statusClass]">
    <div class="flex items-center justify-between mb-2">
      <div class="flex items-center gap-2">
        <span class="tool-type-icon" :title="toolCategory">{{ toolIcon }}</span>
        <span class="tool-name">{{ toolCall.name }}</span>
        <span v-if="status === 'started'" class="running-indicator" />
      </div>
      <div class="flex items-center gap-2 text-xs">
        <span v-if="toolCall.duration_ms" class="duration-badge">
          {{ formatDuration(toolCall.duration_ms) }}
        </span>
        <span :class="statusBadgeClass">
          <span class="status-dot" />
          {{ statusText }}
        </span>
      </div>
    </div>
    
    <!-- Progress bar for running tools -->
    <div v-if="status === 'started'" class="progress-bar">
      <div class="progress-bar-inner" />
    </div>
    
    <!-- Input -->
    <div class="tool-section">
      <button 
        @click="showInput = !showInput" 
        class="tool-section-header"
      >
        <span class="flex items-center gap-1">
          <span class="text-xs">📥</span>
          Input
        </span>
        <span class="chevron" :class="{ 'rotate-90': showInput }">›</span>
      </button>
      <Transition name="expand">
        <pre v-if="showInput" class="tool-code">{{ formatJson(toolCall.input) }}</pre>
      </Transition>
    </div>
    
    <!-- Output -->
    <div v-if="toolCall.output || status === 'started'" class="tool-section">
      <button 
        @click="showOutput = !showOutput" 
        class="tool-section-header"
      >
        <span class="flex items-center gap-1">
          <span class="text-xs">📤</span>
          Output
        </span>
        <span class="chevron" :class="{ 'rotate-90': showOutput }">›</span>
      </button>
      <Transition name="expand">
        <div v-if="showOutput">
          <pre v-if="toolCall.output" class="tool-code tool-output" :class="{ 'output-error': status === 'error' }">{{ truncateOutput(toolCall.output) }}</pre>
          <div v-else class="tool-code flex items-center gap-2 text-nanna-text-dim">
            <span class="animate-pulse">Waiting for result...</span>
          </div>
        </div>
      </Transition>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'

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

const showInput = ref(false)
const showOutput = ref(true)

// Tool category mapping for icons
const toolCategories: Record<string, { icon: string; category: string }> = {
  // File operations
  read_file: { icon: '📄', category: 'File' },
  write_file: { icon: '✏️', category: 'File' },
  list_dir: { icon: '📁', category: 'File' },
  // Execution
  exec: { icon: '⚡', category: 'Shell' },
  // Web
  web_fetch: { icon: '🌐', category: 'Web' },
  web_search: { icon: '🔍', category: 'Search' },
  // Browser
  browser: { icon: '🖥️', category: 'Browser' },
  // Memory
  memory_search: { icon: '🧠', category: 'Memory' },
  memory_get: { icon: '📚', category: 'Memory' },
  // Default
  echo: { icon: '💬', category: 'Debug' },
}

const toolIcon = computed(() => {
  const name = props.toolCall.name.toLowerCase()
  return toolCategories[name]?.icon || '🔧'
})

const toolCategory = computed(() => {
  const name = props.toolCall.name.toLowerCase()
  return toolCategories[name]?.category || 'Tool'
})

const statusText = computed(() => {
  switch (props.status) {
    case 'started': return 'Running'
    case 'completed': return 'Done'
    case 'error': return 'Failed'
    default: return ''
  }
})

const statusClass = computed(() => ({
  'tool-running': props.status === 'started',
  'tool-success': props.status === 'completed',
  'tool-error': props.status === 'error',
}))

const statusBadgeClass = computed(() => ({
  'status-badge': true,
  'status-running': props.status === 'started',
  'status-success': props.status === 'completed',
  'status-error': props.status === 'error',
}))

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60000).toFixed(1)}m`
}

function formatJson(obj: any): string {
  try {
    return JSON.stringify(obj, null, 2)
  } catch {
    return String(obj)
  }
}

function truncateOutput(output: string): string {
  const maxLength = 2000
  if (output.length > maxLength) {
    return output.substring(0, maxLength) + '\n... (truncated)'
  }
  return output
}
</script>

<style scoped>
.tool-call-card {
  background: var(--color-nanna-bg-elevated);
  border-radius: 0.5rem;
  padding: 0.75rem;
  margin: 0.5rem 0;
  border-left: 3px solid var(--color-nanna-secondary);
  font-size: 0.875rem;
  transition: all 0.2s ease;
}

.tool-running {
  border-left-color: var(--color-nanna-warning);
  box-shadow: 0 0 0 1px var(--color-nanna-warning) inset,
              0 0 20px -5px var(--color-nanna-warning);
}

.tool-success {
  border-left-color: var(--color-nanna-success);
}

.tool-error {
  border-left-color: var(--color-nanna-error);
  box-shadow: 0 0 0 1px var(--color-nanna-error) inset;
}

.tool-type-icon {
  font-size: 1.1rem;
  filter: drop-shadow(0 0 2px currentColor);
}

.tool-name {
  font-family: var(--font-mono);
  font-weight: 600;
  color: var(--color-nanna-secondary);
}

.running-indicator {
  width: 8px;
  height: 8px;
  background: var(--color-nanna-warning);
  border-radius: 50%;
  animation: blink 1s infinite;
}

@keyframes blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

.duration-badge {
  padding: 0.125rem 0.375rem;
  background: var(--color-nanna-bg-surface);
  border-radius: 0.25rem;
  color: var(--color-nanna-text-dim);
  font-family: var(--font-mono);
}

.status-badge {
  display: flex;
  align-items: center;
  gap: 0.375rem;
  padding: 0.125rem 0.5rem;
  border-radius: 9999px;
  font-size: 0.7rem;
  font-weight: 500;
}

.status-running {
  background: var(--color-nanna-warning);
  background: linear-gradient(90deg, var(--color-nanna-warning), #f59e0b);
  color: #000;
}

.status-success {
  background: var(--color-nanna-success);
  background: linear-gradient(90deg, var(--color-nanna-success), #10b981);
  color: #000;
}

.status-error {
  background: var(--color-nanna-error);
  background: linear-gradient(90deg, var(--color-nanna-error), #ef4444);
  color: #fff;
}

.status-dot {
  width: 6px;
  height: 6px;
  background: currentColor;
  border-radius: 50%;
}

.status-running .status-dot {
  animation: pulse-dot 1s infinite;
}

@keyframes pulse-dot {
  0%, 100% { transform: scale(1); }
  50% { transform: scale(1.5); }
}

.progress-bar {
  height: 2px;
  background: var(--color-nanna-bg-surface);
  border-radius: 1px;
  overflow: hidden;
  margin: 0.5rem 0;
}

.progress-bar-inner {
  height: 100%;
  width: 30%;
  background: var(--color-nanna-warning);
  border-radius: 1px;
  animation: progress 1.5s ease-in-out infinite;
}

@keyframes progress {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(400%); }
}

.tool-section {
  margin-top: 0.5rem;
}

.tool-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 0.375rem 0.5rem;
  background: var(--color-nanna-bg-surface);
  border-radius: 0.25rem;
  color: var(--color-nanna-text-muted);
  font-size: 0.75rem;
  cursor: pointer;
  transition: all 0.2s;
  border: 1px solid transparent;
}

.tool-section-header:hover {
  background: var(--color-nanna-bg-deep);
  border-color: var(--color-nanna-primary);
}

.chevron {
  transition: transform 0.2s;
  font-size: 1rem;
  line-height: 1;
}

.rotate-90 {
  transform: rotate(90deg);
}

.tool-code {
  margin-top: 0.25rem;
  padding: 0.5rem;
  background: var(--color-nanna-bg-deep);
  border-radius: 0.25rem;
  font-family: var(--font-mono);
  font-size: 0.75rem;
  color: var(--color-nanna-text);
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 200px;
  overflow-y: auto;
  border: 1px solid var(--color-nanna-primary);
}

.tool-output {
  color: var(--color-nanna-accent);
  border-color: var(--color-nanna-accent);
}

.output-error {
  color: var(--color-nanna-error);
  border-color: var(--color-nanna-error);
  background: linear-gradient(135deg, var(--color-nanna-bg-deep), rgba(239, 68, 68, 0.1));
}

.expand-enter-active,
.expand-leave-active {
  transition: all 0.2s ease;
  overflow: hidden;
}

.expand-enter-from,
.expand-leave-to {
  opacity: 0;
  max-height: 0;
}

.expand-enter-to,
.expand-leave-from {
  opacity: 1;
  max-height: 300px;
}
</style>
