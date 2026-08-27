<script setup lang="ts">
import { computed } from 'vue'
import { cn } from '~/lib/utils'
import type { NuiIconName } from './NuiIcon.vue'

interface Option {
  value: string
  label: string
  disabled?: boolean
}

const props = withDefaults(defineProps<{
  modelValue?: string
  options: Option[]
  placeholder?: string
  /** Optional leading glyph (e.g. 'workspaces' on the workspace picker). */
  icon?: NuiIconName
  /** 'attached' squares off the top corners — the pill hangs from the edge above it. */
  variant?: 'default' | 'attached'
  disabled?: boolean
  class?: string
}>(), {
  placeholder: 'Select…',
  variant: 'default',
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const selectedLabel = computed(() =>
  props.options.find(o => o.value === props.modelValue)?.label ?? props.placeholder,
)
</script>

<template>
  <div
    :class="cn(
      'relative flex h-10 items-center gap-2.5 bg-nui-accent px-4 overflow-clip',
      props.variant === 'attached' ? 'rounded-b-lg' : 'rounded-lg',
      props.disabled && 'opacity-50',
      props.class,
    )"
  >
    <NuiIcon v-if="props.icon" :name="props.icon" :size="16" class="text-nui-fg" />
    <p class="flex-1 min-w-0 truncate text-xs text-nui-fg">{{ selectedLabel }}</p>
    <NuiIcon name="chevron-down" :size="16" class="text-nui-fg" />
    <!-- The native control drives interaction; the pill above is the visual. -->
    <select
      :value="props.modelValue"
      :disabled="props.disabled"
      class="absolute inset-0 h-full w-full cursor-pointer opacity-0 disabled:cursor-not-allowed"
      @change="emit('update:modelValue', ($event.target as HTMLSelectElement).value)"
    >
      <option v-if="props.placeholder && !props.modelValue" value="" disabled selected>
        {{ props.placeholder }}
      </option>
      <option
        v-for="option in props.options"
        :key="option.value"
        :value="option.value"
        :disabled="option.disabled"
      >
        {{ option.label }}
      </option>
    </select>
  </div>
</template>
