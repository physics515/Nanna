<script setup lang="ts">
import { TooltipRoot, TooltipTrigger, TooltipPortal, TooltipContent, TooltipArrow, TooltipProvider } from 'radix-vue'

interface Props {
  content: string
  side?: 'top' | 'right' | 'bottom' | 'left'
  delayDuration?: number
  class?: string
}

const props = withDefaults(defineProps<Props>(), {
  side: 'top',
  delayDuration: 200,
})
</script>

<template>
  <TooltipProvider :delay-duration="props.delayDuration">
    <TooltipRoot>
      <TooltipTrigger as-child>
        <slot />
      </TooltipTrigger>
      <TooltipPortal>
        <TooltipContent
          :side="props.side"
          :side-offset="5"
          class="tooltip-content"
        >
          {{ props.content }}
          <TooltipArrow class="fill-[#334155]" />
        </TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </TooltipProvider>
</template>

<style scoped>
.tooltip-content {
  z-index: 50;
  overflow: hidden;
  border-radius: 0.375rem;
  background: rgba(51, 65, 85, 0.3);
  border: 1px solid rgba(255, 255, 255, 0.06);
  padding: 0.375rem 0.75rem;
  font-size: 0.75rem;
  color: #e2e8f0;
  box-shadow: 0 4px 6px -1px rgba(0, 0, 0, 0.2);
}
.tooltip-content[data-state="delayed-open"],
.tooltip-content[data-state="instant-open"] {
  animation: tooltip-in 150ms ease-out forwards;
}
.tooltip-content[data-state="closed"] {
  animation: tooltip-out 100ms ease-in forwards;
}

@keyframes tooltip-in {
  from {
    opacity: 0;
    transform: scale(0.95);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}
@keyframes tooltip-out {
  from {
    opacity: 1;
    transform: scale(1);
  }
  to {
    opacity: 0;
    transform: scale(0.95);
  }
}
</style>
