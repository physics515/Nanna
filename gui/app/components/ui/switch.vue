<script setup lang="ts">
import { SwitchRoot, SwitchThumb } from 'radix-vue'
import { cn } from '~/lib/utils'

interface Props {
  modelValue?: boolean
  disabled?: boolean
  class?: string
}

const props = defineProps<Props>()

const emit = defineEmits<{
  'update:modelValue': [value: boolean]
}>()
</script>

<template>
  <SwitchRoot
    :checked="props.modelValue"
    :disabled="props.disabled"
    :class="cn(
      'peer inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent transition-colors',
      'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-nanna-accent focus-visible:ring-offset-2 focus-visible:ring-offset-nanna-bg-deep',
      'disabled:cursor-not-allowed disabled:opacity-50',
      'data-[state=checked]:bg-nanna-primary data-[state=unchecked]:bg-nanna-bg-elevated',
      props.class
    )"
    @update:checked="emit('update:modelValue', $event)"
  >
    <SwitchThumb
      :class="cn(
        'pointer-events-none block h-5 w-5 rounded-full bg-white shadow-lg ring-0 transition-transform',
        'data-[state=checked]:translate-x-5 data-[state=unchecked]:translate-x-0'
      )"
    />
  </SwitchRoot>
</template>
