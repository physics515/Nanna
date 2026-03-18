<script setup lang="ts">
import { useSplatter } from '~/composables/useSplatter'

interface QueuedMessage {
  id: string
  content: string
  timestamp: string
}

defineProps<{
  count: number
  messages: QueuedMessage[]
}>()

const emit = defineEmits<{
  clear: []
  remove: [id: string]
}>()

const { splatterBg, onEnter, onLeave } = useSplatter({
  colors: ['139,92,246', '99,102,241', '167,139,250'],
  opacityRanges: [[0.06, 0.08], [0.04, 0.06], [0.02, 0.04]],
  sizes: ['70%', '65%', '55%'],
})
</script>

<template>
  <div
    class="queue-card"
    :style="{ background: splatterBg }"
    @mouseenter="onEnter"
    @mouseleave="onLeave"
  >
    <div class="queue-header">
      <div class="queue-info">
        <svg class="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
            d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
        </svg>
        <span>{{ count }} message{{ count > 1 ? 's' : '' }} queued</span>
      </div>
      <button class="queue-clear" @click="emit('clear')">Clear queue</button>
    </div>
    <div v-if="messages.length > 0" class="queue-list">
      <div v-for="(qMsg, idx) in messages.slice(0, 3)" :key="qMsg.id" class="queue-item">
        <span class="queue-num">{{ idx + 1 }}.</span>
        <span class="queue-text">{{ qMsg.content }}</span>
        <button class="queue-remove" @click="emit('remove', qMsg.id)" title="Remove">
          <svg class="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      </div>
      <div v-if="messages.length > 3" class="queue-more">
        +{{ messages.length - 3 }} more...
      </div>
    </div>
  </div>
</template>

<style scoped>
.queue-card {
  border-radius: 0.75rem;
  padding: 10px 14px;
  overflow: hidden;
}

.queue-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.queue-info {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  color: #e2e8f0;
}

.queue-clear {
  font-size: 11px;
  color: rgba(148, 163, 184, 0.6);
  background: none;
  border: none;
  cursor: pointer;
  transition: color 0.15s;
}
.queue-clear:hover {
  color: #e2e8f0;
}

.queue-list {
  margin-top: 8px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.queue-item {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

.queue-num {
  color: rgba(148, 163, 184, 0.4);
  flex-shrink: 0;
}

.queue-text {
  color: #cbd5e1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 300px;
}

.queue-remove {
  color: rgba(148, 163, 184, 0.3);
  background: none;
  border: none;
  cursor: pointer;
  margin-left: auto;
  transition: color 0.15s;
  flex-shrink: 0;
}
.queue-remove:hover {
  color: #fb7185;
}

.queue-more {
  font-size: 11px;
  color: rgba(148, 163, 184, 0.4);
}
</style>
