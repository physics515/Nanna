<script setup lang="ts">
import { cn } from '~/lib/utils'
import type { NuiIconName } from './NuiIcon.vue'

const props = withDefaults(defineProps<{
  label: string
  icon?: NuiIconName
  color?: 'info' | 'accent' | 'pink' | 'green' | 'yellow'
  class?: string
}>(), {
  icon: 'dot',
  color: 'info',
})

const colorClass = {
  info: 'bg-nui-info',
  accent: 'bg-nui-accent',
  pink: 'bg-nui-pink',
  green: 'bg-nui-green',
  yellow: 'bg-nui-yellow',
}
</script>

<template>
  <div
    :class="cn(
      'flex h-10 items-center gap-1 rounded-lg px-4 overflow-clip text-nui-bg',
      colorClass[props.color],
      props.class,
    )"
    :title="props.label"
  >
    <NuiIcon v-if="props.icon" :name="props.icon" :size="16" />
    <p class="whitespace-nowrap text-xs">{{ props.label }}</p>
  </div>
</template>
