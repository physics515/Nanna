<template>
  <div :class="['tool-call-card', statusClass]">
    <div class="flex items-center justify-between mb-2">
      <div class="flex items-center gap-2">
        <span class="tool-icon">{{ statusIcon }}</span>
        <span class="tool-name">{{ toolCall.name }}</span>
      </div>
      <div class="flex items-center gap-2 text-xs">
        <span v-if="toolCall.duration_ms" class="text-nanna-text-dim">
          {{ toolCall.duration_ms }}ms
        </span>
        <span :class="statusBadgeClass">{{ statusText }}</span>
      </div>
    </div>
    
    <!-- Input -->
    <div class="tool-section">
      <button 
        @click="showInput = !showInput" 
        class="tool-section-header"
      >
        <span>Input</span>
        <span class="text-xs">{{ showInput ? '▼' : '▶' }}</span>
      </button>
      <Transition name="expand">
        <pre v-if="showInput" class="tool-code">{{ formatJson(toolCall.input) }}</pre>
      </Transition>
    </div>
    
    <!-- Output -->
    <div v-if="toolCall.output" class="tool-section">
      <button 
        @click="showOutput = !showOutput" 
        class="tool-section-header"
      >
        <span>Output</span>
        <span class="text-xs">{{ showOutput ? '▼' : '▶' }}</span>
      </button>
      <Transition name="expand">
        <pre v-if="showOutput" class="tool-code tool-output">{{ truncateOutput(toolCall.output) }}</pre>
      </Transition>
    </div>
    
    <!-- Loading indicator for in-progress calls -->
    <div v-if="status === 'started'" class="tool-loading">
      <span class="loading-dot"></span>
      <span class="loading-dot" style="animation-delay: 0.2s"></span>
      <span class="loading-dot" style="animation-delay: 0.4s"></span>
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

const statusIcon = computed(() => {
  switch (props.status) {
    case 'started': return '⚡'
    case 'completed': return '✅'
    case 'error': return '❌'
    default: return '🔧'
  }
})

const statusText = computed(() => {
  switch (props.status) {
    case 'started': return 'Running...'
    case 'completed': return 'Success'
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
  'bg-nanna-warning/20 text-nanna-warning': props.status === 'started',
  'bg-nanna-success/20 text-nanna-success': props.status === 'completed',
  'bg-nanna-error/20 text-nanna-error': props.status === 'error',
}))

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
}

.tool-running {
  border-left-color: var(--color-nanna-warning);
  animation: pulse 2s infinite;
}

.tool-success {
  border-left-color: var(--color-nanna-success);
}

.tool-error {
  border-left-color: var(--color-nanna-error);
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.7; }
}

.tool-icon {
  font-size: 1rem;
}

.tool-name {
  font-family: var(--font-mono);
  font-weight: 600;
  color: var(--color-nanna-secondary);
}

.status-badge {
  padding: 0.125rem 0.5rem;
  border-radius: 9999px;
  font-size: 0.7rem;
  font-weight: 500;
}

.tool-section {
  margin-top: 0.5rem;
}

.tool-section-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  padding: 0.25rem 0.5rem;
  background: var(--color-nanna-bg-surface);
  border-radius: 0.25rem;
  color: var(--color-nanna-text-muted);
  font-size: 0.75rem;
  cursor: pointer;
  transition: background 0.2s;
}

.tool-section-header:hover {
  background: var(--color-nanna-bg-deep);
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
}

.tool-output {
  color: var(--color-nanna-accent);
}

.tool-loading {
  display: flex;
  gap: 0.25rem;
  padding: 0.5rem;
  justify-content: center;
}

.loading-dot {
  width: 6px;
  height: 6px;
  background: var(--color-nanna-warning);
  border-radius: 50%;
  animation: bounce 1s infinite;
}

@keyframes bounce {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(-4px); }
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
