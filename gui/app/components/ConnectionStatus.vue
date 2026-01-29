<template>
  <Transition name="slide">
    <div 
      v-if="visible"
      :class="['connection-status', statusClass]"
    >
      <div class="flex items-center gap-3">
        <span class="status-icon">{{ icon }}</span>
        <div class="flex-1">
          <div class="font-medium">{{ title }}</div>
          <div v-if="message" class="text-sm opacity-80">{{ message }}</div>
        </div>
        <button 
          v-if="canRetry"
          @click="$emit('retry')"
          class="retry-btn"
          :disabled="isRetrying"
        >
          {{ isRetrying ? 'Retrying...' : 'Retry' }}
        </button>
        <button 
          v-if="canDismiss"
          @click="$emit('dismiss')"
          class="dismiss-btn"
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
      return '🔄'
    case 'connected':
      return '✅'
    case 'error':
      return '❌'
    case 'offline':
      return '📡'
    default:
      return '⚡'
  }
})

const title = computed(() => {
  switch (props.status) {
    case 'connecting':
      return 'Connecting...'
    case 'reconnecting':
      return 'Reconnecting...'
    case 'connected':
      return 'Connected'
    case 'error':
      return 'Connection Error'
    case 'offline':
      return 'Offline'
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
  border: 1px solid rgba(234, 179, 8, 0.3);
  color: #fbbf24;
}

.status-connected {
  background: linear-gradient(135deg, rgba(34, 197, 94, 0.2), rgba(34, 197, 94, 0.1));
  border: 1px solid rgba(34, 197, 94, 0.3);
  color: #4ade80;
}

.status-error {
  background: linear-gradient(135deg, rgba(239, 68, 68, 0.2), rgba(239, 68, 68, 0.1));
  border: 1px solid rgba(239, 68, 68, 0.3);
  color: #f87171;
}

.status-offline {
  background: linear-gradient(135deg, rgba(156, 163, 175, 0.2), rgba(156, 163, 175, 0.1));
  border: 1px solid rgba(156, 163, 175, 0.3);
  color: #9ca3af;
}

.status-icon {
  font-size: 1.25rem;
  animation: pulse 2s infinite;
}

.status-connecting .status-icon,
.status-reconnecting .status-icon {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.6; }
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
  opacity: 0.6;
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
