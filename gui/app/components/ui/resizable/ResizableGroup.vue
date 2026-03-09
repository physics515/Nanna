<script setup lang="ts">
import { SplitterGroup } from 'radix-vue'

defineProps<{
  direction: 'horizontal' | 'vertical'
  class?: string
}>()
</script>

<template>
  <SplitterGroup :direction="direction" :class="['flex h-full w-full', $props.class]">
    <slot />
  </SplitterGroup>
</template>
