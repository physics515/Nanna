<script setup lang="ts">
import { cn } from '~/lib/utils'
import { ChevronDown } from 'lucide-vue-next'

interface Option {
  value: string
  label: string
  disabled?: boolean
}

interface Props {
  modelValue?: string
  options: Option[]
  placeholder?: string
  disabled?: boolean
  class?: string
}

const props = withDefaults(defineProps<Props>(), {
  placeholder: 'Select an option',
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()
</script>

<template>
  <div class="relative">
    <select
      :value="props.modelValue"
      :disabled="props.disabled"
      :class="cn(
        'flex h-10 w-full appearance-none rounded-lg bg-black/20 border border-white/[0.08] px-4 py-2 pr-10 text-sm text-nanna-text transition-colors cursor-pointer',
        'focus:outline-none focus:border-nanna-primary/50 focus:ring-1 focus:ring-nanna-primary/20',
        'disabled:cursor-not-allowed disabled:opacity-50',
        props.class
      )"
      @change="emit('update:modelValue', ($event.target as HTMLSelectElement).value)"
    >
      <option v-if="props.placeholder" value="" disabled selected class="text-nanna-text-dim">
        {{ props.placeholder }}
      </option>
      <option 
        v-for="option in props.options" 
        :key="option.value"
        :value="option.value"
        :disabled="option.disabled"
        class="bg-nanna-bg-surface text-nanna-text"
      >
        {{ option.label }}
      </option>
    </select>
    <ChevronDown class="absolute right-3 top-1/2 -translate-y-1/2 h-4 w-4 text-nanna-text-dim pointer-events-none" />
  </div>
</template>
