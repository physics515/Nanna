<template>
  <span v-if="hasActivity" class="flex items-center gap-1" :class="compact ? '' : 'text-xs text-nanna-accent'">
    <span class="animate-pulse text-nanna-accent">●</span>
    <span v-if="!compact">{{ statusText }}</span>
  </span>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useSessionState } from '~/composables/useSessionState'

const props = withDefaults(defineProps<{
  sessionId: string
  compact?: boolean
}>(), {
  compact: false,
})

const sessionIdRef = computed(() => props.sessionId)

const {
  isLoading,
  isStreaming,
  activeToolCalls,
  hasActiveWork,
} = useSessionState(sessionIdRef)

const hasActivity = computed(() => hasActiveWork.value)

const statusText = computed(() => {
  // Check for active tool calls first
  const runningTools = activeToolCalls.value.filter(t => t.status === 'started')
  if (runningTools.length > 0) {
    return `Running ${runningTools[0].name}...`
  }

  if (isStreaming.value) {
    return 'Streaming...'
  }

  if (isLoading.value) {
    return 'Thinking...'
  }

  return 'Working...'
})
</script>
