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
      'flex h-10 w-full rounded-lg bg-black/20 border border-white/[0.08] px-4 py-2 text-sm text-nanna-text placeholder:text-nanna-text-dim/50 transition-colors',
      'focus:outline-none focus:border-nanna-primary/50 focus:ring-1 focus:ring-nanna-primary/20',
      'disabled:cursor-not-allowed disabled:opacity-50',
      props.class
    )"
    @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
  />
</template>
