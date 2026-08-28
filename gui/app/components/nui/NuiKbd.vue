<script setup lang="ts">
defineProps<{
  keys: string
}>()
</script>

<template>
  <kbd class="bg-nui-muted p-0.5 font-nui text-xs font-[450] not-italic leading-normal text-nui-bg">{{ keys }}</kbd>
</template>
