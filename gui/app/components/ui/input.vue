<script setup lang="ts">
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '~/lib/utils'

/**
 * `size` must be a declared prop, not a fallthrough attribute: HTML's `size` on an `<input>`
 * takes a positive integer, so a stray `size="sm"` reaches the DOM and throws an IndexSizeError
 * that Vue reports as a warning and then swallows. Scale matches `UiButton` so the two line up.
 */
const inputVariants = cva(
  'flex w-full rounded-lg bg-black/20 border border-white/[0.08] text-nanna-text placeholder:text-nanna-text-dim/50 transition-colors focus:outline-none focus:border-nanna-primary/50 focus:ring-1 focus:ring-nanna-primary/20 disabled:cursor-not-allowed disabled:opacity-50',
  {
    variants: {
      size: {
        default: 'h-10 px-4 py-2 text-sm',
        sm: 'h-8 px-3 text-xs',
        lg: 'h-12 px-6 text-base',
      },
    },
    defaultVariants: {
      size: 'default',
    },
  }
)

type InputVariants = VariantProps<typeof inputVariants>

interface Props {
  modelValue?: string
  type?: string
  placeholder?: string
  disabled?: boolean
  size?: InputVariants['size']
  class?: string
}

const props = withDefaults(defineProps<Props>(), {
  type: 'text',
  size: 'default',
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()
</script>

<template>
  <input
    :type="props.type"
    :value="props.modelValue"
    :placeholder="props.placeholder"
    :disabled="props.disabled"
    :class="cn(inputVariants({ size: props.size }), props.class)"
    @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
  />
</template>
