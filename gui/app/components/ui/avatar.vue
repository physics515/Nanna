<script setup lang="ts">
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '~/lib/utils'

const avatarVariants = cva(
  'relative flex shrink-0 overflow-hidden rounded-full items-center justify-center font-bold',
  {
    variants: {
      size: {
        default: 'h-10 w-10 text-sm',
        sm: 'h-8 w-8 text-xs',
        lg: 'h-12 w-12 text-base',
      },
      variant: {
        default: 'bg-nanna-bg-elevated/50 text-nanna-text',
        primary: 'bg-nanna-primary/20 text-nanna-primary',
        accent: 'bg-nanna-primary/15 text-nanna-primary-hover',
        secondary: 'bg-nanna-secondary/20 text-nanna-secondary',
      },
    },
    defaultVariants: {
      size: 'default',
      variant: 'default',
    },
  }
)

type AvatarVariants = VariantProps<typeof avatarVariants>

interface Props {
  size?: AvatarVariants['size']
  variant?: AvatarVariants['variant']
  src?: string
  fallback?: string
  class?: string
}

const props = withDefaults(defineProps<Props>(), {
  size: 'default',
  variant: 'default',
})
</script>

<template>
  <span :class="cn(avatarVariants({ size: props.size, variant: props.variant }), props.class)">
    <img 
      v-if="props.src" 
      :src="props.src" 
      class="aspect-square h-full w-full object-cover"
    />
    <span v-else>{{ props.fallback }}</span>
  </span>
</template>
