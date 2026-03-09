<template>
  <div class="session-item-wrap">
    <button
      @click="$emit('select', session)"
      @contextmenu.prevent="showMenu = true"
      :class="['session-btn', { active: isActive }]"
      @mouseenter="!isActive && meshEnter()"
      @mouseleave="!isActive && meshLeave()"
    >
      <!-- Splatter mesh (visible when active, hover preview when inactive) -->
      <span class="session-mesh" :style="{ background: splatterBg, opacity: isActive ? 1 : 0 }" />

      <div class="session-content">
        <div class="session-row">
          <span class="session-name">{{ session.name }}</span>
          <div class="session-actions">
            <SessionActivityBadge :session-id="session.id" compact />
            <button
              @click.stop="showMenu = !showMenu"
              class="session-menu-btn"
            >
              <svg viewBox="0 0 4 16" fill="currentColor" style="width: 4px; height: 12px;">
                <circle cx="2" cy="2" r="1.5" />
                <circle cx="2" cy="8" r="1.5" />
                <circle cx="2" cy="14" r="1.5" />
              </svg>
            </button>
          </div>
        </div>
        <div class="session-date">{{ formatDate(session.updated_at) }}</div>
      </div>
    </button>

    <!-- Context Menu -->
    <Transition name="menu">
      <div
        v-if="showMenu"
        v-click-outside="() => showMenu = false"
        class="session-context-menu"
      >
        <button @click="startRename" class="ctx-item">Rename</button>
        <button @click="confirmDelete" class="ctx-item ctx-danger">Delete</button>
      </div>
    </Transition>

    <!-- Rename Modal -->
    <Teleport to="body">
      <Transition name="modal">
        <div v-if="isRenaming" class="rename-overlay">
          <div class="rename-dialog">
            <h3 style="font-size: 1rem; font-weight: 600; color: #e2e8f0; margin-bottom: 1rem;">Rename Session</h3>
            <input
              v-model="newName"
              ref="renameInput"
              @keydown.enter="saveRename"
              @keydown.escape="isRenaming = false"
              class="input"
              placeholder="Session name"
              style="margin-bottom: 1rem;"
            />
            <div style="display: flex; justify-content: flex-end; gap: 0.5rem;">
              <button @click="isRenaming = false" class="btn-ghost">Cancel</button>
              <button @click="saveRename" class="btn-primary" :disabled="!newName.trim()">Save</button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { useConfirm } from '~/composables/useConfirm'
import { useSplatter } from '~/composables/useSplatter'

const { confirm } = useConfirm()

interface SessionInfo {
  id: string
  name: string
  created_at: string
  updated_at: string
  message_count: number
  workspace_id: string | null
  workspace_name: string | null
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

// Splatter for active state — violet/indigo palette
const { splatterBg, onEnter: meshEnter, onLeave: meshLeave } = useSplatter({
  colors: ['139,92,246', '129,140,248', '99,102,241'],
  opacityRanges: [[0.15, 0.25], [0.12, 0.2], [0.08, 0.15]],
  sizes: ['65%', '60%', '50%'],
})

// Keep splatter animating while active
watch(() => props.isActive, (active) => {
  if (active) meshEnter()
  else meshLeave()
}, { immediate: true })

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
    confirmText: 'Delete',
    destructive: true
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
.session-item-wrap {
  position: relative;
}

.session-btn {
  position: relative;
  display: block;
  width: 100%;
  text-align: left;
  padding: 8px 10px;
  border-radius: 8px;
  border: none;
  background: transparent;
  cursor: pointer;
  overflow: hidden;
  isolation: isolate;
  transition: color 0.15s;
}

/* Splatter mesh layer */
.session-mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  border-radius: inherit;
  transition: opacity 0.25s ease;
  -webkit-mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  -webkit-mask-size: 200px 200px;
  mask-size: 200px 200px;
}

/* Hover: show mesh faintly */
.session-btn:hover .session-mesh {
  opacity: 0.4 !important;
}
.session-btn.active .session-mesh {
  opacity: 1 !important;
}

.session-content {
  position: relative;
  z-index: 1;
}

.session-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.session-name {
  font-size: 0.8rem;
  color: rgba(196, 205, 214, 0.7);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
  min-width: 0;
}
.session-btn.active .session-name {
  color: #e2e8f0;
  font-weight: 500;
}

.session-actions {
  display: flex;
  align-items: center;
  gap: 4px;
  flex-shrink: 0;
}

.session-menu-btn {
  opacity: 0;
  padding: 4px 6px;
  border-radius: 4px;
  border: none;
  background: transparent;
  color: rgba(196, 205, 214, 0.5);
  cursor: pointer;
  transition: opacity 0.15s, background 0.15s, color 0.15s;
}
.session-btn:hover .session-menu-btn {
  opacity: 1;
}
.session-menu-btn:hover {
  background: rgba(255, 255, 255, 0.06);
  color: #e2e8f0;
}

.session-date {
  font-size: 0.65rem;
  color: #64748b;
  margin-top: 2px;
}
.session-btn.active .session-date {
  color: rgba(139, 92, 246, 0.6);
}

/* Context menu */
.session-context-menu {
  position: absolute;
  right: 0;
  top: 100%;
  width: 140px;
  background: #1e293b;
  border: 1px solid rgba(255, 255, 255, 0.06);
  border-radius: 8px;
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
  z-index: 50;
  overflow: hidden;
  padding: 4px;
}
.ctx-item {
  display: block;
  width: 100%;
  text-align: left;
  padding: 6px 10px;
  border: none;
  background: transparent;
  color: #e2e8f0;
  font-size: 0.8rem;
  border-radius: 4px;
  cursor: pointer;
  transition: background 0.1s;
}
.ctx-item:hover { background: rgba(255, 255, 255, 0.06); }
.ctx-danger { color: #fb7185; }
.ctx-danger:hover { background: rgba(251, 113, 133, 0.1); }

/* Rename overlay */
.rename-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 50;
}
.rename-dialog {
  background: #1e293b;
  border: 1px solid rgba(255, 255, 255, 0.06);
  border-radius: 12px;
  padding: 1.5rem;
  width: 360px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.5);
}

/* Transitions */
.menu-enter-active, .menu-leave-active { transition: all 0.15s ease; }
.menu-enter-from, .menu-leave-to { opacity: 0; transform: scale(0.95); }
.modal-enter-active, .modal-leave-active { transition: all 0.2s ease; }
.modal-enter-from, .modal-leave-to { opacity: 0; }
</style>
