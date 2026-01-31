<template>
  <div class="flex flex-col h-full">
    <!-- Header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <div class="flex items-center justify-between gap-3">
        <div>
          <h2 class="text-base sm:text-lg font-semibold text-nanna-text">Workspaces</h2>
          <p class="text-xs sm:text-sm text-nanna-text-muted">
            Project directories with AGENTS.md, SOUL.md, and other context files
          </p>
        </div>
        <div class="flex gap-2">
          <UiButton @click="openFolderDialog" variant="secondary" size="sm">
            <FolderOpen class="w-4 h-4 mr-1" />
            Open Folder
          </UiButton>
          <UiButton @click="refreshWorkspaces" variant="ghost" size="sm" :disabled="isLoading">
            <RefreshCw :class="['w-4 h-4', isLoading && 'animate-spin']" />
          </UiButton>
        </div>
      </div>
    </header>

    <!-- Content -->
    <div class="flex-1 overflow-y-auto p-4 sm:p-6">
      <div class="max-w-4xl mx-auto space-y-6">
        
        <!-- Active Workspace Banner -->
        <div v-if="activeWorkspace" class="p-4 rounded-xl bg-gradient-to-r from-nanna-accent/20 to-nanna-primary/20 border border-nanna-accent/30">
          <div class="flex items-center justify-between">
            <div class="flex items-center gap-3">
              <div class="w-10 h-10 rounded-lg bg-nanna-accent/30 flex items-center justify-center">
                <FolderCheck class="w-5 h-5 text-nanna-accent" />
              </div>
              <div>
                <div class="text-sm text-nanna-text-muted">Active Workspace</div>
                <div class="font-semibold text-nanna-text">{{ activeWorkspace.name }}</div>
              </div>
            </div>
            <div class="flex items-center gap-2">
              <UiButton @click="reloadActiveWorkspace" variant="ghost" size="sm" title="Reload context">
                <RefreshCw class="w-4 h-4" />
              </UiButton>
              <UiButton @click="viewWorkspaceDetails(activeWorkspace)" variant="secondary" size="sm">
                View Details
              </UiButton>
            </div>
          </div>
          
          <!-- Context indicators -->
          <div class="flex gap-2 mt-3 flex-wrap">
            <UiBadge v-if="activeWorkspace.has_soul" variant="accent">SOUL.md</UiBadge>
            <UiBadge v-if="activeWorkspace.has_user" variant="secondary">USER.md</UiBadge>
            <UiBadge v-if="activeWorkspace.has_agents" variant="secondary">AGENTS.md</UiBadge>
            <UiBadge v-if="activeWorkspace.has_memory" variant="secondary">MEMORY.md</UiBadge>
            <span class="text-xs text-nanna-text-dim ml-auto">
              {{ activeWorkspace.context_chars.toLocaleString() }} chars
            </span>
          </div>
        </div>

        <!-- Empty State -->
        <div v-if="workspaces.length === 0 && !isLoading" class="text-center py-12">
          <div class="w-16 h-16 mx-auto mb-4 rounded-full bg-nanna-bg-elevated flex items-center justify-center">
            <FolderPlus class="w-8 h-8 text-nanna-text-dim" />
          </div>
          <h3 class="text-lg font-semibold text-nanna-text mb-2">No workspaces open</h3>
          <p class="text-sm text-nanna-text-muted mb-4 max-w-md mx-auto">
            Open a folder containing AGENTS.md, SOUL.md, or other workspace files to get started.
          </p>
          <UiButton @click="openFolderDialog">
            <FolderOpen class="w-4 h-4 mr-2" />
            Open Workspace Folder
          </UiButton>
        </div>

        <!-- Workspace List -->
        <div v-else class="space-y-3">
          <h3 class="text-sm font-medium text-nanna-text-muted mb-2">
            {{ workspaces.length }} Workspace{{ workspaces.length !== 1 ? 's' : '' }}
          </h3>
          
          <UiCard
            v-for="ws in workspaces"
            :key="ws.id"
            :class="[
              'cursor-pointer transition-all hover:border-nanna-primary/40',
              ws.active && 'border-nanna-accent/50 bg-nanna-accent/5'
            ]"
            @click="selectWorkspace(ws)"
          >
            <div class="flex items-center gap-3">
              <!-- Icon -->
              <div :class="[
                'w-10 h-10 rounded-lg flex items-center justify-center shrink-0',
                ws.active ? 'bg-nanna-accent/30' : 'bg-nanna-bg-elevated'
              ]">
                <Folder :class="['w-5 h-5', ws.active ? 'text-nanna-accent' : 'text-nanna-text-muted']" />
              </div>
              
              <!-- Info -->
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2">
                  <span class="font-medium text-nanna-text truncate">{{ ws.name }}</span>
                  <UiBadge v-if="ws.active" variant="accent" class="shrink-0">Active</UiBadge>
                </div>
                <div class="text-xs text-nanna-text-dim truncate">{{ ws.path }}</div>
              </div>
              
              <!-- Context files -->
              <div class="hidden sm:flex gap-1 shrink-0">
                <span v-if="ws.has_soul" class="text-xs px-1.5 py-0.5 rounded bg-nanna-bg-elevated text-nanna-text-muted">S</span>
                <span v-if="ws.has_user" class="text-xs px-1.5 py-0.5 rounded bg-nanna-bg-elevated text-nanna-text-muted">U</span>
                <span v-if="ws.has_agents" class="text-xs px-1.5 py-0.5 rounded bg-nanna-bg-elevated text-nanna-text-muted">A</span>
                <span v-if="ws.has_memory" class="text-xs px-1.5 py-0.5 rounded bg-nanna-bg-elevated text-nanna-text-muted">M</span>
              </div>
              
              <!-- Actions -->
              <div class="flex gap-1 shrink-0">
                <UiButton 
                  v-if="!ws.active" 
                  @click.stop="setActive(ws)" 
                  variant="ghost" 
                  size="sm"
                  title="Set as active"
                >
                  <Play class="w-4 h-4" />
                </UiButton>
                <UiButton 
                  @click.stop="closeWorkspace(ws)" 
                  variant="ghost" 
                  size="sm"
                  title="Close workspace"
                  class="hover:text-nanna-error"
                >
                  <X class="w-4 h-4" />
                </UiButton>
              </div>
            </div>
          </UiCard>
        </div>

        <!-- Workspace Files Reference -->
        <UiCard class="mt-8">
          <h3 class="text-sm font-semibold text-nanna-accent mb-3 flex items-center gap-2">
            <FileText class="w-4 h-4" />
            Workspace Files Reference
          </h3>
          <div class="grid gap-2 text-sm">
            <div class="flex items-start gap-2 p-2 rounded bg-nanna-bg-elevated/50">
              <code class="text-nanna-accent shrink-0">SOUL.md</code>
              <span class="text-nanna-text-muted">Agent personality, identity, voice</span>
            </div>
            <div class="flex items-start gap-2 p-2 rounded bg-nanna-bg-elevated/50">
              <code class="text-nanna-accent shrink-0">USER.md</code>
              <span class="text-nanna-text-muted">Info about the user (name, preferences)</span>
            </div>
            <div class="flex items-start gap-2 p-2 rounded bg-nanna-bg-elevated/50">
              <code class="text-nanna-accent shrink-0">AGENTS.md</code>
              <span class="text-nanna-text-muted">How the agent should behave in this workspace</span>
            </div>
            <div class="flex items-start gap-2 p-2 rounded bg-nanna-bg-elevated/50">
              <code class="text-nanna-accent shrink-0">TOOLS.md</code>
              <span class="text-nanna-text-muted">Tool-specific notes and configurations</span>
            </div>
            <div class="flex items-start gap-2 p-2 rounded bg-nanna-bg-elevated/50">
              <code class="text-nanna-accent shrink-0">MEMORY.md</code>
              <span class="text-nanna-text-muted">Long-term curated memories</span>
            </div>
            <div class="flex items-start gap-2 p-2 rounded bg-nanna-bg-elevated/50">
              <code class="text-nanna-accent shrink-0">memory/</code>
              <span class="text-nanna-text-muted">Daily notes (YYYY-MM-DD.md files)</span>
            </div>
          </div>
        </UiCard>

      </div>
    </div>

    <!-- Workspace Details Modal -->
    <Teleport to="body">
      <div v-if="selectedWorkspace" class="fixed inset-0 bg-black/60 flex items-center justify-center z-50 p-4" @click.self="selectedWorkspace = null">
        <div class="bg-nanna-bg-surface rounded-xl w-full max-w-2xl max-h-[80vh] overflow-hidden border border-nanna-primary/20 shadow-2xl">
          <!-- Modal Header -->
          <div class="flex items-center justify-between p-4 border-b border-nanna-primary/10">
            <div class="flex items-center gap-3">
              <FolderCheck class="w-5 h-5 text-nanna-accent" />
              <h3 class="font-semibold text-nanna-text">{{ selectedWorkspace.name }}</h3>
            </div>
            <UiButton @click="selectedWorkspace = null" variant="ghost" size="sm">
              <X class="w-4 h-4" />
            </UiButton>
          </div>
          
          <!-- Modal Content -->
          <div class="p-4 overflow-y-auto max-h-[60vh] space-y-4">
            <!-- Path -->
            <div class="p-3 rounded-lg bg-nanna-bg-elevated/50">
              <div class="text-xs text-nanna-text-muted mb-1">Path</div>
              <code class="text-sm text-nanna-text break-all">{{ selectedWorkspace.path }}</code>
            </div>
            
            <!-- Context Files Status -->
            <div>
              <div class="text-sm font-medium text-nanna-text mb-2">Context Files</div>
              <div class="grid grid-cols-2 gap-2">
                <div :class="['p-2 rounded', selectedWorkspace.has_soul ? 'bg-nanna-success/10' : 'bg-nanna-bg-elevated/30']">
                  <span :class="selectedWorkspace.has_soul ? 'text-nanna-success' : 'text-nanna-text-dim'">
                    {{ selectedWorkspace.has_soul ? '✓' : '○' }} SOUL.md
                  </span>
                </div>
                <div :class="['p-2 rounded', selectedWorkspace.has_user ? 'bg-nanna-success/10' : 'bg-nanna-bg-elevated/30']">
                  <span :class="selectedWorkspace.has_user ? 'text-nanna-success' : 'text-nanna-text-dim'">
                    {{ selectedWorkspace.has_user ? '✓' : '○' }} USER.md
                  </span>
                </div>
                <div :class="['p-2 rounded', selectedWorkspace.has_agents ? 'bg-nanna-success/10' : 'bg-nanna-bg-elevated/30']">
                  <span :class="selectedWorkspace.has_agents ? 'text-nanna-success' : 'text-nanna-text-dim'">
                    {{ selectedWorkspace.has_agents ? '✓' : '○' }} AGENTS.md
                  </span>
                </div>
                <div :class="['p-2 rounded', selectedWorkspace.has_memory ? 'bg-nanna-success/10' : 'bg-nanna-bg-elevated/30']">
                  <span :class="selectedWorkspace.has_memory ? 'text-nanna-success' : 'text-nanna-text-dim'">
                    {{ selectedWorkspace.has_memory ? '✓' : '○' }} MEMORY.md
                  </span>
                </div>
              </div>
            </div>
            
            <!-- Stats -->
            <div class="flex gap-4 text-sm">
              <div>
                <span class="text-nanna-text-muted">Total context:</span>
                <span class="text-nanna-text ml-1 font-mono">{{ selectedWorkspace.context_chars.toLocaleString() }} chars</span>
              </div>
              <div>
                <span class="text-nanna-text-muted">~Tokens:</span>
                <span class="text-nanna-text ml-1 font-mono">{{ Math.round(selectedWorkspace.context_chars / 4).toLocaleString() }}</span>
              </div>
            </div>
          </div>
          
          <!-- Modal Footer -->
          <div class="flex justify-end gap-2 p-4 border-t border-nanna-primary/10">
            <UiButton @click="reloadWorkspaceById(selectedWorkspace.id)" variant="secondary" size="sm">
              <RefreshCw class="w-4 h-4 mr-1" />
              Reload
            </UiButton>
            <UiButton v-if="!selectedWorkspace.active" @click="setActiveAndClose(selectedWorkspace)" variant="default" size="sm">
              <Play class="w-4 h-4 mr-1" />
              Set Active
            </UiButton>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { 
  Folder, FolderOpen, FolderPlus, FolderCheck, RefreshCw, X, Play, FileText, ChevronRight 
} from 'lucide-vue-next'

interface WorkspaceInfo {
  id: string
  name: string
  path: string
  active: boolean
  has_agents: boolean
  has_soul: boolean
  has_user: boolean
  has_memory: boolean
  context_chars: number
}

const workspaces = ref<WorkspaceInfo[]>([])
const isLoading = ref(false)
const selectedWorkspace = ref<WorkspaceInfo | null>(null)

const activeWorkspace = computed(() => workspaces.value.find(ws => ws.active))

onMounted(async () => {
  await loadWorkspaces()
})

async function loadWorkspaces() {
  isLoading.value = true
  try {
    workspaces.value = await invoke<WorkspaceInfo[]>('list_workspaces')
  } catch (e) {
    console.error('Failed to load workspaces:', e)
  } finally {
    isLoading.value = false
  }
}

async function refreshWorkspaces() {
  await loadWorkspaces()
}

async function openFolderDialog() {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: 'Select Workspace Folder',
    })
    
    if (selected && typeof selected === 'string') {
      await openWorkspace(selected)
    }
  } catch (e) {
    console.error('Failed to open folder dialog:', e)
  }
}

async function openWorkspace(path: string) {
  isLoading.value = true
  try {
    const ws = await invoke<WorkspaceInfo>('open_workspace', { path })
    workspaces.value = await invoke<WorkspaceInfo[]>('list_workspaces')
    console.log('Opened workspace:', ws.name)
  } catch (e) {
    console.error('Failed to open workspace:', e)
  } finally {
    isLoading.value = false
  }
}

async function selectWorkspace(ws: WorkspaceInfo) {
  selectedWorkspace.value = ws
}

async function setActive(ws: WorkspaceInfo) {
  try {
    await invoke('set_active_workspace', { id: ws.id })
    await loadWorkspaces()
  } catch (e) {
    console.error('Failed to set active workspace:', e)
  }
}

async function setActiveAndClose(ws: WorkspaceInfo) {
  await setActive(ws)
  selectedWorkspace.value = null
}

async function closeWorkspace(ws: WorkspaceInfo) {
  try {
    await invoke('close_workspace', { id: ws.id })
    await loadWorkspaces()
  } catch (e) {
    console.error('Failed to close workspace:', e)
  }
}

async function reloadActiveWorkspace() {
  if (activeWorkspace.value) {
    await reloadWorkspaceById(activeWorkspace.value.id)
  }
}

async function reloadWorkspaceById(id: string) {
  try {
    await invoke('reload_workspace', { id })
    await loadWorkspaces()
  } catch (e) {
    console.error('Failed to reload workspace:', e)
  }
}

function viewWorkspaceDetails(ws: WorkspaceInfo) {
  selectedWorkspace.value = ws
}
</script>

<style scoped>
.expand-enter-active,
.expand-leave-active {
  transition: all 0.2s ease;
  overflow: hidden;
}
.expand-enter-from,
.expand-leave-to {
  opacity: 0;
  max-height: 0;
}
.expand-enter-to,
.expand-leave-from {
  opacity: 1;
  max-height: 500px;
}
</style>
