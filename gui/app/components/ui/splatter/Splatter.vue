<script setup lang="ts">
import type { MaybeRefOrGetter } from 'vue'

const props = withDefaults(defineProps<{
  colors?: MaybeRefOrGetter<[string, string, string]>
  opacityRanges?: [[number, number], [number, number], [number, number]]
  sizes?: [string, string, string]
  edgeOpacity?: number
  tag?: string
  /** When true, shows the initial random splatter but disables all animation */
  frozen?: boolean
  /** What triggers the animation: 'hover' (default) or 'focus' (focusin/focusout) */
  trigger?: 'hover' | 'focus'
}>(), {
  tag: 'div',
  frozen: false,
  trigger: 'hover',
})

const { splatterBg, onEnter, onLeave } = useSplatter({
  colors: props.colors,
  opacityRanges: props.opacityRanges,
  sizes: props.sizes,
  edgeOpacity: props.edgeOpacity,
})

defineExpose({ onEnter, onLeave })
</script>

<template>
  <component
    :is="tag"
    class="splatter"
    @mouseenter="!frozen && trigger === 'hover' && onEnter()"
    @mouseleave="!frozen && trigger === 'hover' && onLeave()"
    @focusin="!frozen && trigger === 'focus' && onEnter()"
    @focusout="!frozen && trigger === 'focus' && onLeave()"
  >
    <span class="splatter__mesh" :style="{ background: splatterBg }" />
    <slot />
  </component>
</template>

<style scoped>
.splatter {
  position: relative;
  isolation: isolate;
  overflow: hidden;
}

.splatter__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
  -webkit-mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  -webkit-mask-size: 200px 200px;
  mask-size: 200px 200px;
}
</style>
