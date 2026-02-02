<template>
  <UiModal v-model="isOpen" title="Open Workspace" size="md">
    <div class="space-y-4">
      <!-- Search -->
      <div class="relative">
        <Search class="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-nanna-text-dim" />
        <input
          v-model="searchQuery"
          type="text"
          placeholder="Search workspaces..."
          class="w-full pl-10 pr-4 py-2 rounded-lg bg-nanna-bg-elevated border border-nanna-primary/20 text-nanna-text text-sm focus:outline-none focus:border-nanna-accent"
        />
      </div>

      <!-- Workspace List -->
      <div class="max-h-[300px] overflow-y-auto space-y-1">
        <!-- No workspaces -->
        <div v-if="filteredWorkspaces.length === 0 && !searchQuery" class="text-center py-8">
          <FolderPlus class="w-10 h-10 mx-auto text-nanna-text-dim mb-2" />
          <p class="text-sm text-nanna-text-muted">No workspaces available</p>
          <p class="text-xs text-nanna-text-dim mt-1">
            Go to the Workspaces page to create or open one
          </p>
        </div>

        <!-- No search results -->
        <div v-else-if="filteredWorkspaces.length === 0" class="text-center py-8">
          <Search class="w-10 h-10 mx-auto text-nanna-text-dim mb-2" />
          <p class="text-sm text-nanna-text-muted">No matching workspaces</p>
        </div>

        <!-- Workspace items -->
        <button
          v-for="ws in filteredWorkspaces"
          :key="ws.id"
          :class="[
            'w-full flex items-center gap-3 p-3 rounded-lg text-left transition-colors',
            isAlreadyOpen(ws.id) 
              ? 'bg-nanna-accent/10 border border-nanna-accent/20 cursor-default'
              : 'hover:bg-nanna-bg-elevated'
          ]"
          :disabled="isAlreadyOpen(ws.id)"
          @click="selectWorkspace(ws)"
        >
          <div :class="[
            'w-10 h-10 rounded-lg flex items-center justify-center shrink-0',
            isAlreadyOpen(ws.id) ? 'bg-nanna-accent/20' : 'bg-nanna-bg-elevated'
          ]">
            <Folder :class="[
              'w-5 h-5',
              isAlreadyOpen(ws.id) ? 'text-nanna-accent' : 'text-nanna-text-muted'
            ]" />
          </div>
          
          <div class="flex-1 min-w-0">
            <div class="flex items-center gap-2">
              <span :class="[
                'font-medium truncate',
                isAlreadyOpen(ws.id) ? 'text-nanna-accent' : 'text-nanna-text'
              ]">
                {{ ws.name }}
              </span>
              <UiBadge v-if="isAlreadyOpen(ws.id)" variant="accent" class="shrink-0 text-[10px]">
                Open
              </UiBadge>
            </div>
            <div class="text-xs text-nanna-text-dim truncate">{{ ws.path }}</div>
          </div>

          <Check v-if="isAlreadyOpen(ws.id)" class="w-4 h-4 text-nanna-accent shrink-0" />
        </button>
      </div>

      <!-- Footer with link to workspaces page -->
      <div class="pt-2 border-t border-nanna-primary/10">
        <NuxtLink 
          to="/workspaces" 
          class="flex items-center justify-center gap-2 text-sm text-nanna-text-muted hover:text-nanna-accent transition-colors"
          @click="isOpen = false"
        >
          <Settings class="w-4 h-4" />
          <span>Manage Workspaces</span>
        </NuxtLink>
      </div>
    </div>
  </UiModal>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { Search, Folder, FolderPlus, Check, Settings } from 'lucide-vue-next'

interface WorkspaceInfo {
  id: string
  name: string
  path: string
  active?: boolean
}

const props = defineProps<{
  openTabIds: string[]
}>()

const emit = defineEmits<{
  select: [workspace: WorkspaceInfo]
}>()

const isOpen = defineModel<boolean>({ default: false })
const searchQuery = ref('')
const allWorkspaces = ref<WorkspaceInfo[]>([])

const filteredWorkspaces = computed(() => {
  if (!searchQuery.value) return allWorkspaces.value
  const q = searchQuery.value.toLowerCase()
  return allWorkspaces.value.filter(ws => 
    ws.name.toLowerCase().includes(q) || ws.path.toLowerCase().includes(q)
  )
})

function isAlreadyOpen(id: string): boolean {
  return props.openTabIds.includes(id)
}

function selectWorkspace(ws: WorkspaceInfo) {
  if (isAlreadyOpen(ws.id)) return
  emit('select', ws)
  isOpen.value = false
}

async function loadWorkspaces() {
  try {
    allWorkspaces.value = await invoke<WorkspaceInfo[]>('list_workspaces')
  } catch (e) {
    console.error('Failed to load workspaces:', e)
  }
}

// Reload when modal opens
watch(isOpen, (open) => {
  if (open) {
    searchQuery.value = ''
    loadWorkspaces()
  }
})

onMounted(() => {
  loadWorkspaces()
})
</script>
