<script setup lang="ts">
import { SplitterResizeHandle } from 'radix-vue'

defineProps<{
  class?: string
}>()
</script>

<template>
  <SplitterResizeHandle :class="['panel-resize-handle', $props.class]">
    <slot />
  </SplitterResizeHandle>
</template>

<style scoped>
.panel-resize-handle {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 4px;
  flex-shrink: 0;
  background: transparent;
  transition: background 0.15s ease;
  cursor: col-resize;
}
.panel-resize-handle::after {
  content: '';
  position: absolute;
  top: 0;
  bottom: 0;
  width: 1px;
  background: transparent;
  transition: background 0.15s ease;
}
.panel-resize-handle:hover::after,
.panel-resize-handle[data-state="drag"]::after {
  background: rgba(139, 92, 246, 0.4);
}
.panel-resize-handle[data-state="drag"] {
  background: rgba(139, 92, 246, 0.08);
}
</style>
