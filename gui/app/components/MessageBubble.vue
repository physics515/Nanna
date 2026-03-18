<script setup lang="ts">
import { useSplatter } from '~/composables/useSplatter'

const props = withDefaults(defineProps<{
  variant: 'user' | 'assistant'
}>(), {
  variant: 'assistant',
})

const splatterColors = computed<[string, string, string]>(() => {
  return props.variant === 'user'
    ? ['139, 92, 246', '120, 80, 220', '160, 120, 250']   // violet
    : ['34, 211, 238', '56, 189, 248', '20, 184, 220']     // cyan
})

const { splatterBg, onEnter: splatterEnter, onLeave: splatterLeave } = useSplatter({
  colors: splatterColors,
  opacityRanges: [[0.08, 0.14], [0.06, 0.10], [0.04, 0.08]],
  sizes: ['65%', '60%', '50%'],
  lerpSpeed: 0.006,
  interval: 3000,
})
</script>

<template>
  <div
    class="msg-bubble"
    :class="`msg-bubble--${variant}`"
    @mouseenter="splatterEnter"
    @mouseleave="splatterLeave"
  >
    <!-- Layer 0: animated splatter gradient -->
    <span class="msg-bubble__splatter" :style="{ background: splatterBg }" />
    <!-- Layer 1: content -->
    <div class="msg-bubble__content">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.msg-bubble {
  position: relative;
  isolation: isolate;
  overflow: hidden;
  border-radius: 0.75rem;
  padding: 0.75rem 1rem;
  font-size: 14px;
  line-height: 1.6;
  background: transparent;
  border: 1px solid rgba(71, 85, 105, 0.2);
}

/* User: violet tint */
.msg-bubble--user {
  border-color: rgba(139, 92, 246, 0.15);
}

/* Assistant: cyan tint */
.msg-bubble--assistant {
  border-color: rgba(34, 211, 238, 0.12);
}

/* Layer 0: animated splatter */
.msg-bubble__splatter {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
  opacity: 1;
  transition: opacity 0.3s ease;
}

/* Layer 1: content */
.msg-bubble__content {
  position: relative;
  z-index: 1;
}
</style>
