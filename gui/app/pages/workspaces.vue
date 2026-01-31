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
          <UiButton @click="showCreateDialog = true" variant="default" size="sm">
            <FolderPlus class="w-4 h-4 mr-1" />
            Create
          </UiButton>
          <UiButton @click="openFolderDialog" variant="secondary" size="sm">
            <FolderOpen class="w-4 h-4 mr-1" />
            Open
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
            <UiBadge v-else variant="secondary" class="opacity-50">○ SOUL.md</UiBadge>
            <UiBadge v-if="activeWorkspace.has_user" variant="secondary">USER.md</UiBadge>
            <UiBadge v-else variant="secondary" class="opacity-50">○ USER.md</UiBadge>
            <UiBadge v-if="activeWorkspace.has_agents" variant="secondary">AGENTS.md</UiBadge>
            <UiBadge v-if="activeWorkspace.has_memory" variant="secondary">MEMORY.md</UiBadge>
            
            <!-- Repair button if missing files -->
            <UiButton 
              v-if="!activeWorkspace.has_soul || !activeWorkspace.has_user"
              @click="repairWorkspace(activeWorkspace)"
              variant="ghost"
              size="sm"
              class="ml-auto text-nanna-warning"
            >
              <Wrench class="w-3 h-3 mr-1" />
              Add missing files
            </UiButton>
            
            <span v-else class="text-xs text-nanna-text-dim ml-auto">
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
            Create a new workspace or open an existing folder containing workspace files.
          </p>
          <div class="flex gap-3 justify-center">
            <UiButton @click="showCreateDialog = true">
              <FolderPlus class="w-4 h-4 mr-2" />
              Create Workspace
            </UiButton>
            <UiButton @click="openFolderDialog" variant="secondary">
              <FolderOpen class="w-4 h-4 mr-2" />
              Open Existing
            </UiButton>
          </div>
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
                  <UiBadge v-if="!ws.has_soul && !ws.has_agents" variant="warning" class="shrink-0">
                    Incomplete
                  </UiBadge>
                </div>
                <div class="text-xs text-nanna-text-dim truncate">{{ ws.path }}</div>
              </div>
              
              <!-- Context files -->
              <div class="hidden sm:flex gap-1 shrink-0">
                <span 
                  v-for="file in contextFiles" 
                  :key="file.key"
                  :class="[
                    'text-xs px-1.5 py-0.5 rounded',
                    ws[file.key] ? 'bg-nanna-success/20 text-nanna-success' : 'bg-nanna-bg-elevated/30 text-nanna-text-dim'
                  ]"
                  :title="file.name"
                >
                  {{ file.short }}
                </span>
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

    <!-- Create Workspace Dialog -->
    <Teleport to="body">
      <div v-if="showCreateDialog" class="fixed inset-0 bg-black/60 flex items-center justify-center z-50 p-4" @click.self="showCreateDialog = false">
        <div class="bg-nanna-bg-surface rounded-xl w-full max-w-lg overflow-hidden border border-nanna-primary/20 shadow-2xl">
          <!-- Dialog Header -->
          <div class="flex items-center justify-between p-4 border-b border-nanna-primary/10">
            <div class="flex items-center gap-3">
              <FolderPlus class="w-5 h-5 text-nanna-accent" />
              <h3 class="font-semibold text-nanna-text">Create Workspace</h3>
            </div>
            <UiButton @click="showCreateDialog = false" variant="ghost" size="sm">
              <X class="w-4 h-4" />
            </UiButton>
          </div>
          
          <!-- Dialog Content -->
          <div class="p-4 space-y-4">
            <!-- Folder Selection -->
            <div>
              <label class="text-sm font-medium text-nanna-text mb-2 block">Location</label>
              <div class="flex gap-2">
                <input 
                  v-model="createPath"
                  type="text"
                  class="flex-1 px-3 py-2 rounded-lg bg-nanna-bg-elevated border border-nanna-primary/20 text-nanna-text text-sm focus:outline-none focus:border-nanna-accent"
                  placeholder="Select a folder..."
                  readonly
                />
                <UiButton @click="selectCreateFolder" variant="secondary" size="sm">
                  Browse
                </UiButton>
              </div>
              <p v-if="createValidity && createValidity.exists && createValidity.is_valid" class="text-xs text-nanna-warning mt-1">
                ⚠️ This folder already has workspace files. Missing files will be added.
              </p>
            </div>
            
            <!-- File Selection -->
            <div>
              <label class="text-sm font-medium text-nanna-text mb-2 block">Files to create</label>
              <div class="space-y-2">
                <label 
                  v-for="file in availableFiles" 
                  :key="file.name"
                  class="flex items-center gap-3 p-2 rounded-lg hover:bg-nanna-bg-elevated/50 cursor-pointer"
                >
                  <input 
                    type="checkbox" 
                    v-model="createFiles"
                    :value="file.name"
                    :disabled="createValidity && createValidity[file.existsKey]"
                    class="rounded border-nanna-primary/30"
                  />
                  <div class="flex-1">
                    <code class="text-nanna-accent text-sm">{{ file.name }}</code>
                    <span v-if="createValidity && createValidity[file.existsKey]" class="text-xs text-nanna-success ml-2">
                      ✓ exists
                    </span>
                  </div>
                  <span class="text-xs text-nanna-text-dim">{{ file.desc }}</span>
                </label>
              </div>
            </div>
          </div>
          
          <!-- Dialog Footer -->
          <div class="flex justify-end gap-2 p-4 border-t border-nanna-primary/10">
            <UiButton @click="showCreateDialog = false" variant="ghost" size="sm">
              Cancel
            </UiButton>
            <UiButton 
              @click="createWorkspace" 
              :disabled="!createPath || createFiles.length === 0"
              size="sm"
            >
              <FolderPlus class="w-4 h-4 mr-1" />
              Create
            </UiButton>
          </div>
        </div>
      </div>
    </Teleport>

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
                <div 
                  v-for="file in detailFiles"
                  :key="file.key"
                  :class="['p-2 rounded flex items-center justify-between', selectedWorkspace[file.key] ? 'bg-nanna-success/10' : 'bg-nanna-bg-elevated/30']"
                >
                  <span :class="selectedWorkspace[file.key] ? 'text-nanna-success' : 'text-nanna-text-dim'">
                    {{ selectedWorkspace[file.key] ? '✓' : '○' }} {{ file.name }}
                  </span>
                  <UiButton 
                    v-if="!selectedWorkspace[file.key]"
                    @click="createSingleFile(selectedWorkspace, file.name)"
                    variant="ghost"
                    size="sm"
                    class="h-6 px-2 text-xs"
                  >
                    Create
                  </UiButton>
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

    <!-- Repair Dialog -->
    <Teleport to="body">
      <div v-if="showRepairDialog" class="fixed inset-0 bg-black/60 flex items-center justify-center z-50 p-4" @click.self="showRepairDialog = false">
        <div class="bg-nanna-bg-surface rounded-xl w-full max-w-lg overflow-hidden border border-nanna-primary/20 shadow-2xl">
          <div class="flex items-center justify-between p-4 border-b border-nanna-primary/10">
            <div class="flex items-center gap-3">
              <Wrench class="w-5 h-5 text-nanna-warning" />
              <h3 class="font-semibold text-nanna-text">Add Missing Files</h3>
            </div>
            <UiButton @click="showRepairDialog = false" variant="ghost" size="sm">
              <X class="w-4 h-4" />
            </UiButton>
          </div>
          
          <div class="p-4 space-y-4">
            <p class="text-sm text-nanna-text-muted">
              The workspace <strong class="text-nanna-text">{{ repairTarget?.name }}</strong> is missing some files. 
              Select which files to create with default templates:
            </p>
            
            <div class="space-y-2">
              <label 
                v-for="file in missingFilesForRepair" 
                :key="file.name"
                class="flex items-center gap-3 p-2 rounded-lg hover:bg-nanna-bg-elevated/50 cursor-pointer"
              >
                <input 
                  type="checkbox" 
                  v-model="repairFiles"
                  :value="file.name"
                  class="rounded border-nanna-primary/30"
                />
                <code class="text-nanna-accent text-sm">{{ file.name }}</code>
                <span class="text-xs text-nanna-text-dim">{{ file.desc }}</span>
              </label>
            </div>
          </div>
          
          <div class="flex justify-end gap-2 p-4 border-t border-nanna-primary/10">
            <UiButton @click="showRepairDialog = false" variant="ghost" size="sm">
              Cancel
            </UiButton>
            <UiButton 
              @click="executeRepair" 
              :disabled="repairFiles.length === 0"
              size="sm"
            >
              <Wrench class="w-4 h-4 mr-1" />
              Create Files
            </UiButton>
          </div>
        </div>
      </div>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { 
  Folder, FolderOpen, FolderPlus, FolderCheck, RefreshCw, X, Play, FileText, Wrench
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

interface WorkspaceValidity {
  exists: boolean
  is_valid: boolean
  has_soul: boolean
  has_user: boolean
  has_agents: boolean
  has_tools: boolean
  has_memory: boolean
  has_memory_folder: boolean
}

const workspaces = ref<WorkspaceInfo[]>([])
const isLoading = ref(false)
const selectedWorkspace = ref<WorkspaceInfo | null>(null)

// Create dialog state
const showCreateDialog = ref(false)
const createPath = ref('')
const createFiles = ref<string[]>(['SOUL.md', 'USER.md', 'AGENTS.md'])
const createValidity = ref<WorkspaceValidity | null>(null)

// Repair dialog state
const showRepairDialog = ref(false)
const repairTarget = ref<WorkspaceInfo | null>(null)
const repairFiles = ref<string[]>([])

const activeWorkspace = computed(() => workspaces.value.find(ws => ws.active))

const contextFiles = [
  { key: 'has_soul', name: 'SOUL.md', short: 'S' },
  { key: 'has_user', name: 'USER.md', short: 'U' },
  { key: 'has_agents', name: 'AGENTS.md', short: 'A' },
  { key: 'has_memory', name: 'MEMORY.md', short: 'M' },
]

const detailFiles = [
  { key: 'has_soul', name: 'SOUL.md' },
  { key: 'has_user', name: 'USER.md' },
  { key: 'has_agents', name: 'AGENTS.md' },
  { key: 'has_memory', name: 'MEMORY.md' },
]

const availableFiles = [
  { name: 'SOUL.md', desc: 'Agent personality', existsKey: 'has_soul' },
  { name: 'USER.md', desc: 'User info', existsKey: 'has_user' },
  { name: 'AGENTS.md', desc: 'Behavior rules', existsKey: 'has_agents' },
  { name: 'TOOLS.md', desc: 'Tool notes', existsKey: 'has_tools' },
  { name: 'MEMORY.md', desc: 'Long-term memory', existsKey: 'has_memory' },
]

const missingFilesForRepair = computed(() => {
  if (!repairTarget.value) return []
  return availableFiles.filter(f => {
    const key = f.existsKey as keyof WorkspaceInfo
    return !repairTarget.value![key]
  })
})

onMounted(async () => {
  await loadWorkspaces()
})

// Check validity when create path changes
watch(createPath, async (path) => {
  if (path) {
    try {
      createValidity.value = await invoke<WorkspaceValidity>('check_workspace_validity', { path })
      // Uncheck files that already exist
      createFiles.value = createFiles.value.filter(f => {
        const file = availableFiles.find(af => af.name === f)
        if (!file) return true
        return !createValidity.value![file.existsKey as keyof WorkspaceValidity]
      })
    } catch (e) {
      createValidity.value = null
    }
  } else {
    createValidity.value = null
  }
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
      // Check if it's a valid workspace
      const validity = await invoke<WorkspaceValidity>('check_workspace_validity', { path: selected })
      
      if (!validity.is_valid) {
        // Not a workspace - offer to create one
        createPath.value = selected
        createValidity.value = validity
        showCreateDialog.value = true
      } else {
        // Valid workspace - open it
        await openWorkspace(selected)
      }
    }
  } catch (e) {
    console.error('Failed to open folder dialog:', e)
  }
}

async function selectCreateFolder() {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: 'Select Folder for Workspace',
    })
    
    if (selected && typeof selected === 'string') {
      createPath.value = selected
    }
  } catch (e) {
    console.error('Failed to select folder:', e)
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

async function createWorkspace() {
  if (!createPath.value || createFiles.value.length === 0) return
  
  isLoading.value = true
  try {
    // Initialize workspace with selected files
    await invoke('init_workspace', { 
      path: createPath.value, 
      files: createFiles.value 
    })
    
    // Open the workspace
    await openWorkspace(createPath.value)
    
    // Reset and close dialog
    showCreateDialog.value = false
    createPath.value = ''
    createFiles.value = ['SOUL.md', 'USER.md', 'AGENTS.md']
    createValidity.value = null
  } catch (e) {
    console.error('Failed to create workspace:', e)
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
    // Update selected workspace if it was being viewed
    if (selectedWorkspace.value?.id === id) {
      selectedWorkspace.value = workspaces.value.find(w => w.id === id) || null
    }
  } catch (e) {
    console.error('Failed to reload workspace:', e)
  }
}

function viewWorkspaceDetails(ws: WorkspaceInfo) {
  selectedWorkspace.value = ws
}

function repairWorkspace(ws: WorkspaceInfo) {
  repairTarget.value = ws
  // Pre-select all missing files
  repairFiles.value = missingFilesForRepair.value.map(f => f.name)
  showRepairDialog.value = true
}

async function executeRepair() {
  if (!repairTarget.value || repairFiles.value.length === 0) return
  
  isLoading.value = true
  try {
    await invoke('init_workspace', {
      path: repairTarget.value.path,
      files: repairFiles.value,
    })
    
    // Reload the workspace
    await reloadWorkspaceById(repairTarget.value.id)
    
    // Close dialog
    showRepairDialog.value = false
    repairTarget.value = null
    repairFiles.value = []
  } catch (e) {
    console.error('Failed to repair workspace:', e)
  } finally {
    isLoading.value = false
  }
}

async function createSingleFile(ws: WorkspaceInfo, filename: string) {
  isLoading.value = true
  try {
    await invoke('init_workspace', {
      path: ws.path,
      files: [filename],
    })
    
    await reloadWorkspaceById(ws.id)
    
    // Update the selected workspace view
    if (selectedWorkspace.value?.id === ws.id) {
      selectedWorkspace.value = workspaces.value.find(w => w.id === ws.id) || null
    }
  } catch (e) {
    console.error('Failed to create file:', e)
  } finally {
    isLoading.value = false
  }
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
