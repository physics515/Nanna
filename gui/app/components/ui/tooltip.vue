<script setup lang="ts">
import { TooltipRoot, TooltipTrigger, TooltipPortal, TooltipContent, TooltipArrow, TooltipProvider } from 'radix-vue'
import { cn } from '~/lib/utils'

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
          :class="cn(
            'z-50 overflow-hidden rounded-md bg-nanna-bg-elevated border border-nanna-primary/20 px-3 py-1.5 text-xs text-nanna-text shadow-md',
            'animate-in fade-in-0 zoom-in-95',
            'data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95',
            'data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2',
            props.class
          )"
        >
          {{ props.content }}
          <TooltipArrow class="fill-nanna-bg-elevated" />
        </TooltipContent>
      </TooltipPortal>
    </TooltipRoot>
  </TooltipProvider>
</template>
