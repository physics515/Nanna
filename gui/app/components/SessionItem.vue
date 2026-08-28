<template>
  <div class="group/session relative">
    <button
      type="button"
      @click="$emit('select', session)"
      @contextmenu.prevent="showMenu = true"
      :class="['session-btn flex w-full flex-col items-start gap-2 border-l border-solid p-4 text-left transition-colors hover:bg-white/5', isActive ? 'border-nui-accent bg-white/5' : 'border-transparent']"
      :aria-current="isActive ? 'true' : undefined"
    >
      <div class="flex w-full items-center gap-2">
        <span class="session-name min-w-0 flex-1 overflow-hidden text-ellipsis break-words text-xs font-semibold leading-normal text-nui-fg line-clamp-2">{{ session.name }}</span>
        <SessionActivityBadge :session-id="session.id" compact class="shrink-0" />
      </div>
      <div class="flex w-full min-w-0 items-center gap-2">
        <span class="shrink-0 text-xs leading-normal text-nui-muted">{{ formatDate(session.updated_at) }}</span>
        <!-- The sidebar is where a user notices that one chat runs on a
             different model. Read-only: the pin is changed in the chat
             header, and it covers chat replies only. -->
        <span
          v-if="pinnedModel"
          class="session-model min-w-0 truncate text-xs leading-normal text-nui-info"
          :title="`This chat is pinned to ${pinnedModel}`"
        >{{ modelDisplayName(pinnedModel) }}</span>
      </div>
    </button>
    <button
      type="button"
      @click.stop="showMenu = !showMenu"
      class="session-menu-btn absolute right-1 top-3 z-[2] p-2 text-nui-muted opacity-0 transition-opacity hover:text-nui-fg focus-visible:opacity-100 group-hover/session:opacity-100"
      aria-label="Session menu"
      title="Session menu"
      :aria-expanded="showMenu ? 'true' : 'false'"
    >
      <NuiIcon name="overflow-menu" :size="16" />
    </button>

    <!-- Context Menu -->
    <Transition name="menu">
      <div
        v-if="showMenu"
        v-click-outside="() => showMenu = false"
        class="absolute right-0 top-full z-50 w-36 overflow-hidden border border-solid border-nui-muted/40 bg-nui-bg p-1"
      >
        <button @click="startRename" class="ctx-item block w-full px-2.5 py-1.5 text-left text-xs text-nui-fg transition-colors hover:bg-white/5">Rename</button>
        <button @click="confirmDelete" class="ctx-item ctx-danger block w-full px-2.5 py-1.5 text-left text-xs text-nui-pink transition-colors hover:bg-nui-pink/10">Delete</button>
      </div>
    </Transition>

    <!-- Rename Modal -->
    <Teleport to="body">
      <Transition name="modal">
        <div v-if="isRenaming" class="rename-overlay fixed inset-0 z-50 flex items-center justify-center bg-black/50">
          <div class="rename-dialog w-96 border border-solid border-nui-muted/40 bg-nui-bg p-6 font-nui">
            <h3 class="mb-4 text-sm font-semibold leading-normal text-nui-fg">Rename Session</h3>
            <input
              v-model="newName"
              ref="renameInput"
              @keydown.enter="saveRename"
              @keydown.escape="isRenaming = false"
              class="mb-4 w-full border border-solid border-nui-muted/40 bg-transparent p-2 text-xs leading-normal text-nui-fg outline-none placeholder:text-nui-muted focus:border-nui-accent"
              placeholder="Session name"
            />
            <div class="flex justify-end gap-2">
              <button @click="isRenaming = false" class="px-4 py-2 text-xs text-nui-muted transition-colors hover:text-nui-fg">Cancel</button>
              <button @click="saveRename" class="rounded-lg bg-nui-accent px-4 py-2 text-xs text-nui-fg transition-opacity disabled:opacity-50" :disabled="!newName.trim()">Save</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { modelDisplayName } from '~/lib/modelSpecs'
import { knownChatModel } from '~/composables/useSessionState'
import { useConfirm } from '~/composables/useConfirm'
const { confirm } = useConfirm()

interface SessionInfo {
  id: string
  name: string
  created_at: string
  updated_at: string
  message_count: number
  workspace_id: string | null
  workspace_name: string | null
  /** Model this chat is pinned to; absent/null = the global `[llm]` default. */
  chat_model?: string | null
}

const props = defineProps<{
  session: SessionInfo
  isActive: boolean
}>()

const emit = defineEmits<{
  (e: 'select', session: SessionInfo): void
  (e: 'deleted', sessionId: string): void
  (e: 'renamed', session: SessionInfo): void
}>()

/**
 * The pin to show. `session` comes from the layout's list, which is only
 * reloaded at mount, on `sessions-cleared`, and on a workspace-tab change — so
 * on its own it goes on claiming a pin the header has since removed. Where
 * this window has its own answer for the chat, that one is newer and wins.
 */
const pinnedModel = computed(() => {
  const known = knownChatModel(props.session.id)
  return known === undefined ? (props.session.chat_model ?? null) : known
})

const showMenu = ref(false)
const isRenaming = ref(false)
const newName = ref('')
const renameInput = ref<HTMLInputElement | null>(null)

function startRename() {
  showMenu.value = false
  newName.value = props.session.name
  isRenaming.value = true
  nextTick(() => {
    renameInput.value?.focus()
    renameInput.value?.select()
  })
}

async function saveRename() {
  if (!newName.value.trim()) return
  try {
    await invoke('rename_session', {
      sessionId: props.session.id,
      name: newName.value.trim()
    })
    emit('renamed', { ...props.session, name: newName.value.trim() })
    isRenaming.value = false
  } catch (e) { console.error('Failed to rename:', e) }
}

async function confirmDelete() {
  showMenu.value = false
  const confirmed = await confirm({
    title: 'Delete Session',
    message: `Delete "${props.session.name}"? This cannot be undone.`,
    confirmLabel: 'Delete',
    danger: true
  })
  if (!confirmed) return
  try {
    await invoke('delete_session', { sessionId: props.session.id })
    emit('deleted', props.session.id)
  } catch (e) { console.error('Failed to delete:', e) }
}

function formatDate(dateStr: string): string {
  const date = new Date(dateStr)
  const now = new Date()
  const diff = now.getTime() - date.getTime()
  if (diff < 60000) return 'Just now'
  if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`
  if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`
  if (diff < 604800000) return `${Math.floor(diff / 86400000)}d ago`
  return date.toLocaleDateString()
}

interface ClickOutsideElement extends HTMLElement {
  _clickOutside?: (event: MouseEvent) => void
}

const vClickOutside = {
  mounted(el: ClickOutsideElement, binding: any) {
    el._clickOutside = (event: MouseEvent) => {
      if (!(el === event.target || el.contains(event.target as Node))) {
        binding.value()
      }
    }
    document.addEventListener('click', el._clickOutside)
  },
  unmounted(el: ClickOutsideElement) {
    if (el._clickOutside) document.removeEventListener('click', el._clickOutside)
  }
}
</script>

<style scoped>
/* Transitions */
.menu-enter-active, .menu-leave-active { transition: all 0.15s ease; }
.menu-enter-from, .menu-leave-to { opacity: 0; transform: scale(0.95); }
.modal-enter-active, .modal-leave-active { transition: all 0.2s ease; }
.modal-enter-from, .modal-leave-to { opacity: 0; }
</style>
