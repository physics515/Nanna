<script setup lang="ts">
import { cn } from '~/lib/utils'

interface Props {
  modelValue?: string
  type?: string
  placeholder?: string
  disabled?: boolean
  class?: string
}

const props = withDefaults(defineProps<Props>(), {
  type: 'text',
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
    :class="cn(
      'flex h-10 w-full rounded-lg bg-nanna-bg-surface/50 border border-nanna-primary/20 px-4 py-2 text-sm text-nanna-text placeholder:text-nanna-text-dim transition-all',
      'focus:outline-none focus:border-nanna-accent focus:ring-1 focus:ring-nanna-accent/30',
      'disabled:cursor-not-allowed disabled:opacity-50',
      props.class
    )"
    @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
  />
</template>
