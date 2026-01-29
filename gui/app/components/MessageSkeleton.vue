<template>
  <div class="message-skeleton" :class="{ 'ml-12': isUser, 'mr-12': !isUser }">
    <div class="flex items-start gap-3">
      <div 
        :class="[
          'w-8 h-8 rounded-full skeleton-pulse flex-shrink-0',
          isUser ? 'bg-nanna-primary/30' : 'bg-nanna-accent/30'
        ]"
      />
      <div class="flex-1 space-y-2">
        <div class="h-3 w-16 skeleton-pulse rounded" />
        <div class="space-y-1.5">
          <div 
            v-for="(width, i) in lineWidths" 
            :key="i" 
            class="h-4 skeleton-pulse rounded"
            :style="{ width }"
          />
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  isUser?: boolean
  lines?: number
}>()

const lineWidths = computed(() => {
  const count = props.lines || 3
  // Generate varied widths for natural look
  return Array.from({ length: count }, (_, i) => {
    if (i === count - 1) return '40%' // Last line shorter
    return `${70 + Math.random() * 25}%`
  })
})
</script>

<style scoped>
.message-skeleton {
  padding: 1rem;
  border-radius: 0.5rem;
  background: var(--color-nanna-bg-surface);
}

.skeleton-pulse {
  background: linear-gradient(
    90deg,
    var(--color-nanna-bg-elevated) 0%,
    var(--color-nanna-bg-surface) 50%,
    var(--color-nanna-bg-elevated) 100%
  );
  background-size: 200% 100%;
  animation: shimmer 1.5s infinite;
}

@keyframes shimmer {
  0% {
    background-position: 200% 0;
  }
  100% {
    background-position: -200% 0;
  }
}
</style>
