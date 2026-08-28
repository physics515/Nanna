<script setup lang="ts">
const props = defineProps<{
  title: string
}>()
</script>

<template>
  <div class="flex w-full items-center gap-4 pb-4 pr-4 pt-2">
    <p class="min-w-0 flex-1 truncate text-sm font-semibold leading-normal text-nui-fg" :title="props.title">
      {{ props.title }}
    </p>
    <!-- Right side controls: agent select, model tag, … -->
    <slot />
  </div>
</template>
