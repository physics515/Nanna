<script setup lang="ts">
const props = withDefaults(defineProps<{
  opacity?: number
  blur?: number
  colors?: [string, string, string]
  sizes?: [string, string, string]
  tag?: string
}>(), {
  tag: 'div',
})

const { meshBg, containerStyle, onEnter, onLeave } = useGroundGlass({
  opacity: props.opacity,
  blur: props.blur,
  colors: props.colors,
  sizes: props.sizes,
})

defineExpose({ onEnter, onLeave })
</script>

<template>
  <component
    :is="tag"
    class="ground-glass"
    :style="containerStyle"
    @mouseenter="onEnter"
    @mouseleave="onLeave"
  >
    <span class="ground-glass__mesh" :style="{ background: meshBg }" />
    <slot />
  </component>
</template>

<style scoped>
.ground-glass {
  position: relative;
  isolation: isolate;
  overflow: hidden;
  border-radius: 0.75rem;
  /* Glass slab borders */
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  border-left: 1px solid rgba(255, 255, 255, 0.04);
  border-bottom: 1.5px solid rgba(71, 85, 105, 0.18);
  border-right: 1px solid rgba(71, 85, 105, 0.10);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.04),
    0 1.5px 1px -0.5px rgba(0, 0, 0, 0.18),
    0 3px 8px -3px rgba(0, 0, 0, 0.12);
}

/* Layer 0: animated mesh gradient */
.ground-glass__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
}

/* Layer 2: ground glass noise overlay */
.ground-glass::after {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 2;
  pointer-events: none;
  border-radius: inherit;
  opacity: 0.18;
  background-blend-mode: soft-light;
  background: repeating-radial-gradient(
    circle,
    #1a2035,
    #1a2035 2px,
    #253050 2px 4px,
    #1a2035 4px 6px,
    #253050 6px 8px,
    #1a2035 8px 10px,
    #253050 10px 12px
  ) 0 0 / 100% 100%;
}
</style>
