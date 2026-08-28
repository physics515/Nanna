<script setup lang="ts">
// The wordmark is the project's own /logo.svg — the same mark the Figma
// design embeds (three hard-light layer groups).
const props = withDefaults(defineProps<{
  height?: number
}>(), {
  height: 20,
})
</script>

<template>
  <img
    src="/logo.svg"
    alt="Nanna"
    class="block select-none pointer-events-none"
    :style="{ height: `${props.height}px`, width: 'auto' }"
  />
</template>
