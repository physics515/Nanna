<script setup lang="ts">
import { SplitterPanel } from 'radix-vue'

defineProps<{
  defaultSize?: number
  minSize?: number
  maxSize?: number
  collapsible?: boolean
  order?: number
  class?: string
}>()
</script>

<template>
  <SplitterPanel
    :default-size="defaultSize"
    :min-size="minSize"
    :max-size="maxSize"
    :collapsible="collapsible"
    :order="order"
    :class="$props.class"
  >
    <slot />
  </SplitterPanel>
</template>
