<script setup lang="ts">
import type { NuiIconName } from './NuiIcon.vue'

const props = withDefaults(defineProps<{
  icon: NuiIconName
  label: string
  active?: boolean
  /** Color of the active-item edge. */
  accent?: 'accent' | 'green'
  size?: number
}>(), {
  accent: 'accent',
  size: 32,
})

const accentClass = {
  accent: 'border-nui-accent',
  green: 'border-nui-green',
}
</script>

<template>
  <button
    type="button"
    class="flex items-center justify-center border-l px-4 py-2 transition-colors"
    :class="props.active
      ? [accentClass[props.accent], 'text-nui-fg']
      : 'border-transparent text-nui-fg/70 hover:text-nui-fg'"
    :aria-label="props.label"
    :aria-current="props.active ? 'page' : undefined"
    :title="props.label"
  >
    <NuiIcon :name="props.icon" :size="props.size" />
  </button>
</template>
