<script setup lang="ts">
import { computed } from 'vue'

/** Icon names mirror the SVG files in public/icons/nui/ — the exact
 * glyphs exported from the Figma design. */
export type NuiIconName =
  | 'add' | 'agents' | 'attach' | 'channels' | 'chat' | 'chevron-down'
  | 'circle-check' | 'close' | 'copy' | 'delete' | 'dot' | 'edit-task'
  | 'for-you' | 'input-circle' | 'log' | 'maximize' | 'memory' | 'minimize'
  | 'model-stats' | 'notifications' | 'output-circle' | 'overflow-menu'
  | 'scheduler' | 'send' | 'settings' | 'tasks' | 'tool-stats' | 'toolbox'
  | 'update' | 'workspaces'

const props = withDefaults(defineProps<{
  name: NuiIconName
  size?: number
}>(), {
  size: 16,
})

// The exported SVGs carry baked-in fills, so they are applied as CSS masks:
// the glyph geometry comes from the asset, the color from `currentColor`.
const url = computed(() => `/icons/nui/${props.name}.svg`)
</script>

<template>
  <span
    class="nui-icon"
    :style="{
      width: `${props.size}px`,
      height: `${props.size}px`,
      'mask-image': `url(${url})`,
      '-webkit-mask-image': `url(${url})`,
    }"
    aria-hidden="true"
  />
</template>

<style scoped>
.nui-icon {
  display: inline-block;
  flex-shrink: 0;
  background-color: currentColor;
  mask-repeat: no-repeat;
  -webkit-mask-repeat: no-repeat;
  mask-size: 100% 100%;
  -webkit-mask-size: 100% 100%;
}
</style>
