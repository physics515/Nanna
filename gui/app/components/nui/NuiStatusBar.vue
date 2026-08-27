<script setup lang="ts">
const props = withDefaults(defineProps<{
  uiVersion?: string
  serverVersion?: string
  connected?: boolean
  statusText?: string
  updating?: boolean
  /** An update is downloadable — the button colors up and names the version. */
  updateLabel?: string
  updateTooltip?: string
}>(), {
  statusText: 'Connected to server',
})

const emit = defineEmits<{
  update: []
}>()
</script>

<template>
  <footer class="flex h-9 w-full shrink-0 items-center gap-4 px-9 text-xs font-semibold leading-normal text-nui-muted">
    <p v-if="props.uiVersion" class="whitespace-nowrap">v{{ props.uiVersion }} UI</p>
    <p v-if="props.serverVersion" class="whitespace-nowrap">v{{ props.serverVersion }} Server</p>
    <button
      type="button"
      class="flex items-center gap-1 p-2 transition-colors disabled:opacity-50"
      :class="props.updateLabel ? 'text-nui-info hover:text-nui-fg' : 'text-nui-muted hover:text-nui-fg'"
      :title="props.updateTooltip ?? 'Check for updates'"
      :aria-label="props.updateTooltip ?? 'Check for updates'"
      :disabled="props.updating"
      @click="emit('update')"
    >
      <NuiIcon name="update" :size="16" :class="props.updating && 'animate-spin'" />
      <span v-if="props.updateLabel" class="whitespace-nowrap">{{ props.updateLabel }}</span>
    </button>
    <span class="min-w-0 flex-1" />
    <div class="flex items-center gap-1">
      <NuiIcon name="dot" :size="16" :class="props.connected ? 'text-nui-muted' : 'text-nui-pink'" />
      <p class="whitespace-nowrap">{{ props.statusText }}</p>
    </div>
  </footer>
</template>
