<script setup lang="ts">
import { cn } from '~/lib/utils'

interface Props {
  modelValue?: string
  placeholder?: string
  disabled?: boolean
  rows?: number
  class?: string
}

const props = withDefaults(defineProps<Props>(), {
  rows: 3,
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()
</script>

<template>
  <textarea
    :value="props.modelValue"
    :placeholder="props.placeholder"
    :disabled="props.disabled"
    :rows="props.rows"
    :class="cn(
      'flex min-h-[80px] w-full rounded-lg bg-nanna-bg-surface/50 border border-nanna-primary/20 px-4 py-3 text-sm text-nanna-text placeholder:text-nanna-text-dim transition-all resize-none',
      'focus:outline-none focus:border-nanna-accent focus:ring-1 focus:ring-nanna-accent/30',
      'disabled:cursor-not-allowed disabled:opacity-50',
      props.class
    )"
    @input="emit('update:modelValue', ($event.target as HTMLTextAreaElement).value)"
  />
</template>
