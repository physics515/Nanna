<template>
  <div class="min-h-screen bg-nanna-bg-deep bg-grid relative">
    <!-- Mobile Header -->
    <header class="lg:hidden fixed top-0 left-0 right-0 z-40 flex items-center justify-between px-4 py-3 bg-nanna-bg-surface/95 backdrop-blur border-b border-nanna-primary/10">
      <button 
        @click="sidebarOpen = true"
        class="p-2 rounded-lg text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
        aria-label="Open menu"
      >
        <Menu class="w-5 h-5" />
      </button>
      
      <NuxtLink to="/" class="flex items-center gap-2">
        <span class="text-lg font-bold text-nanna-accent crt-glow">NANNA</span>
      </NuxtLink>
      
      <button 
        @click="createNewSession"
        class="p-2 rounded-lg text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
        aria-label="New chat"
      >
        <Plus class="w-5 h-5" />
      </button>
    </header>
    
    <!-- Mobile Sidebar (Sheet) -->
    <UiSheet v-model:open="sidebarOpen" side="left">
      <template #trigger>
        <span></span>
      </template>
      
      <div class="flex flex-col h-full -m-6">
        <!-- Logo -->
        <div class="p-4 border-b border-nanna-primary/10">
          <NuxtLink to="/" @click="sidebarOpen = false" class="block">
            <h1 class="text-2xl font-bold text-nanna-accent crt-glow">NANNA</h1>
            <p class="text-xs text-nanna-text-muted mt-1">AI Assistant</p>
          </NuxtLink>
        </div>
        
        <!-- New Chat button -->
        <div class="p-4">
          <UiButton @click="createNewSession(); sidebarOpen = false" class="w-full justify-start">
            <Plus class="w-4 h-4" />
            <span>New Chat</span>
          </UiButton>
        </div>
        
        <!-- Current Tab indicator (mobile) -->
        <div :class="[
          'px-4 py-2 border-b',
          currentTab?.type === 'workspace' 
            ? 'bg-nanna-accent/10 border-nanna-accent/20' 
            : 'bg-nanna-bg-elevated/50 border-nanna-primary/10'
        ]">
          <div class="flex items-center gap-2 text-xs">
            <component :is="currentTab?.type === 'workspace' ? FolderKanban : Globe" 
              :class="['w-3 h-3', currentTab?.type === 'workspace' ? 'text-nanna-accent' : 'text-nanna-text-dim']" 
            />
            <span :class="currentTab?.type === 'workspace' ? 'text-nanna-accent font-medium' : 'text-nanna-text-dim'">
              {{ currentTabName }}
            </span>
          </div>
        </div>
        
        <!-- Sessions list -->
        <nav class="flex-1 px-4 space-y-1 overflow-y-auto pt-2">
          <div class="text-xs text-nanna-text-dim uppercase tracking-wider mb-2">
            {{ currentTab?.type === 'workspace' ? 'Workspace Chats' : 'Global Chats' }}
          </div>
          
          <SessionItem
            v-for="session in sessions" 
            :key="session.id"
            :session="session"
            :is-active="currentSessionId === session.id"
            @select="(s) => { switchSession(s); sidebarOpen = false }"
            @deleted="onSessionDeleted"
            @renamed="onSessionRenamed"
          />
          
          <div v-if="sessions.length === 0" class="text-sm text-nanna-text-dim py-4 text-center">
            {{ currentTab?.type === 'workspace' ? 'No workspace chats yet' : 'No global chats yet' }}
          </div>
        </nav>
        
        <!-- Footer -->
        <div class="p-4 border-t border-nanna-primary/10 space-y-1">
          <NuxtLink to="/memory" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Brain class="w-4 h-4" /><span>Memory</span>
          </NuxtLink>
          <NuxtLink to="/logs" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <FileText class="w-4 h-4" /><span>Logs</span>
          </NuxtLink>
          <NuxtLink to="/workspaces" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <FolderKanban class="w-4 h-4" /><span>Workspaces</span>
          </NuxtLink>
          <NuxtLink to="/agents" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Bot class="w-4 h-4" /><span>Agents</span>
          </NuxtLink>
          <NuxtLink to="/channels" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Radio class="w-4 h-4" /><span>Channels</span>
          </NuxtLink>
          <NuxtLink to="/tools" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Wrench class="w-4 h-4" /><span>Tools</span>
          </NuxtLink>
          <NuxtLink to="/scheduler" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Clock class="w-4 h-4" /><span>Scheduler</span>
          </NuxtLink>
          <NuxtLink to="/settings" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Settings class="w-4 h-4" /><span>Settings</span>
          </NuxtLink>
          <button @click="hideToTray(); sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <ChevronDown class="w-4 h-4" /><span>Hide to Tray</span>
          </button>
          <div class="flex items-center justify-between text-xs text-nanna-text-dim px-3 pt-2">
            <div class="flex items-center gap-2">
              <span>v0.1.0</span>
              <span v-if="backendStatus" class="px-1.5 py-0.5 rounded text-[10px]" 
                    :class="isDaemon ? 'bg-nanna-accent/20 text-nanna-accent' : 'bg-nanna-bg-elevated text-nanna-text-dim'">
                {{ isDaemon ? 'daemon' : 'embedded' }}
              </span>
            </div>
            <span :class="backendStatus?.connected ? 'text-nanna-success' : (apiKeySet ? 'text-nanna-warning' : 'text-nanna-error')">
              {{ backendStatus?.connected ? '● Connected' : (apiKeySet ? '○ Disconnected' : '○ No API Key') }}
            </span>
          </div>
        </div>
      </div>
    </UiSheet>
    
    <!-- Main content -->
    <div class="flex h-screen">
      <!-- Desktop Sidebar -->
      <aside class="hidden lg:flex w-64 bg-nanna-bg-surface border-r border-nanna-primary/10 flex-col">
        <!-- Logo -->
        <div class="p-4 border-b border-nanna-primary/10">
          <NuxtLink to="/" class="block">
            <h1 class="text-2xl font-bold text-nanna-accent crt-glow">NANNA</h1>
            <p class="text-xs text-nanna-text-muted mt-1">AI Assistant</p>
          </NuxtLink>
        </div>
        
        <!-- New Chat button -->
        <div class="p-4">
          <UiButton @click="createNewSession" class="w-full justify-start">
            <Plus class="w-4 h-4" />
            <span>New Chat</span>
          </UiButton>
        </div>
        
        <!-- Current Tab indicator -->
        <div :class="[
          'px-4 py-2 border-b',
          currentTab?.type === 'workspace' 
            ? 'bg-nanna-accent/10 border-nanna-accent/20' 
            : 'bg-nanna-bg-elevated/50 border-nanna-primary/10'
        ]">
          <div class="flex items-center gap-2 text-xs">
            <component :is="currentTab?.type === 'workspace' ? FolderKanban : Globe" 
              :class="['w-3 h-3', currentTab?.type === 'workspace' ? 'text-nanna-accent' : 'text-nanna-text-dim']" 
            />
            <span :class="currentTab?.type === 'workspace' ? 'text-nanna-accent font-medium truncate' : 'text-nanna-text-dim'">
              {{ currentTabName }}
            </span>
          </div>
        </div>
        
        <!-- Sessions list -->
        <nav class="flex-1 px-4 space-y-1 overflow-y-auto pt-2">
          <div class="text-xs text-nanna-text-dim uppercase tracking-wider mb-2">
            {{ currentTab?.type === 'workspace' ? 'Workspace Chats' : 'Global Chats' }}
          </div>
          
          <SessionItem
            v-for="session in sessions" 
            :key="session.id"
            :session="session"
            :is-active="currentSessionId === session.id"
            @select="switchSession"
            @deleted="onSessionDeleted"
            @renamed="onSessionRenamed"
          />
          
          <div v-if="sessions.length === 0" class="text-sm text-nanna-text-dim py-4 text-center">
            {{ currentTab?.type === 'workspace' ? 'No workspace chats yet' : 'No global chats yet' }}
          </div>
        </nav>
        
        <!-- Footer -->
        <div class="p-4 border-t border-nanna-primary/10 space-y-1">
          <NuxtLink to="/memory" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Brain class="w-4 h-4" /><span>Memory</span>
          </NuxtLink>
          <NuxtLink to="/logs" @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <FileText class="w-4 h-4" /><span>Logs</span>
          </NuxtLink>
          <NuxtLink to="/workspaces" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <FolderKanban class="w-4 h-4" /><span>Workspaces</span>
          </NuxtLink>
          <NuxtLink to="/agents" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Bot class="w-4 h-4" /><span>Agents</span>
          </NuxtLink>
          <NuxtLink to="/channels" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Radio class="w-4 h-4" /><span>Channels</span>
          </NuxtLink>
          <NuxtLink to="/tools" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Wrench class="w-4 h-4" /><span>Tools</span>
          </NuxtLink>
          <NuxtLink to="/scheduler" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Clock class="w-4 h-4" /><span>Scheduler</span>
          </NuxtLink>
          <NuxtLink to="/settings" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <Settings class="w-4 h-4" /><span>Settings</span>
          </NuxtLink>
          <button @click="hideToTray"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors">
            <ChevronDown class="w-4 h-4" /><span>Hide to Tray</span>
          </button>
          <div class="flex items-center justify-between text-xs text-nanna-text-dim px-3 pt-2">
            <div class="flex items-center gap-2">
              <span>v0.1.0</span>
              <span v-if="backendStatus" class="px-1.5 py-0.5 rounded text-[10px]" 
                    :class="isDaemon ? 'bg-nanna-accent/20 text-nanna-accent' : 'bg-nanna-bg-elevated text-nanna-text-dim'">
                {{ isDaemon ? 'daemon' : 'embedded' }}
              </span>
            </div>
            <span :class="backendStatus?.connected ? 'text-nanna-success' : (apiKeySet ? 'text-nanna-warning' : 'text-nanna-error')">
              {{ backendStatus?.connected ? '● Connected' : (apiKeySet ? '○ Disconnected' : '○ No API Key') }}
            </span>
          </div>
        </div>
      </aside>
      
      <!-- Main area with workspace tabs -->
      <main class="flex-1 flex flex-col pt-14 lg:pt-0 relative overflow-hidden">
        <!-- Workspace Tabs (desktop only, on chat page) -->
        <WorkspaceTabs
          v-if="route.path === '/' || route.path === ''"
          class="hidden lg:flex"
          :open-workspaces="openWorkspaces"
          :current-tab="currentTab"
          @select="selectTab"
          @close="closeWorkspaceTab"
          @add="showWorkspacePicker = true"
        />
        
        <!-- Mobile workspace tabs (horizontal scroll) -->
        <WorkspaceTabs
          v-if="(route.path === '/' || route.path === '') && openWorkspaces.length > 0"
          class="lg:hidden"
          :open-workspaces="openWorkspaces"
          :current-tab="currentTab"
          @select="selectTab"
          @close="closeWorkspaceTab"
          @add="showWorkspacePicker = true"
        />
        
        <!-- Content area - takes remaining space, allows child to handle scrolling -->
        <div class="flex-1 overflow-hidden">
          <slot />
        </div>
      </main>
    </div>
    
    <!-- Workspace Picker Modal -->
    <WorkspacePicker
      v-model="showWorkspacePicker"
      :open-tab-ids="openTabIds"
      @select="openWorkspaceTab"
    />
    
    <!-- Close confirmation dialog -->
    <CloseDialog />

    <!-- Global confirmation dialog -->
    <ConfirmDialog />
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, provide } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Menu, Plus, Brain, Radio, Settings, ChevronDown, FolderKanban, Bot, Wrench, Clock, Globe, FileText } from 'lucide-vue-next'

interface SessionInfo {
  id: string
  name: string
  created_at: string
  updated_at: string
  message_count: number
  workspace_id: string | null
  workspace_name: string | null
}

interface WorkspaceInfo {
  id: string
  name: string
  path: string
  active?: boolean
}

interface Tab {
  type: 'global' | 'workspace'
  workspaceId?: string
}

interface AppConfig {
  theme: string
  model: string
  api_key_set: boolean
  available_models: string[]
}

const route = useRoute()

const sessions = ref<SessionInfo[]>([])
const currentSessionId = ref<string | null>(null)
const apiKeySet = ref(false)
const sidebarOpen = ref(false)
const showWorkspacePicker = ref(false)

// Workspace tabs state
const openWorkspaces = ref<WorkspaceInfo[]>([])
const currentTab = ref<Tab>({ type: 'global' })

let unlistenTrayNewChat: UnlistenFn | null = null
let unlistenCloseRequested: UnlistenFn | null = null
let unlistenSessionsCleared: UnlistenFn | null = null
let unlistenSessionRenamed: UnlistenFn | null = null

// Computed
const currentTabName = computed(() => {
  if (!currentTab.value || currentTab.value.type === 'global') {
    return 'Global'
  }
  const ws = openWorkspaces.value.find(w => w.id === currentTab.value.workspaceId)
  return ws?.name || 'Workspace'
})

const openTabIds = computed(() => openWorkspaces.value.map(w => w.id))

// For backwards compatibility - provide the current workspace if in workspace tab
const activeWorkspace = computed(() => {
  if (currentTab.value?.type === 'workspace') {
    return openWorkspaces.value.find(w => w.id === currentTab.value.workspaceId) || null
  }
  return null
})

// Tab management functions for child components
function addWorkspaceTab(ws: WorkspaceInfo) {
  if (!openWorkspaces.value.some(w => w.id === ws.id)) {
    openWorkspaces.value.push(ws)
    saveTabsToStorage()
  }
}

function selectWorkspaceTab(workspaceId: string) {
  // Ensure tab is open
  const ws = openWorkspaces.value.find(w => w.id === workspaceId)
  if (!ws) {
    // Need to fetch workspace info and add it
    loadOpenWorkspaces().then(() => {
      const found = openWorkspaces.value.find(w => w.id === workspaceId)
      if (found) {
        currentTab.value = { type: 'workspace', workspaceId }
      }
    })
  } else {
    currentTab.value = { type: 'workspace', workspaceId }
  }
}

function selectGlobalTab() {
  currentTab.value = { type: 'global' }
}

// Provide to child components
provide('currentSessionId', currentSessionId)
provide('sessions', sessions)
provide('activeWorkspace', activeWorkspace)
provide('currentTab', currentTab)
provide('openWorkspaces', openWorkspaces)
provide('addWorkspaceTab', addWorkspaceTab)
provide('selectWorkspaceTab', selectWorkspaceTab)
provide('selectGlobalTab', selectGlobalTab)

// Initialize notifications
const { checkPermission } = useNotifications()

// Initialize backend (daemon or embedded mode)
const { init: initBackend, status: backendStatus, isDaemon } = useBackend()

// Close handler
const { handleClose, loadCloseMode } = useCloseHandler()

// LocalStorage keys
const TABS_STORAGE_KEY = 'nanna-workspace-tabs'
const CURRENT_TAB_KEY = 'nanna-current-tab'

onMounted(async () => {
  // Initialize backend first
  const mode = await initBackend()
  console.log(`Nanna running in ${mode} mode`)
  
  // Load saved tabs from localStorage
  loadTabsFromStorage()
  
  // Load workspace data for open tabs
  await loadOpenWorkspaces()
  
  // Load sessions for current tab
  await loadSessions()
  await loadConfig()
  
  // Sync currentSessionId from URL query param
  const urlSessionId = route.query.session as string | undefined
  if (urlSessionId && sessions.value.some(s => s.id === urlSessionId)) {
    currentSessionId.value = urlSessionId
  }
  
  // Listen for tray "new chat" event
  unlistenTrayNewChat = await listen('tray-new-chat', () => {
    createNewSession()
  })

  // Listen for sessions cleared event (from settings page)
  unlistenSessionsCleared = await listen('sessions-cleared', async () => {
    console.log('Sessions cleared event received, refreshing...')
    await loadSessions()
    currentSessionId.value = sessions.value[0]?.id || null
  })

  // Listen for session renamed event (from auto-naming or other sources)
  unlistenSessionRenamed = await listen<{ id: string, name: string }>('session-renamed', (event) => {
    const { id, name } = event.payload
    const idx = sessions.value.findIndex(s => s.id === id)
    if (idx !== -1) {
      sessions.value[idx] = { ...sessions.value[idx], name }
    }
  })

  // Listen for window close request
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  const window = getCurrentWindow()
  unlistenCloseRequested = await window.onCloseRequested(async (event) => {
    event.preventDefault()
    await handleClose()
  })
  
  await loadCloseMode()
  await checkPermission()
})

onUnmounted(() => {
  if (unlistenTrayNewChat) unlistenTrayNewChat()
  if (unlistenCloseRequested) unlistenCloseRequested()
  if (unlistenSessionsCleared) unlistenSessionsCleared()
  if (unlistenSessionRenamed) unlistenSessionRenamed()
})

// Watch for route changes to sync currentSessionId
watch(() => route.query.session, (newSessionId) => {
  if (typeof newSessionId === 'string' && sessions.value.some(s => s.id === newSessionId)) {
    currentSessionId.value = newSessionId
  }
})

// Close sidebar on route change (mobile)
watch(() => route.fullPath, () => {
  sidebarOpen.value = false
})

// Reload sessions when tab changes
watch(currentTab, async () => {
  await loadSessions()
  // Reset currentSessionId when switching tabs
  currentSessionId.value = sessions.value[0]?.id || null
  if (currentSessionId.value) {
    navigateTo(`/?session=${currentSessionId.value}`)
  }
  saveTabsToStorage()
}, { deep: true })

// Storage helpers
function loadTabsFromStorage() {
  try {
    const savedTabs = localStorage.getItem(TABS_STORAGE_KEY)
    const savedCurrent = localStorage.getItem(CURRENT_TAB_KEY)
    
    if (savedTabs) {
      const tabIds: string[] = JSON.parse(savedTabs)
      // We'll populate openWorkspaces after loading from backend
      // For now just store the IDs
      openWorkspaces.value = tabIds.map(id => ({ id, name: '', path: '' }))
    }
    
    if (savedCurrent) {
      currentTab.value = JSON.parse(savedCurrent)
    }
  } catch (e) {
    console.error('Failed to load tabs from storage:', e)
  }
}

function saveTabsToStorage() {
  try {
    const tabIds = openWorkspaces.value.map(w => w.id)
    localStorage.setItem(TABS_STORAGE_KEY, JSON.stringify(tabIds))
    localStorage.setItem(CURRENT_TAB_KEY, JSON.stringify(currentTab.value))
  } catch (e) {
    console.error('Failed to save tabs to storage:', e)
  }
}

async function loadOpenWorkspaces() {
  try {
    // Get all workspaces from backend
    const allWorkspaces = await invoke<WorkspaceInfo[]>('list_workspaces')
    
    // Filter to only those we have tabs for
    const savedIds = openWorkspaces.value.map(w => w.id)
    openWorkspaces.value = allWorkspaces.filter(ws => savedIds.includes(ws.id))
    
    // Validate currentTab still exists
    if (currentTab.value?.type === 'workspace') {
      const exists = openWorkspaces.value.some(w => w.id === currentTab.value.workspaceId)
      if (!exists) {
        currentTab.value = { type: 'global' }
      }
    }
    
    saveTabsToStorage()
  } catch (e) {
    console.error('Failed to load workspaces:', e)
    openWorkspaces.value = []
  }
}

async function loadSessions() {
  try {
    const workspaceId = currentTab.value?.type === 'workspace' 
      ? currentTab.value.workspaceId ?? null 
      : null
    sessions.value = await invoke<SessionInfo[]>('list_sessions', { workspaceId })
    
    const firstSession = sessions.value[0]
    if (firstSession && !currentSessionId.value) {
      currentSessionId.value = firstSession.id
    }
  } catch (e) {
    console.error('Failed to load sessions:', e)
  }
}

async function loadConfig() {
  try {
    const config = await invoke<AppConfig>('get_config')
    apiKeySet.value = config.api_key_set
  } catch (e) {
    console.error('Failed to load config:', e)
  }
}

// Tab management
function selectTab(tab: Tab) {
  currentTab.value = tab
}

function openWorkspaceTab(ws: WorkspaceInfo) {
  // Add to open workspaces if not already there
  if (!openWorkspaces.value.some(w => w.id === ws.id)) {
    openWorkspaces.value.push(ws)
  }
  // Switch to the tab
  currentTab.value = { type: 'workspace', workspaceId: ws.id }
  saveTabsToStorage()
}

function closeWorkspaceTab(workspaceId: string) {
  openWorkspaces.value = openWorkspaces.value.filter(w => w.id !== workspaceId)
  
  // If closing current tab, switch to global
  if (currentTab.value?.type === 'workspace' && currentTab.value.workspaceId === workspaceId) {
    currentTab.value = { type: 'global' }
  }
  
  saveTabsToStorage()
}

async function createNewSession() {
  try {
    const workspaceId = currentTab.value?.type === 'workspace' 
      ? currentTab.value.workspaceId ?? null 
      : null
    const session = await invoke<SessionInfo>('create_session', { name: null, workspaceId })
    currentSessionId.value = session.id
    
    await loadSessions()
    navigateTo(`/?session=${session.id}`)
  } catch (e) {
    console.error('Failed to create session:', e)
  }
}

function switchSession(session: SessionInfo) {
  currentSessionId.value = session.id
  navigateTo(`/?session=${session.id}`)
}

function onSessionDeleted(sessionId: string) {
  sessions.value = sessions.value.filter(s => s.id !== sessionId)
  if (currentSessionId.value === sessionId) {
    currentSessionId.value = sessions.value[0]?.id || null
    if (currentSessionId.value) {
      navigateTo(`/?session=${currentSessionId.value}`)
    }
  }
}

function onSessionRenamed(updated: SessionInfo) {
  const idx = sessions.value.findIndex(s => s.id === updated.id)
  if (idx !== -1) {
    sessions.value[idx] = updated
  }
}

async function hideToTray() {
  try {
    await invoke('hide_to_tray')
  } catch (e) {
    console.error('Failed to hide to tray:', e)
  }
}
</script>
