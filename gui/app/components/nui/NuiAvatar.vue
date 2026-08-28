<script setup lang="ts">
const props = withDefaults(defineProps<{
  /** 'user' = purple disc with the for-you glyph; 'nanna' = pink disc with the wordmark. */
  variant: 'user' | 'nanna'
  size?: number
}>(), {
  size: 48,
})
</script>

<template>
  <div
    class="flex items-center justify-center rounded-full shrink-0 overflow-clip"
    :class="props.variant === 'user' ? 'bg-nui-accent' : 'bg-nui-pink'"
    :style="{ width: `${props.size}px`, height: `${props.size}px` }"
  >
    <NuiIcon v-if="props.variant === 'user'" name="for-you" :size="Math.round(props.size * 2 / 3)" class="text-nui-fg" />
    <!-- Wordmark is 52:21 — sized to the same 2/3 inset the icon variant uses. -->
    <NuiLogo v-else :height="Math.round((props.size * 2 / 3) * 21 / 52)" />
  </div>
</template>
