<template>
  <div class="shortcuts-help" role="region" aria-label="Keyboard shortcuts">
    <h3 class="text-sm font-medium text-nanna-text mb-2">Keyboard shortcuts</h3>
    <ul class="space-y-1.5">
      <li v-for="s in items" :key="s.key" class="flex items-center justify-between gap-3 text-xs">
        <span class="text-nanna-text-muted">{{ s.description }}</span>
        <kbd class="shortcut-kbd">{{ s.key }}</kbd>
      </li>
    </ul>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { listShortcuts } from '~/composables/useShortcuts'

const items = ref<Array<{ key: string; description: string }>>([])

onMounted(() => {
  // Bound after layout mounts its global shortcuts.
  items.value = listShortcuts()
})

// Static fallbacks so the panel is useful even before binds run.
const defaults = [
  { key: 'Ctrl/Cmd+K', description: 'Command palette (reserved)' },
  { key: 'Ctrl/Cmd+Shift+N', description: 'New chat' },
  { key: 'Ctrl/Cmd+Shift+L', description: 'Focus chat input' },
  { key: 'Ctrl/Cmd+.', description: 'Stop generation' },
  { key: 'Esc', description: 'Close topmost dialog/menu' },
]

const resolved = computed(() => (items.value.length ? items.value : defaults))
// expose resolved as items for template simplicity
items.value = defaults
</script>

<style scoped>
.shortcut-kbd {
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 0.7rem;
  padding: 0.125rem 0.4rem;
  border-radius: 0.25rem;
  border: 1px solid rgba(148, 163, 184, 0.35);
  background: rgba(15, 23, 42, 0.55);
  color: #e2e8f0;
  white-space: nowrap;
}
</style>
