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
        <!-- Empty - we use the header button -->
        <span></span>
      </template>
      
      <div class="flex flex-col h-full -m-6">
        <!-- Logo -->
        <div class="p-4 border-b border-nanna-primary/10">
          <NuxtLink to="/" @click="sidebarOpen = false" class="block">
            <h1 class="text-2xl font-bold text-nanna-accent crt-glow">
              NANNA
            </h1>
            <p class="text-xs text-nanna-text-muted mt-1">
              AI Assistant
            </p>
          </NuxtLink>
        </div>
        
        <!-- New Chat button -->
        <div class="p-4">
          <UiButton 
            @click="createNewSession(); sidebarOpen = false"
            class="w-full justify-start"
          >
            <Plus class="w-4 h-4" />
            <span>New Chat</span>
          </UiButton>
        </div>
        
        <!-- Workspace indicator (mobile) -->
        <div v-if="activeWorkspace" class="px-4 py-2 bg-nanna-accent/10 border-b border-nanna-accent/20">
          <div class="flex items-center gap-2 text-xs">
            <FolderKanban class="w-3 h-3 text-nanna-accent" />
            <span class="text-nanna-accent font-medium truncate">{{ activeWorkspace.name }}</span>
          </div>
        </div>
        <div v-else class="px-4 py-2 bg-nanna-bg-elevated/50 border-b border-nanna-primary/10">
          <div class="flex items-center gap-2 text-xs text-nanna-text-dim">
            <span>Global</span>
            <span class="text-[10px] opacity-60">(all memory)</span>
          </div>
        </div>
        
        <!-- Sessions list -->
        <nav class="flex-1 px-4 space-y-1 overflow-y-auto pt-2">
          <div class="text-xs text-nanna-text-dim uppercase tracking-wider mb-2">
            {{ activeWorkspace ? 'Workspace Chats' : 'Global Chats' }}
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
            {{ activeWorkspace ? 'No workspace chats yet' : 'No global chats yet' }}
          </div>
        </nav>
        
        <!-- Footer -->
        <div class="p-4 border-t border-nanna-primary/10 space-y-1">
          <NuxtLink 
            to="/memory" 
            @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Brain class="w-4 h-4" />
            <span>Memory</span>
          </NuxtLink>
          <NuxtLink 
            to="/workspaces" 
            @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <FolderKanban class="w-4 h-4" />
            <span>Workspaces</span>
          </NuxtLink>
          <NuxtLink 
            to="/agents" 
            @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Bot class="w-4 h-4" />
            <span>Agents</span>
          </NuxtLink>
          <NuxtLink 
            to="/channels" 
            @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Radio class="w-4 h-4" />
            <span>Channels</span>
          </NuxtLink>
          <NuxtLink 
            to="/tools" 
            @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Wrench class="w-4 h-4" />
            <span>Tools</span>
          </NuxtLink>
          <NuxtLink 
            to="/settings" 
            @click="sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Settings class="w-4 h-4" />
            <span>Settings</span>
          </NuxtLink>
          <button 
            @click="hideToTray(); sidebarOpen = false"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <ChevronDown class="w-4 h-4" />
            <span>Hide to Tray</span>
          </button>
          <div class="flex items-center justify-between text-xs text-nanna-text-dim px-3 pt-2">
            <span>v0.1.0</span>
            <span :class="apiKeySet ? 'text-nanna-success' : 'text-nanna-error'">
              {{ apiKeySet ? '● Connected' : '○ No API Key' }}
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
            <h1 class="text-2xl font-bold text-nanna-accent crt-glow">
              NANNA
            </h1>
            <p class="text-xs text-nanna-text-muted mt-1">
              AI Assistant
            </p>
          </NuxtLink>
        </div>
        
        <!-- New Chat button -->
        <div class="p-4">
          <UiButton 
            @click="createNewSession"
            class="w-full justify-start"
          >
            <Plus class="w-4 h-4" />
            <span>New Chat</span>
          </UiButton>
        </div>
        
        <!-- Workspace indicator -->
        <div v-if="activeWorkspace" class="px-4 py-2 bg-nanna-accent/10 border-b border-nanna-accent/20">
          <div class="flex items-center gap-2 text-xs">
            <FolderKanban class="w-3 h-3 text-nanna-accent" />
            <span class="text-nanna-accent font-medium truncate">{{ activeWorkspace.name }}</span>
          </div>
        </div>
        <div v-else class="px-4 py-2 bg-nanna-bg-elevated/50 border-b border-nanna-primary/10">
          <div class="flex items-center gap-2 text-xs text-nanna-text-dim">
            <span>Global</span>
            <span class="text-[10px] opacity-60">(all memory)</span>
          </div>
        </div>
        
        <!-- Sessions list -->
        <nav class="flex-1 px-4 space-y-1 overflow-y-auto pt-2">
          <div class="text-xs text-nanna-text-dim uppercase tracking-wider mb-2">
            {{ activeWorkspace ? 'Workspace Chats' : 'Global Chats' }}
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
            {{ activeWorkspace ? 'No workspace chats yet' : 'No global chats yet' }}
          </div>
        </nav>
        
        <!-- Footer -->
        <div class="p-4 border-t border-nanna-primary/10 space-y-1">
          <NuxtLink 
            to="/memory" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Brain class="w-4 h-4" />
            <span>Memory</span>
          </NuxtLink>
          <NuxtLink 
            to="/workspaces" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <FolderKanban class="w-4 h-4" />
            <span>Workspaces</span>
          </NuxtLink>
          <NuxtLink 
            to="/agents" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Bot class="w-4 h-4" />
            <span>Agents</span>
          </NuxtLink>
          <NuxtLink 
            to="/channels" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Radio class="w-4 h-4" />
            <span>Channels</span>
          </NuxtLink>
          <NuxtLink 
            to="/tools" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Wrench class="w-4 h-4" />
            <span>Tools</span>
          </NuxtLink>
          <NuxtLink 
            to="/settings" 
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <Settings class="w-4 h-4" />
            <span>Settings</span>
          </NuxtLink>
          <button 
            @click="hideToTray"
            class="flex items-center gap-3 w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            <ChevronDown class="w-4 h-4" />
            <span>Hide to Tray</span>
          </button>
          <div class="flex items-center justify-between text-xs text-nanna-text-dim px-3 pt-2">
            <span>v0.1.0</span>
            <span :class="apiKeySet ? 'text-nanna-success' : 'text-nanna-error'">
              {{ apiKeySet ? '● Connected' : '○ No API Key' }}
            </span>
          </div>
        </div>
      </aside>
      
      <!-- Main area -->
      <main class="flex-1 flex flex-col pt-14 lg:pt-0">
        <slot />
      </main>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, provide } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { Menu, Plus, Brain, Radio, Settings, ChevronDown, FolderKanban, Bot, Wrench } from 'lucide-vue-next'

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
  active: boolean
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
const activeWorkspace = ref<WorkspaceInfo | null>(null)

let unlistenTrayNewChat: UnlistenFn | null = null

// Provide session switching to child components
provide('currentSessionId', currentSessionId)
provide('sessions', sessions)
provide('activeWorkspace', activeWorkspace)

// Initialize notifications
const { checkPermission } = useNotifications()

onMounted(async () => {
  // Load active workspace first, then sessions (sessions filtered by workspace)
  await loadActiveWorkspace()
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
  
  // Check notification permissions on mount
  await checkPermission()
})

onUnmounted(() => {
  if (unlistenTrayNewChat) unlistenTrayNewChat()
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

async function loadActiveWorkspace() {
  try {
    activeWorkspace.value = await invoke<WorkspaceInfo | null>('get_active_workspace')
  } catch (e) {
    console.error('Failed to load active workspace:', e)
    activeWorkspace.value = null
  }
}

async function loadSessions() {
  try {
    // Pass workspace_id to filter sessions
    // null = show only global sessions, Some(id) = show that workspace's sessions
    const workspaceId = activeWorkspace.value?.id ?? null
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

async function createNewSession() {
  try {
    // Create session in the active workspace (or global if no workspace)
    const workspaceId = activeWorkspace.value?.id ?? null
    const session = await invoke<SessionInfo>('create_session', { 
      name: null, 
      workspaceId 
    })
    currentSessionId.value = session.id
    
    // Reload sessions list from backend to avoid duplicates
    await loadSessions()
    
    // Navigate with session ID in query
    navigateTo(`/?session=${session.id}`)
  } catch (e) {
    console.error('Failed to create session:', e)
  }
}

function switchSession(session: SessionInfo) {
  currentSessionId.value = session.id
  // Navigate with session ID in query
  navigateTo(`/?session=${session.id}`)
}

function onSessionDeleted(sessionId: string) {
  sessions.value = sessions.value.filter(s => s.id !== sessionId)
  if (currentSessionId.value === sessionId) {
    currentSessionId.value = sessions.value[0]?.id || null
    if (currentSessionId.value) {
      window.location.reload()
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
