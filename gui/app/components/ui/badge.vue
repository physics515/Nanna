<script setup lang="ts">
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '~/lib/utils'

const badgeVariants = cva(
  'inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium transition-colors',
  {
    variants: {
      variant: {
        default: 'bg-nanna-primary/20 text-nanna-primary border border-nanna-primary/30',
        secondary: 'bg-nanna-secondary/20 text-nanna-secondary border border-nanna-secondary/30',
        success: 'bg-nanna-success/20 text-nanna-success border border-nanna-success/30',
        warning: 'bg-nanna-warning/20 text-nanna-warning border border-nanna-warning/30',
        error: 'bg-nanna-error/20 text-nanna-error border border-nanna-error/30',
        accent: 'bg-nanna-accent/20 text-nanna-accent border border-nanna-accent/30',
        outline: 'text-nanna-text-muted border border-nanna-text-dim/30',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  }
)

type BadgeVariants = VariantProps<typeof badgeVariants>

interface Props {
  variant?: BadgeVariants['variant']
  class?: string
}

const props = withDefaults(defineProps<Props>(), {
  variant: 'default',
})
</script>

<template>
  <span :class="cn(badgeVariants({ variant: props.variant }), props.class)">
    <slot />
  </span>
</template>
