<template>
  <div class="h-full flex flex-col relative overflow-hidden">

    <!-- Header -->
    <header class="relative z-10 px-4 sm:px-6 py-3 sm:py-4 border-b border-white/[0.04]">
      <div class="flex items-center justify-between">
        <div>
          <h2 class="text-base sm:text-lg font-semibold text-white/90">Workspaces</h2>
          <p class="text-xs sm:text-sm text-white/30">
            {{ workspaces.length }} workspace{{ workspaces.length !== 1 ? 's' : '' }} registered
          </p>
        </div>
        <div class="flex items-center gap-2">
          <UiGlassButton pill size="xs" color="accent" @click="showCreateDialog = true">
            <FolderPlus class="w-3.5 h-3.5" />
            Create
          </UiGlassButton>
          <UiGlassButton pill size="xs" @click="openFolderDialog">
            <FolderOpen class="w-3.5 h-3.5" />
            Open
          </UiGlassButton>
          <UiGlassButton pill size="xs" :disabled="isLoading" @click="refreshWorkspaces">
            <RefreshCw :class="['w-3.5 h-3.5', isLoading && 'animate-spin']" />
          </UiGlassButton>
        </div>
      </div>
    </header>

    <!-- Content -->
    <div class="relative z-10 flex-1 overflow-y-auto p-4 sm:p-6">
      <div class="max-w-4xl mx-auto space-y-4">

        <!-- Active Workspace Banner -->
        <UiGroundGlass v-if="activeWorkspace" class="active-banner">
          <div class="relative z-10 p-4">
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-3">
                <div class="w-9 h-9 rounded-lg bg-white/[0.06] flex items-center justify-center">
                  <FolderCheck class="w-5 h-5 text-cyan-300/80" />
                </div>
                <div>
                  <div class="text-[10px] uppercase tracking-wider text-white/30">Active Workspace</div>
                  <div class="font-semibold text-cyan-300/90">{{ activeWorkspace.name }}</div>
                </div>
              </div>
              <div class="flex items-center gap-2">
                <UiGlassButton pill size="xs" @click="reloadActiveWorkspace">
                  <RefreshCw class="w-3.5 h-3.5" />
                </UiGlassButton>
                <UiGlassButton pill size="xs" @click="viewWorkspaceDetails(activeWorkspace)">
                  View Details
                </UiGlassButton>
              </div>
            </div>

            <!-- Context file indicators -->
            <div class="flex gap-1.5 mt-3 flex-wrap items-center">
              <span
                v-for="file in contextFiles"
                :key="file.key"
                :class="[
                  'text-[10px] px-2 py-0.5 rounded-full',
                  activeWorkspace[file.key]
                    ? 'bg-white/[0.08] text-white/60'
                    : 'bg-white/[0.03] text-white/20'
                ]"
              >
                {{ activeWorkspace[file.key] ? '✓' : '○' }} {{ file.name }}
              </span>

              <button
                v-if="!activeWorkspace.has_soul || !activeWorkspace.has_user"
                @click="repairWorkspace(activeWorkspace)"
                class="ml-auto text-[10px] text-amber-300/60 hover:text-amber-300/90 transition-colors flex items-center gap-1"
              >
                <Wrench class="w-3 h-3" />
                Add missing files
              </button>

              <span v-else class="text-[10px] text-white/20 ml-auto">
                {{ activeWorkspace.context_chars.toLocaleString() }} chars
              </span>
            </div>
          </div>
        </UiGroundGlass>

        <!-- Empty State -->
        <div v-if="workspaces.length === 0 && !isLoading" class="flex items-center justify-center min-h-[400px]">
          <div class="text-center max-w-md px-4">
            <div class="text-5xl sm:text-6xl mb-4">📂</div>
            <h3 class="text-lg sm:text-xl font-semibold text-white/80 mb-2">No workspaces open</h3>
            <p class="text-sm text-white/30 mb-6">
              Create a new workspace or open an existing folder containing workspace files.
            </p>
            <div class="flex gap-3 justify-center">
              <UiGlassButton pill size="sm" color="accent" @click="showCreateDialog = true">
                <FolderPlus class="w-4 h-4" />
                Create Workspace
              </UiGlassButton>
              <UiGlassButton pill size="sm" @click="openFolderDialog">
                <FolderOpen class="w-4 h-4" />
                Open Existing
              </UiGlassButton>
            </div>
          </div>
        </div>

        <!-- Workspace List -->
        <div v-else class="space-y-2">

          <!-- Global Option -->
          <UiGroundGlass
            :class="['ws-card', { 'ws-card--active': !activeWorkspace }]"
            @click="clearActiveWorkspace"
          >
            <div class="relative z-10 flex items-center gap-3 p-3 cursor-pointer">
              <div :class="[
                'w-9 h-9 rounded-lg flex items-center justify-center shrink-0',
                !activeWorkspace ? 'bg-purple-500/20' : 'bg-white/[0.04]'
              ]">
                <Globe :class="['w-5 h-5', !activeWorkspace ? 'text-purple-400/80' : 'text-white/30']" />
              </div>
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2">
                  <span class="font-medium text-white/90">Global</span>
                  <span v-if="!activeWorkspace" class="text-[10px] px-2 py-0.5 rounded-full bg-purple-500/20 text-purple-300/80">
                    Active
                  </span>
                </div>
                <div class="text-xs text-white/25">No workspace context · Uses global memory only</div>
              </div>
            </div>
          </UiGroundGlass>

          <!-- Workspace Cards -->
          <UiGroundGlass
            v-for="ws in workspaces"
            :key="ws.id"
            :class="['ws-card', { 'ws-card--active': ws.active }]"
            @click="selectWorkspace(ws)"
          >
            <div class="relative z-10 flex items-center gap-3 p-3 cursor-pointer">
              <!-- Icon -->
              <div :class="[
                'w-9 h-9 rounded-lg flex items-center justify-center shrink-0',
                ws.active ? 'bg-cyan-400/15' : 'bg-white/[0.04]'
              ]">
                <Folder :class="['w-5 h-5', ws.active ? 'text-cyan-300/80' : 'text-white/30']" />
              </div>

              <!-- Info -->
              <div class="flex-1 min-w-0">
                <div class="flex items-center gap-2">
                  <span class="font-medium text-white/90 truncate">{{ ws.name }}</span>
                  <span v-if="ws.active" class="text-[10px] px-2 py-0.5 rounded-full bg-cyan-400/15 text-cyan-300/80">
                    Active
                  </span>
                  <span v-if="!ws.has_soul && !ws.has_agents" class="text-[10px] px-2 py-0.5 rounded-full bg-amber-400/15 text-amber-300/70">
                    Incomplete
                  </span>
                </div>
                <div class="text-xs text-white/25 truncate">{{ ws.path }}</div>
              </div>

              <!-- Context file dots -->
              <div class="hidden sm:flex gap-1 shrink-0">
                <span
                  v-for="file in contextFiles"
                  :key="file.key"
                  :class="[
                    'text-[10px] px-1.5 py-0.5 rounded',
                    ws[file.key] ? 'bg-emerald-400/15 text-emerald-300/60' : 'bg-white/[0.03] text-white/15'
                  ]"
                  :title="file.name"
                >
                  {{ file.short }}
                </span>
              </div>

              <!-- Actions -->
              <div class="flex gap-1 shrink-0" @click.stop>
                <button
                  v-if="!ws.active"
                  @click="setActive(ws)"
                  class="p-1.5 rounded-md text-white/20 hover:text-white/60 hover:bg-white/[0.04] transition-all"
                  title="Set as active"
                >
                  <Play class="w-4 h-4" />
                </button>
                <button
                  @click="closeWorkspace(ws)"
                  class="p-1.5 rounded-md text-white/20 hover:text-red-400/60 hover:bg-white/[0.04] transition-all"
                  title="Close workspace"
                >
                  <X class="w-4 h-4" />
                </button>
              </div>
            </div>
          </UiGroundGlass>
        </div>

        <!-- Workspace Files Reference -->
        <UiGroundGlass class="mt-6">
          <div class="relative z-10 p-4">
            <h3 class="text-xs font-semibold text-purple-300/70 mb-3 flex items-center gap-2 uppercase tracking-wider">
              <FileText class="w-3.5 h-3.5" />
              Workspace Files Reference
            </h3>
            <div class="grid gap-1.5 text-sm">
              <div
                v-for="ref in fileReference"
                :key="ref.name"
                class="flex items-start gap-3 px-3 py-2 rounded-lg bg-white/[0.03]"
              >
                <code class="text-cyan-300/70 text-xs shrink-0 mt-0.5">{{ ref.name }}</code>
                <span class="text-white/35 text-xs">{{ ref.desc }}</span>
              </div>
            </div>
          </div>
        </UiGroundGlass>

      </div>
    </div>

    <!-- ═══ Create Workspace Dialog ═══ -->
    <Teleport to="body">
      <Transition name="dialog">
        <div v-if="showCreateDialog" class="fixed inset-0 z-50 flex items-center justify-center p-4" @click.self="showCreateDialog = false">
          <div class="absolute inset-0 bg-black/60 backdrop-blur-sm" />
          <UiGroundGlass class="dialog-panel relative w-full max-w-lg">
            <div class="relative z-10">
              <!-- Header -->
              <div class="flex items-center justify-between p-4 border-b border-white/[0.04]">
                <div class="flex items-center gap-3">
                  <FolderPlus class="w-5 h-5 text-cyan-300/70" />
                  <h3 class="font-semibold text-white/90">Create Workspace</h3>
                </div>
                <button @click="showCreateDialog = false" class="text-white/20 hover:text-white/60 transition-colors">
                  <X class="w-4 h-4" />
                </button>
              </div>

              <!-- Content -->
              <div class="p-4 space-y-4">
                <!-- Folder Selection -->
                <div>
                  <label class="text-xs font-medium text-white/50 mb-2 block">Location</label>
                  <div class="flex gap-2">
                    <UiGlassInput
                      v-model="createPath"
                      placeholder="Select a folder..."
                      mono
                      disabled
                      class="flex-1"
                    />
                    <UiGlassButton pill size="xs" @click="selectCreateFolder">
                      Browse
                    </UiGlassButton>
                  </div>
                  <p v-if="createValidity && createValidity.exists && createValidity.is_valid" class="text-[11px] text-amber-300/60 mt-1.5">
                    ⚠️ This folder already has workspace files. Missing files will be added.
                  </p>
                </div>

                <!-- File Selection -->
                <div>
                  <label class="text-xs font-medium text-white/50 mb-2 block">Files to create</label>
                  <div class="space-y-1">
                    <label
                      v-for="file in availableFiles"
                      :key="file.name"
                      class="flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-white/[0.04] cursor-pointer transition-colors"
                    >
                      <input
                        type="checkbox"
                        v-model="createFiles"
                        :value="file.name"
                        :disabled="createValidity && createValidity[file.existsKey]"
                        class="rounded border-white/20 bg-transparent"
                      />
                      <div class="flex-1">
                        <code class="text-cyan-300/70 text-xs">{{ file.name }}</code>
                        <span v-if="createValidity && createValidity[file.existsKey]" class="text-[10px] text-emerald-300/60 ml-2">
                          ✓ exists
                        </span>
                      </div>
                      <span class="text-[11px] text-white/25">{{ file.desc }}</span>
                    </label>
                  </div>
                </div>
              </div>

              <!-- Footer -->
              <div class="flex justify-end gap-2 p-4 border-t border-white/[0.04]">
                <UiGlassButton pill size="xs" @click="showCreateDialog = false">
                  Cancel
                </UiGlassButton>
                <UiGlassButton
                  pill
                  size="xs"
                  color="accent"
                  :disabled="!createPath || createFiles.length === 0"
                  @click="createWorkspace"
                >
                  <FolderPlus class="w-3.5 h-3.5" />
                  Create
                </UiGlassButton>
              </div>
            </div>
          </UiGroundGlass>
        </div>
      </Transition>
    </Teleport>

    <!-- ═══ Workspace Details Dialog ═══ -->
    <Teleport to="body">
      <Transition name="dialog">
        <div v-if="selectedWorkspace" class="fixed inset-0 z-50 flex items-center justify-center p-4" @click.self="selectedWorkspace = null">
          <div class="absolute inset-0 bg-black/60 backdrop-blur-sm" />
          <UiGroundGlass class="dialog-panel relative w-full max-w-2xl max-h-[80vh] overflow-hidden">
            <div class="relative z-10">
              <!-- Header -->
              <div class="flex items-center justify-between p-4 border-b border-white/[0.04]">
                <div class="flex items-center gap-3">
                  <FolderCheck class="w-5 h-5 text-cyan-300/70" />
                  <h3 class="font-semibold text-white/90">{{ selectedWorkspace.name }}</h3>
                </div>
                <button @click="selectedWorkspace = null" class="text-white/20 hover:text-white/60 transition-colors">
                  <X class="w-4 h-4" />
                </button>
              </div>

              <!-- Content -->
              <div class="p-4 overflow-y-auto max-h-[60vh] space-y-4">
                <!-- Path -->
                <div class="px-3 py-2.5 rounded-lg bg-white/[0.03]">
                  <div class="text-[10px] text-white/25 mb-0.5">Path</div>
                  <code class="text-xs text-white/60 break-all">{{ selectedWorkspace.path }}</code>
                </div>

                <!-- Context Files Status -->
                <div>
                  <div class="text-xs font-medium text-white/50 mb-2">Context Files</div>
                  <div class="grid grid-cols-2 gap-1.5">
                    <div
                      v-for="file in detailFiles"
                      :key="file.key"
                      :class="[
                        'flex items-center justify-between px-3 py-2 rounded-lg',
                        selectedWorkspace[file.key] ? 'bg-emerald-400/[0.06]' : 'bg-white/[0.03]'
                      ]"
                    >
                      <span :class="selectedWorkspace[file.key] ? 'text-emerald-300/60 text-xs' : 'text-white/20 text-xs'">
                        {{ selectedWorkspace[file.key] ? '✓' : '○' }} {{ file.name }}
                      </span>
                      <UiGlassButton
                        v-if="!selectedWorkspace[file.key]"
                        pill
                        size="xs"
                        @click="createSingleFile(selectedWorkspace, file.name)"
                        class="!py-0.5 !px-2 !text-[10px]"
                      >
                        Create
                      </UiGlassButton>
                    </div>
                  </div>
                </div>

                <!-- Stats -->
                <div class="flex gap-6 text-xs text-white/30">
                  <div>
                    <span class="text-white/20">Total context:</span>
                    <span class="text-white/50 ml-1 font-mono">{{ selectedWorkspace.context_chars.toLocaleString() }} chars</span>
                  </div>
                  <div>
                    <span class="text-white/20">~Tokens:</span>
                    <span class="text-white/50 ml-1 font-mono">{{ Math.round(selectedWorkspace.context_chars / 4).toLocaleString() }}</span>
                  </div>
                </div>
              </div>

              <!-- Footer -->
              <div class="flex justify-end gap-2 p-4 border-t border-white/[0.04]">
                <UiGlassButton pill size="xs" @click="reloadWorkspaceById(selectedWorkspace.id)">
                  <RefreshCw class="w-3.5 h-3.5" />
                  Reload
                </UiGlassButton>
                <UiGlassButton
                  v-if="!selectedWorkspace.active"
                  pill
                  size="xs"
                  color="accent"
                  @click="setActiveAndClose(selectedWorkspace)"
                >
                  <Play class="w-3.5 h-3.5" />
                  Set Active
                </UiGlassButton>
              </div>
            </div>
          </UiGroundGlass>
        </div>
      </Transition>
    </Teleport>

    <!-- ═══ Repair Dialog ═══ -->
    <Teleport to="body">
      <Transition name="dialog">
        <div v-if="showRepairDialog" class="fixed inset-0 z-50 flex items-center justify-center p-4" @click.self="showRepairDialog = false">
          <div class="absolute inset-0 bg-black/60 backdrop-blur-sm" />
          <UiGroundGlass class="dialog-panel relative w-full max-w-lg">
            <div class="relative z-10">
              <!-- Header -->
              <div class="flex items-center justify-between p-4 border-b border-white/[0.04]">
                <div class="flex items-center gap-3">
                  <Wrench class="w-5 h-5 text-amber-300/70" />
                  <h3 class="font-semibold text-white/90">Add Missing Files</h3>
                </div>
                <button @click="showRepairDialog = false" class="text-white/20 hover:text-white/60 transition-colors">
                  <X class="w-4 h-4" />
                </button>
              </div>

              <!-- Content -->
              <div class="p-4 space-y-4">
                <p class="text-xs text-white/40">
                  The workspace <strong class="text-white/70">{{ repairTarget?.name }}</strong> is missing some files.
                  Select which files to create with default templates:
                </p>

                <div class="space-y-1">
                  <label
                    v-for="file in missingFilesForRepair"
                    :key="file.name"
                    class="flex items-center gap-3 px-3 py-2 rounded-lg hover:bg-white/[0.04] cursor-pointer transition-colors"
                  >
                    <input
                      type="checkbox"
                      v-model="repairFiles"
                      :value="file.name"
                      class="rounded border-white/20 bg-transparent"
                    />
                    <code class="text-cyan-300/70 text-xs">{{ file.name }}</code>
                    <span class="text-[11px] text-white/25">{{ file.desc }}</span>
                  </label>
                </div>
              </div>

              <!-- Footer -->
              <div class="flex justify-end gap-2 p-4 border-t border-white/[0.04]">
                <UiGlassButton pill size="xs" @click="showRepairDialog = false">
                  Cancel
                </UiGlassButton>
                <UiGlassButton
                  pill
                  size="xs"
                  color="accent"
                  :disabled="repairFiles.length === 0"
                  @click="executeRepair"
                >
                  <Wrench class="w-3.5 h-3.5" />
                  Create Files
                </UiGlassButton>
              </div>
            </div>
          </UiGroundGlass>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, watch, inject } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import {
  Folder, FolderOpen, FolderPlus, FolderCheck, RefreshCw, X, Play, FileText, Wrench, Globe
} from 'lucide-vue-next'

// Inject tab management functions from layout
const addWorkspaceTab = inject<(ws: WorkspaceInfo) => void>('addWorkspaceTab')
const selectWorkspaceTab = inject<(id: string) => void>('selectWorkspaceTab')
const selectGlobalTab = inject<() => void>('selectGlobalTab')

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

const fileReference = [
  { name: 'SOUL.md', desc: 'Agent personality, identity, voice' },
  { name: 'USER.md', desc: 'Info about the user (name, preferences)' },
  { name: 'AGENTS.md', desc: 'How the agent should behave in this workspace' },
  { name: 'TOOLS.md', desc: 'Tool-specific notes and configurations' },
  { name: 'MEMORY.md', desc: 'Long-term curated memories' },
  { name: 'memory/', desc: 'Daily notes (YYYY-MM-DD.md files)' },
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
      const validity = await invoke<WorkspaceValidity>('check_workspace_validity', { path: selected })

      if (!validity.is_valid) {
        createPath.value = selected
        createValidity.value = validity
        showCreateDialog.value = true
      } else {
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
    addWorkspaceTab?.(ws)
    selectWorkspaceTab?.(ws.id)
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
    await invoke('init_workspace', {
      path: createPath.value,
      files: createFiles.value
    })
    await openWorkspace(createPath.value)
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
    addWorkspaceTab?.(ws)
    selectWorkspaceTab?.(ws.id)
  } catch (e) {
    console.error('Failed to set active workspace:', e)
  }
}

async function clearActiveWorkspace() {
  try {
    await invoke('clear_active_workspace')
    await loadWorkspaces()
    selectGlobalTab?.()
  } catch (e) {
    console.error('Failed to clear active workspace:', e)
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
    await reloadWorkspaceById(repairTarget.value.id)
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
/* ═══ Workspace cards ═══ */
.ws-card {
  transition: border-color 0.15s ease;
}
.ws-card:hover {
  border-color: rgba(139, 92, 246, 0.25);
}
.ws-card--active {
  border-color: rgba(139, 92, 246, 0.3);
}

/* ═══ Active workspace banner ═══ */
.active-banner {
  border-color: rgba(34, 211, 238, 0.15);
}

/* ═══ Dialog panels ═══ */
.dialog-panel {
  border-radius: 1rem;
}

/* ═══ Dialog transitions ═══ */
.dialog-enter-active {
  transition: all 0.15s ease-out;
}
.dialog-leave-active {
  transition: all 0.1s ease-in;
}
.dialog-enter-from {
  opacity: 0;
  transform: scale(0.97) translateY(4px);
}
.dialog-leave-to {
  opacity: 0;
  transform: scale(0.97);
}
</style>
