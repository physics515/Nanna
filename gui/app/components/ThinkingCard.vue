<template>
  <div
    class="thinking-card"
    :class="{ 'thinking-card--active': isActive }"
    :style="{ background: splatterBg }"
    @mouseenter="onEnter"
    @mouseleave="onLeave"
  >
    <!-- Collapsed header (always visible) -->
    <button class="thinking-header" @click="expanded = !expanded">
      <span class="thinking-icon">💭</span>
      <span class="thinking-label">Thinking</span>
      <span v-if="isActive" class="thinking-active-dot" />
      <span v-if="!isActive && wordCount > 0" class="thinking-word-count">{{ wordCount }} words</span>
      <span class="thinking-spacer" />
      <svg class="thinking-chevron" :class="{ 'thinking-chevron--open': expanded }" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 2l3 3-3 3" />
      </svg>
    </button>

    <!-- Expanded detail -->
    <Transition name="thinking-expand">
      <div v-if="expanded" class="thinking-detail">
        <pre class="thinking-content">{{ content }}<span v-if="isActive" class="cursor-blink">▋</span></pre>
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import { useSplatter } from '~/composables/useSplatter'

const props = defineProps<{
  content: string
  isActive?: boolean
}>()

const expanded = ref(false)

const wordCount = computed(() => {
  if (!props.content) return 0
  return props.content.trim().split(/\s+/).length
})

const statusColors = computed<[string, string, string]>(() => {
  if (props.isActive) {
    return ['251,191,36', '234,179,8', '245,158,11'] // amber while thinking
  }
  return ['100,116,139', '71,85,105', '148,163,184'] // slate when done
})

const { splatterBg, onEnter, onLeave } = useSplatter({
  colors: statusColors,
  opacityRanges: [[0.06, 0.08], [0.04, 0.06], [0.02, 0.04]],
  sizes: ['70%', '65%', '55%'],
})
</script>

<style scoped>
.thinking-card {
  position: relative;
  border-radius: 0.5rem;
  overflow: hidden;
  transition: box-shadow 0.15s ease;
}
.thinking-card:hover {
  box-shadow: 0 1px 4px rgba(0, 0, 0, 0.15);
}

.thinking-header {
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

.thinking-icon {
  font-size: 13px;
  flex-shrink: 0;
}

.thinking-label {
  font-family: var(--font-mono, monospace);
  font-weight: 600;
  color: #94a3b8;
  flex-shrink: 0;
}

.thinking-card--active .thinking-label {
  color: #fbbf24;
}

.thinking-active-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #fbbf24;
  animation: thinking-blink 1s infinite;
  flex-shrink: 0;
}

@keyframes thinking-blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

.thinking-word-count {
  font-family: var(--font-mono, monospace);
  font-size: 10px;
  color: rgba(148, 163, 184, 0.5);
  flex-shrink: 0;
}

.thinking-spacer {
  flex: 1;
}

.thinking-chevron {
  width: 8px;
  height: 8px;
  color: rgba(148, 163, 184, 0.4);
  flex-shrink: 0;
  transition: transform 0.15s ease;
}
.thinking-chevron--open {
  transform: rotate(90deg);
}

/* Expanded detail */
.thinking-detail {
  padding: 0 10px 8px;
}

.thinking-content {
  padding: 6px 8px;
  background: rgba(0, 0, 0, 0.2);
  border-radius: 4px;
  font-family: var(--font-mono, monospace);
  font-size: 11px;
  color: #94a3b8;
  overflow-x: auto;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 300px;
  overflow-y: auto;
}

.cursor-blink {
  animation: blink 1s step-end infinite;
}

@keyframes blink {
  0%, 100% { opacity: 1; }
  50% { opacity: 0; }
}

/* Expand transition */
.thinking-expand-enter-active,
.thinking-expand-leave-active {
  transition: all 0.15s ease;
  overflow: hidden;
}
.thinking-expand-enter-from,
.thinking-expand-leave-to {
  opacity: 0;
  max-height: 0;
}
.thinking-expand-enter-to,
.thinking-expand-leave-from {
  opacity: 1;
  max-height: 500px;
}
</style>
