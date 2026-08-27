<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  /** Fraction complete, 0–1. */
  value: number
}>()

const pct = computed(() => `${Math.min(1, Math.max(0, props.value)) * 100}%`)
</script>

<template>
  <div
    class="flex h-2 w-full border border-solid border-nui-pink"
    role="progressbar"
    :aria-valuenow="Math.round(Math.min(1, Math.max(0, props.value)) * 100)"
    aria-valuemin="0"
    aria-valuemax="100"
  >
    <div class="bg-nui-pink" :style="{ width: pct }" />
  </div>
</template>
