<script setup lang="ts">
const props = defineProps<{
  title: string
  /** Secondary line, e.g. relative time ("35m ago"). */
  meta?: string
  active?: boolean
}>()
</script>

<template>
  <button
    type="button"
    class="flex w-full flex-col items-start gap-2 overflow-clip p-4 text-left transition-colors hover:bg-white/5"
    :class="props.active && 'bg-white/5'"
    :title="props.title"
  >
    <p class="w-full overflow-hidden text-ellipsis break-words text-xs font-semibold leading-normal text-nui-fg line-clamp-2">
      {{ props.title }}
    </p>
    <p v-if="props.meta" class="w-full truncate text-xs leading-normal text-nui-muted">
      {{ props.meta }}
    </p>
  </button>
</template>
