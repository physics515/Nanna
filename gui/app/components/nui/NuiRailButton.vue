<script setup lang="ts">
import type { NuiIconName } from './NuiIcon.vue'

const props = withDefaults(defineProps<{
  icon: NuiIconName
  label: string
  active?: boolean
  /** Color of the active-item edge. */
  accent?: 'accent' | 'green'
  size?: number
  /** Small pink counter over the icon (e.g. unread notifications). */
  badge?: number
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
    class="relative flex items-center justify-center border-l px-4 py-2 transition-colors"
    :class="props.active
      ? [accentClass[props.accent], 'text-nui-fg']
      : 'border-transparent text-nui-fg/70 hover:text-nui-fg'"
    :aria-label="props.label"
    :aria-current="props.active ? 'page' : undefined"
    :title="props.label"
  >
    <NuiIcon :name="props.icon" :size="props.size" />
    <span
      v-if="props.badge && props.badge > 0"
      class="absolute right-2 top-0 min-w-4 rounded-full bg-nui-pink px-1 text-center text-[10px] font-semibold leading-4 text-nui-bg"
    >{{ props.badge > 9 ? '9+' : props.badge }}</span>
  </button>
</template>
