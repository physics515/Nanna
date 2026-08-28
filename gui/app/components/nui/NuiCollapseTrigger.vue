<script setup lang="ts">
/** Header trigger for collapsible message sections (Thinking, tool calls):
 * bold label, muted meta, and a chevron that points down when open. */
const props = defineProps<{
  label: string
  meta?: string
}>()

const open = defineModel<boolean>({ default: true })
</script>

<template>
  <button
    type="button"
    class="flex min-w-0 items-center gap-2.5 text-left"
    :aria-expanded="open"
    @click="open = !open"
  >
    <p class="whitespace-nowrap text-sm font-semibold leading-normal text-nui-fg">{{ props.label }}</p>
    <p v-if="props.meta" class="min-w-0 truncate text-xs leading-normal text-nui-muted">{{ props.meta }}</p>
    <NuiIcon
      name="chevron-down"
      :size="16"
      class="text-nui-fg transition-transform"
      :class="!open && '-rotate-90'"
    />
  </button>
</template>
