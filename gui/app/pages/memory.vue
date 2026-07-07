<template>
  <div class="flex flex-col h-full relative overflow-hidden">

    <!-- Header -->
    <header class="relative z-10 px-4 sm:px-6 py-3 sm:py-4 border-b border-white/[0.04]">
      <div class="flex items-center justify-between">
        <div>
          <h2 class="text-base sm:text-lg font-semibold text-white/90">Memory</h2>
          <p class="text-xs sm:text-sm text-white/40">
            {{ semanticMemories.length }} memories
            <span v-if="memoryFilter"> · filtered</span>
          </p>
        </div>

        <div class="flex items-center gap-2">
          <!-- Scope Toggle -->
          <UiGlassTabs v-model="memoryScope" :tabs="scopeItems" size="xs" active-color="accent" />
        </div>
      </div>
    </header>

    <!-- Actions bar -->
    <div class="px-4 sm:px-6 py-3 border-b border-white/[0.04] flex items-center justify-between">
      <div class="flex items-center gap-2">
        <UiSplatterTextarea
          v-model="memoryFilter"
          placeholder="Filter memories..."
          :rows="1"
          class="w-48 sm:w-64"
        />
      </div>
      <div class="flex items-center gap-2">
        <UiGlassButton
          size="xs"
          color="default"
          pill
          :disabled="isDreaming"
          @click="triggerDream"
        >
          {{ isDreaming ? '💭 Dreaming...' : '💭 Dream Now' }}
        </UiGlassButton>
        <UiGlassButton
          v-if="semanticMemories.length > 0"
          size="xs"
          color="danger"
          pill
          @click="clearAllMemories"
        >
          🗑️ Clear All
        </UiGlassButton>
      </div>
    </div>

    <!-- Memory list -->
    <div class="relative z-10 flex-1 overflow-y-auto overflow-x-hidden">
      <div class="p-4 sm:p-6">
        <!-- Empty state -->
        <div v-if="filteredMemories.length === 0" class="flex items-center justify-center min-h-[300px] sm:min-h-[400px]">
          <div class="text-center max-w-md px-4">
            <div class="text-5xl sm:text-6xl mb-4">🧠</div>
            <h3 class="text-lg sm:text-xl font-semibold text-white/80 mb-2">
              {{ memoryFilter ? 'No matching memories' : 'No memories yet' }}
            </h3>
            <p class="text-sm text-white/40">
              {{ memoryFilter ? 'Try a different filter.' : 'Memories are created automatically from conversations and can be added manually.' }}
            </p>
          </div>
        </div>

        <!-- Memories -->
        <div v-else class="space-y-3">
          <div
            v-for="memory in filteredMemories"
            :key="memory.id"
            class="group"
          >
            <!-- Meta row -->
            <div class="flex items-center justify-between gap-3 mb-1 px-1">
              <div class="flex items-center gap-3 text-xs text-white/20">
                <span>{{ formatDate(memory.created_at) }}</span>
                <span v-if="memory.state && memory.state !== 'active'" class="text-amber-300/40">{{ memory.state }}</span>
                <span v-if="memory.importance" class="text-amber-300/40">
                  {{ '★'.repeat(Math.min(Math.round(memory.importance), 5)) }}
                </span>
              </div>
              <div class="flex items-center gap-1">
                <button
                  v-if="editingMemoryId !== memory.id"
                  @click="startEditing(memory)"
                  class="opacity-0 group-hover:opacity-100 text-white/20 hover:text-white/60 transition-all p-1"
                  title="Edit memory"
                >
                  <Pencil class="w-3.5 h-3.5" />
                </button>
                <UiGlassButton
                  v-if="editingMemoryId === memory.id"
                  size="xs"
                  color="accent"
                  pill
                  @click="saveEditing(memory.id)"
                >
                  Save
                </UiGlassButton>
                <UiGlassButton
                  v-if="editingMemoryId === memory.id"
                  size="xs"
                  color="default"
                  pill
                  @click="cancelEditing"
                >
                  Cancel
                </UiGlassButton>
                <button
                  @click="deleteMemory(memory.id)"
                  class="opacity-0 group-hover:opacity-100 text-white/20 hover:text-red-400/60 transition-all p-1"
                  title="Delete memory"
                >
                  <Trash2 class="w-3.5 h-3.5" />
                </button>
              </div>
            </div>
            <!-- Rich text editor -->
            <MemoryEditor
              :model-value="editingMemoryId === memory.id ? editBuffers[memory.id] : memory.content"
              :editable="editingMemoryId === memory.id"
              placeholder="Write a memory..."
              @update:model-value="editBuffers[memory.id] = $event"
            />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted, watch, inject } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Globe, FolderKanban, Trash2, Pencil } from 'lucide-vue-next'

// Scope tabs
const currentWorkspace = inject<any>('currentWorkspace', ref(null))
const memoryScope = ref<string>('global')

const scopeItems = computed(() => {
  const items: { id: string; label: string; icon?: any }[] = [
    { id: 'global', label: 'Global', icon: Globe },
  ]
  if (currentWorkspace?.value) {
    items.push({ id: 'workspace', label: currentWorkspace.value.name || 'Workspace', icon: FolderKanban })
  } else {
    items.push({ id: 'workspace', label: 'Workspace', icon: FolderKanban })
  }
  return items
})

// State
const memoryFilter = ref('')
const isDreaming = ref(false)
const editingMemoryId = ref<string | null>(null)
const editBuffers = reactive<Record<string, string>>({})
const semanticMemories = ref<any[]>([])

function startEditing(memory: any) {
  editingMemoryId.value = memory.id
  editBuffers[memory.id] = memory.content
}

function cancelEditing() {
  editingMemoryId.value = null
}

async function saveEditing(id: string) {
  const content = editBuffers[id]
  if (content == null) return
  try {
    await invoke('update_memory', { id, content })
    const mem = semanticMemories.value.find((m: any) => m.id === id)
    if (mem) mem.content = content
  } catch (e) {
    console.error('Failed to update memory:', e)
  }
  editingMemoryId.value = null
}

const filteredMemories = computed(() => {
  if (!memoryFilter.value) return semanticMemories.value
  const q = memoryFilter.value.toLowerCase()
  return semanticMemories.value.filter((m: any) =>
    m.content?.toLowerCase().includes(q)
  )
})

async function fetchMemories() {
  try {
    const res = await invoke('list_memories', {
      scope: memoryScope.value,
      workspaceId: currentWorkspace?.value?.id,
    })
    semanticMemories.value = res as any[]
  } catch (e) {
    console.error('Failed to fetch memories:', e)
  }
}

async function deleteMemory(id: string) {
  try {
    await invoke('delete_memory', { id })
    semanticMemories.value = semanticMemories.value.filter((m: any) => m.id !== id)
  } catch (e) {
    console.error('Failed to delete memory:', e)
  }
}

async function triggerDream() {
  isDreaming.value = true
  try {
    await invoke('trigger_consolidation')
    await fetchMemories()
  } catch (e) {
    console.error('Dream failed:', e)
  } finally {
    isDreaming.value = false
  }
}

async function clearAllMemories() {
  if (!confirm('Are you sure you want to clear all memories? This cannot be undone.')) return
  try {
    await invoke('clear_memories', {
      scope: memoryScope.value,
      workspaceId: currentWorkspace?.value?.id,
    })
    semanticMemories.value = []
  } catch (e) {
    console.error('Failed to clear memories:', e)
  }
}

function formatDate(ts: string | number): string {
  if (!ts) return ''
  const d = new Date(typeof ts === 'number' ? ts * 1000 : ts)
  const now = new Date()
  const diff = now.getTime() - d.getTime()
  if (diff < 60000) return 'just now'
  if (diff < 3600000) return `${Math.floor(diff / 60000)}m ago`
  if (diff < 86400000) return `${Math.floor(diff / 3600000)}h ago`
  if (diff < 604800000) return `${Math.floor(diff / 86400000)}d ago`
  return d.toLocaleDateString()
}

watch(memoryScope, () => {
  fetchMemories()
})

onMounted(() => {
  fetchMemories()
})
</script>
