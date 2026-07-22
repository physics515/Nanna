<template>
  <Transition name="slide">
    <div
      v-if="visible"
      :class="['connection-status', statusClass]"
      role="status"
      :aria-live="status === 'error' || status === 'offline' ? 'assertive' : 'polite'"
    >
      <div class="flex items-center gap-3">
        <span class="status-icon" aria-hidden="true">{{ icon }}</span>
        <div class="flex-1 min-w-0">
          <div class="font-medium">{{ title }}</div>
          <div v-if="message" class="text-sm opacity-90 break-words">{{ message }}</div>
        </div>
        <button
          v-if="canRetry"
          type="button"
          class="retry-btn min-h-8 min-w-[4.5rem]"
          :disabled="isRetrying"
          @click="$emit('retry')"
        >
          {{ isRetrying ? 'Retrying…' : 'Retry' }}
        </button>
        <button
          v-if="canDismiss"
          type="button"
          class="dismiss-btn min-h-8 min-w-8"
          aria-label="Dismiss"
          @click="$emit('dismiss')"
        >
          ✕
        </button>
      </div>
    </div>
  </Transition>
</template>

<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  status: 'connecting' | 'connected' | 'error' | 'offline' | 'reconnecting'
  message?: string
  visible: boolean
  canRetry?: boolean
  canDismiss?: boolean
  isRetrying?: boolean
  /** Optional override for the bold title line. */
  titleOverride?: string
}>()

defineEmits<{
  retry: []
  dismiss: []
}>()

const statusClass = computed(() => ({
  'status-connecting': props.status === 'connecting' || props.status === 'reconnecting',
  'status-connected': props.status === 'connected',
  'status-error': props.status === 'error',
  'status-offline': props.status === 'offline',
}))

const icon = computed(() => {
  switch (props.status) {
    case 'connecting':
    case 'reconnecting':
      return '↻'
    case 'connected':
      return '✓'
    case 'error':
      return '!'
    case 'offline':
      return '⌁'
    default:
      return '•'
  }
})

const title = computed(() => {
  if (props.titleOverride) return props.titleOverride
  switch (props.status) {
    case 'connecting':
      return 'Connecting to daemon…'
    case 'reconnecting':
      return 'Reconnecting to daemon…'
    case 'connected':
      return 'Connected'
    case 'error':
      return 'Something went wrong'
    case 'offline':
      return 'Daemon offline'
    default:
      return ''
  }
})
</script>

<style scoped>
.connection-status {
  padding: 0.75rem 1rem;
  border-radius: 0.5rem;
  margin: 0.5rem;
  backdrop-filter: blur(8px);
}

.status-connecting {
  background: linear-gradient(135deg, rgba(234, 179, 8, 0.2), rgba(234, 179, 8, 0.1));
  border: 1px solid rgba(234, 179, 8, 0.35);
  color: #fbbf24;
}

.status-connected {
  background: linear-gradient(135deg, rgba(34, 197, 94, 0.2), rgba(34, 197, 94, 0.1));
  border: 1px solid rgba(34, 197, 94, 0.35);
  color: #4ade80;
}

.status-error {
  background: linear-gradient(135deg, rgba(239, 68, 68, 0.2), rgba(239, 68, 68, 0.1));
  border: 1px solid rgba(239, 68, 68, 0.35);
  color: #f87171;
}

.status-offline {
  background: linear-gradient(135deg, rgba(156, 163, 175, 0.22), rgba(156, 163, 175, 0.1));
  border: 1px solid rgba(156, 163, 175, 0.35);
  color: #cbd5e1;
}

.status-icon {
  font-size: 1.1rem;
  line-height: 1;
  flex-shrink: 0;
}

.status-connecting .status-icon {
  animation: spin 1.1s linear infinite;
  display: inline-block;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

.retry-btn {
  padding: 0.375rem 0.75rem;
  background: rgba(255, 255, 255, 0.1);
  border: 1px solid currentColor;
  border-radius: 0.375rem;
  font-size: 0.75rem;
  font-weight: 500;
  transition: all 0.2s;
}

.retry-btn:hover:not(:disabled) {
  background: rgba(255, 255, 255, 0.2);
}

.retry-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.dismiss-btn {
  padding: 0.25rem;
  opacity: 0.7;
  transition: opacity 0.2s;
}

.dismiss-btn:hover {
  opacity: 1;
}

.slide-enter-active,
.slide-leave-active {
  transition: all 0.3s ease;
}

.slide-enter-from,
.slide-leave-to {
  opacity: 0;
  transform: translateY(-10px);
}
</style>
