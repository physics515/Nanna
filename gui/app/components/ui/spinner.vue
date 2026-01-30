<script setup lang="ts">
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '~/lib/utils'
import { Loader2 } from 'lucide-vue-next'

const spinnerVariants = cva('animate-spin text-nanna-accent', {
  variants: {
    size: {
      default: 'h-5 w-5',
      sm: 'h-4 w-4',
      lg: 'h-6 w-6',
      xl: 'h-8 w-8',
    },
  },
  defaultVariants: {
    size: 'default',
  },
})

type SpinnerVariants = VariantProps<typeof spinnerVariants>

interface Props {
  size?: SpinnerVariants['size']
  class?: string
}

const props = withDefaults(defineProps<Props>(), {
  size: 'default',
})
</script>

<template>
  <Loader2 :class="cn(spinnerVariants({ size: props.size }), props.class)" />
</template>
