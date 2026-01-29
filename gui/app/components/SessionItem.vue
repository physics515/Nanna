<template>
  <div class="relative group">
    <button 
      @click="$emit('select', session)"
      @contextmenu.prevent="showMenu = true"
      :class="[
        'w-full text-left px-3 py-2 rounded-lg transition-colors text-sm',
        isActive 
          ? 'bg-nanna-primary/20 text-nanna-text border-l-2 border-nanna-primary' 
          : 'text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated'
      ]"
    >
      <div class="flex items-center justify-between">
        <span class="truncate flex-1">{{ session.name }}</span>
        <button 
          @click.stop="showMenu = !showMenu"
          class="opacity-0 group-hover:opacity-100 p-1 hover:bg-nanna-bg-elevated rounded transition-opacity"
        >
          ⋮
        </button>
      </div>
      <div class="text-xs text-nanna-text-dim mt-0.5">
        {{ formatDate(session.updated_at) }}
      </div>
    </button>
    
    <!-- Context Menu -->
    <Transition name="menu">
      <div 
        v-if="showMenu"
        v-click-outside="() => showMenu = false"
        class="absolute right-0 top-0 mt-8 w-40 bg-nanna-bg-surface border border-nanna-primary/20 rounded-lg shadow-lg z-50 overflow-hidden"
      >
        <button 
          @click="startRename"
          class="w-full text-left px-3 py-2 text-sm text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
        >
          ✏️ Rename
        </button>
        <button 
          @click="confirmDelete"
          class="w-full text-left px-3 py-2 text-sm text-nanna-error hover:bg-nanna-error/10 transition-colors"
        >
          🗑️ Delete
        </button>
      </div>
    </Transition>
    
    <!-- Rename Modal -->
    <Teleport to="body">
      <Transition name="modal">
        <div v-if="isRenaming" class="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div class="bg-nanna-bg-surface border border-nanna-primary/20 rounded-xl p-6 w-96 shadow-xl">
            <h3 class="text-lg font-semibold text-nanna-text mb-4">Rename Session</h3>
            <input 
              v-model="newName"
              ref="renameInput"
              @keydown.enter="saveRename"
              @keydown.escape="isRenaming = false"
              class="input mb-4"
              placeholder="Session name"
            />
            <div class="flex justify-end gap-2">
              <button @click="isRenaming = false" class="btn-ghost">
                Cancel
              </button>
              <button @click="saveRename" class="btn-primary" :disabled="!newName.trim()">
                Save
              </button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick } from 'vue'
import { invoke } from '@tauri-apps/api/core'

interface SessionInfo {
  id: string
  name: string
  created_at: string
  updated_at: string
  message_count: number
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
  } catch (e) {
    console.error('Failed to rename:', e)
  }
}

async function confirmDelete() {
  showMenu.value = false
  
  if (!confirm(`Delete "${props.session.name}"? This cannot be undone.`)) return
  
  try {
    await invoke('delete_session', { sessionId: props.session.id })
    emit('deleted', props.session.id)
  } catch (e) {
    console.error('Failed to delete:', e)
  }
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

// Click outside directive
const vClickOutside = {
  mounted(el: HTMLElement, binding: any) {
    el._clickOutside = (event: MouseEvent) => {
      if (!(el === event.target || el.contains(event.target as Node))) {
        binding.value()
      }
    }
    document.addEventListener('click', el._clickOutside)
  },
  unmounted(el: HTMLElement) {
    document.removeEventListener('click', el._clickOutside)
  }
}
</script>

<style scoped>
.menu-enter-active,
.menu-leave-active {
  transition: all 0.15s ease;
}

.menu-enter-from,
.menu-leave-to {
  opacity: 0;
  transform: scale(0.95);
}

.modal-enter-active,
.modal-leave-active {
  transition: all 0.2s ease;
}

.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}
</style>
