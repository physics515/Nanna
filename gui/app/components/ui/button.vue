<script setup lang="ts">
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '~/lib/utils'

const buttonVariants = cva(
  'inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-lg text-sm font-medium transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-nanna-primary/50 disabled:pointer-events-none disabled:opacity-50',
  {
    variants: {
      variant: {
        default: 'glass-btn text-white',
        secondary: 'bg-black/20 text-nanna-text border border-white/[0.08] hover:bg-white/[0.06] hover:border-white/[0.12]',
        ghost: 'text-nanna-text-muted hover:bg-white/[0.06] hover:text-nanna-text',
        destructive: 'bg-nanna-error/20 text-nanna-error border border-nanna-error/30 hover:bg-nanna-error/30',
        link: 'text-nanna-primary underline-offset-4 hover:underline',
        accent: 'glass-btn text-white',
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

<style scoped>
.glass-btn {
  position: relative;
  isolation: isolate;
  overflow: hidden;
  background: rgba(139, 92, 246, 0.5);
  border: 1px solid rgba(139, 92, 246, 0.3);
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
}

.glass-btn:hover {
  background: rgba(139, 92, 246, 0.65);
  border-color: rgba(139, 92, 246, 0.5);
  box-shadow: 0 2px 8px rgba(139, 92, 246, 0.25);
}

/* Ground glass noise overlay */
.glass-btn::after {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 1;
  pointer-events: none;
  border-radius: inherit;
  opacity: 0.12;
  mix-blend-mode: overlay;
  background-image: url("data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='200'%20height='200'%3E%3Cfilter%20id='n'%3E%3CfeTurbulence%20type='fractalNoise'%20baseFrequency='0.65'%20numOctaves='3'%20stitchTiles='stitch'/%3E%3C/filter%3E%3Crect%20width='100%25'%20height='100%25'%20filter='url(%23n)'/%3E%3C/svg%3E");
  background-repeat: repeat;
}
</style>
