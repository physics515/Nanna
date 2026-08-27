<script setup lang="ts">
import type { NuiIconName } from './NuiIcon.vue'

const props = withDefaults(defineProps<{
  icon: NuiIconName
  label: string
  size?: number
  disabled?: boolean
}>(), {
  size: 16,
})
</script>

<template>
  <button
    type="button"
    class="nui-icon-btn flex items-center justify-center p-2 text-nui-fg transition-colors hover:text-white disabled:opacity-50 disabled:pointer-events-none"
    :aria-label="props.label"
    :title="props.label"
    :disabled="props.disabled"
  >
    <NuiIcon :name="props.icon" :size="props.size" />
  </button>
</template>
