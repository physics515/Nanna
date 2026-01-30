<script setup lang="ts">
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '~/lib/utils'

const buttonVariants = cva(
  'inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-lg text-sm font-medium transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-nanna-accent/50 disabled:pointer-events-none disabled:opacity-50',
  {
    variants: {
      variant: {
        default: 'bg-nanna-primary text-white hover:bg-nanna-primary-hover hover:shadow-glow-violet',
        secondary: 'bg-nanna-bg-surface text-nanna-text border border-nanna-primary/30 hover:bg-nanna-bg-elevated hover:border-nanna-primary',
        ghost: 'text-nanna-text-muted hover:bg-nanna-bg-surface hover:text-nanna-text',
        destructive: 'bg-nanna-error/20 text-nanna-error border border-nanna-error/30 hover:bg-nanna-error/30',
        link: 'text-nanna-accent underline-offset-4 hover:underline',
        accent: 'bg-nanna-accent text-nanna-bg-deep hover:bg-nanna-accent-hover hover:shadow-glow',
      },
      size: {
        default: 'h-10 px-4 py-2',
        sm: 'h-8 px-3 text-xs',
        lg: 'h-12 px-6 text-base',
        icon: 'h-10 w-10',
        'icon-sm': 'h-8 w-8',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  }
)

type ButtonVariants = VariantProps<typeof buttonVariants>

interface Props {
  variant?: ButtonVariants['variant']
  size?: ButtonVariants['size']
  class?: string
  disabled?: boolean
  type?: 'button' | 'submit' | 'reset'
}

const props = withDefaults(defineProps<Props>(), {
  variant: 'default',
  size: 'default',
  type: 'button',
})
</script>

<template>
  <button
    :type="props.type"
    :class="cn(buttonVariants({ variant: props.variant, size: props.size }), props.class)"
    :disabled="props.disabled"
  >
    <slot />
  </button>
</template>
