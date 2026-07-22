<template>
  <Teleport to="body">
    <Transition name="cmdk">
      <div
        v-if="open"
        class="cmdk-root"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        @keydown="onKeydown"
      >
        <div class="cmdk-backdrop" @click="emit('close')" />
        <div class="cmdk-panel" @click.stop>
          <div class="cmdk-input-row">
            <Search class="cmdk-search-icon" :size="16" aria-hidden="true" />
            <input
              ref="inputRef"
              v-model="query"
              type="text"
              class="cmdk-input"
              placeholder="Type a command or search…"
              autocomplete="off"
              spellcheck="false"
              aria-label="Search commands"
              @keydown.down.prevent="move(1)"
              @keydown.up.prevent="move(-1)"
              @keydown.enter.prevent="runSelected"
              @keydown.esc.prevent="emit('close')"
            />
            <kbd class="cmdk-esc">esc</kbd>
          </div>

          <div ref="listRef" class="cmdk-list" role="listbox">
            <template v-if="grouped.length">
              <div v-for="section in grouped" :key="section.group" class="cmdk-group">
                <div class="cmdk-group-label">{{ section.group }}</div>
                <button
                  v-for="action in section.items"
                  :key="action.id"
                  type="button"
                  role="option"
                  class="cmdk-item"
                  :class="{ 'cmdk-item--active': action.id === activeId }"
                  :aria-selected="action.id === activeId"
                  @mouseenter="activeId = action.id"
                  @click="run(action)"
                >
                  <span class="cmdk-item-label">{{ action.label }}</span>
                  <kbd v-if="action.shortcut" class="cmdk-chip">{{ action.shortcut }}</kbd>
                </button>
              </div>
            </template>
            <div v-else class="cmdk-empty">No matching commands</div>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { Search } from 'lucide-vue-next'
import { computed, nextTick, ref, watch } from 'vue'
import type { PaletteAction } from '~/lib/commandPalette'
import { filterActions } from '~/lib/commandPalette'
import { pushEscapeHandler } from '~/composables/useShortcuts'

const props = defineProps<{
  open: boolean
  actions: PaletteAction[]
}>()

const emit = defineEmits<{
  close: []
  run: [action: PaletteAction]
}>()

const query = ref('')
const activeId = ref<string | null>(null)
const inputRef = ref<HTMLInputElement | null>(null)
const listRef = ref<HTMLElement | null>(null)

const filtered = computed(() => filterActions(props.actions, query.value))

const grouped = computed(() => {
  const map = new Map<string, PaletteAction[]>()
  const order: string[] = []
  for (const a of filtered.value) {
    if (!map.has(a.group)) {
      map.set(a.group, [])
      order.push(a.group)
    }
    map.get(a.group)!.push(a)
  }
  return order.map((group) => ({ group, items: map.get(group)! }))
})

const flatIds = computed(() => filtered.value.map((a) => a.id))

function ensureActive() {
  if (!flatIds.value.length) {
    activeId.value = null
    return
  }
  if (!activeId.value || !flatIds.value.includes(activeId.value)) {
    activeId.value = flatIds.value[0] ?? null
  }
}

function move(delta: number) {
  const ids = flatIds.value
  if (!ids.length) return
  const idx = activeId.value ? ids.indexOf(activeId.value) : -1
  const next = idx < 0 ? 0 : (idx + delta + ids.length) % ids.length
  activeId.value = ids[next] ?? null
  nextTick(() => {
    const el = listRef.value?.querySelector('.cmdk-item--active') as HTMLElement | null
    el?.scrollIntoView({ block: 'nearest' })
  })
}

function run(action: PaletteAction) {
  emit('run', action)
}

function runSelected() {
  const action = filtered.value.find((a) => a.id === activeId.value)
  if (action) run(action)
}

function onKeydown(e: KeyboardEvent) {
  // Keep focus traps minimal — Escape handled on input + Esc stack.
  if (e.key === 'Tab') e.preventDefault()
}

let popEscape: (() => void) | null = null

watch(
  () => props.open,
  async (isOpen) => {
    if (isOpen) {
      query.value = ''
      ensureActive()
      popEscape?.()
      popEscape = pushEscapeHandler(() => emit('close'))
      await nextTick()
      inputRef.value?.focus()
    } else {
      popEscape?.()
      popEscape = null
    }
  },
)

watch(filtered, () => ensureActive())

</script>

<style scoped>
.cmdk-root {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: flex;
  align-items: flex-start;
  justify-content: center;
  padding: 12vh 1rem 2rem;
}

.cmdk-backdrop {
  position: absolute;
  inset: 0;
  background: rgba(15, 23, 42, 0.72);
  backdrop-filter: blur(4px);
}

.cmdk-panel {
  position: relative;
  width: 100%;
  max-width: 560px;
  max-height: min(70vh, 480px);
  display: flex;
  flex-direction: column;
  border-radius: 12px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  background: linear-gradient(165deg, rgba(41, 45, 62, 0.98), rgba(30, 34, 48, 0.98));
  box-shadow:
    0 0 0 1px rgba(139, 92, 246, 0.08),
    0 24px 64px rgba(0, 0, 0, 0.55);
  overflow: hidden;
}

.cmdk-input-row {
  display: flex;
  align-items: center;
  gap: 0.625rem;
  padding: 0.875rem 1rem;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
}

.cmdk-search-icon {
  flex-shrink: 0;
  color: rgba(196, 205, 214, 0.45);
}

.cmdk-input {
  flex: 1;
  min-width: 0;
  background: transparent;
  border: none;
  outline: none;
  color: #e2e8f0;
  font-size: 0.9375rem;
  line-height: 1.4;
}
.cmdk-input::placeholder {
  color: rgba(196, 205, 214, 0.4);
}

.cmdk-esc {
  flex-shrink: 0;
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 0.65rem;
  padding: 0.15rem 0.4rem;
  border-radius: 4px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  color: rgba(196, 205, 214, 0.45);
  background: rgba(255, 255, 255, 0.03);
}

.cmdk-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.5rem;
}

.cmdk-group + .cmdk-group {
  margin-top: 0.35rem;
}

.cmdk-group-label {
  padding: 0.35rem 0.625rem 0.25rem;
  font-size: 0.65rem;
  font-weight: 600;
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: rgba(196, 205, 214, 0.4);
}

.cmdk-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  width: 100%;
  padding: 0.55rem 0.625rem;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: #c4cdd6;
  font-size: 0.875rem;
  text-align: left;
  cursor: pointer;
  transition: background 0.1s, color 0.1s;
}
.cmdk-item:hover,
.cmdk-item--active {
  background: rgba(139, 92, 246, 0.16);
  color: #e2e8f0;
}

.cmdk-item-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.cmdk-chip {
  flex-shrink: 0;
  font-family: var(--font-mono, ui-monospace, monospace);
  font-size: 0.65rem;
  padding: 0.12rem 0.4rem;
  border-radius: 4px;
  border: 1px solid rgba(255, 255, 255, 0.1);
  color: rgba(196, 205, 214, 0.55);
  background: rgba(255, 255, 255, 0.04);
}

.cmdk-empty {
  padding: 2rem 1rem;
  text-align: center;
  font-size: 0.875rem;
  color: rgba(196, 205, 214, 0.45);
}

.cmdk-enter-active,
.cmdk-leave-active {
  transition: opacity 0.12s ease;
}
.cmdk-enter-active .cmdk-panel,
.cmdk-leave-active .cmdk-panel {
  transition: transform 0.12s ease, opacity 0.12s ease;
}
.cmdk-enter-from,
.cmdk-leave-to {
  opacity: 0;
}
.cmdk-enter-from .cmdk-panel,
.cmdk-leave-to .cmdk-panel {
  opacity: 0;
  transform: translateY(-6px) scale(0.98);
}
</style>
