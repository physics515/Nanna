<template>
  <div class="min-h-screen bg-nanna-bg-deep bg-grid relative">
    <!-- Main content -->
    <div class="flex h-screen">
      <!-- Sidebar -->
      <aside class="w-64 bg-nanna-bg-surface border-r border-nanna-primary/10 flex flex-col">
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
          <button 
            @click="createNewSession"
            class="w-full btn-primary text-left flex items-center gap-2"
          >
            <span>+</span>
            <span>New Chat</span>
          </button>
        </div>
        
        <!-- Sessions list -->
        <nav class="flex-1 px-4 space-y-1 overflow-y-auto">
          <div class="text-xs text-nanna-text-dim uppercase tracking-wider mb-2">
            Recent Chats
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
            No chats yet
          </div>
        </nav>
        
        <!-- Footer -->
        <div class="p-4 border-t border-nanna-primary/10 space-y-1">
          <NuxtLink 
            to="/memory" 
            class="block w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            📚 Memory
          </NuxtLink>
          <NuxtLink 
            to="/settings" 
            class="block w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            ⚙️ Settings
          </NuxtLink>
          <button 
            @click="hideToTray"
            class="block w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            🔽 Hide to Tray
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
      <main class="flex-1 flex flex-col">
        <slot />
      </main>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted, provide } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

interface SessionInfo {
  id: string
  name: string
  created_at: string
  updated_at: string
  message_count: number
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

let unlistenTrayNewChat: UnlistenFn | null = null

// Provide session switching to child components
provide('currentSessionId', currentSessionId)
provide('sessions', sessions)

onMounted(async () => {
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

async function loadSessions() {
  try {
    sessions.value = await invoke<SessionInfo[]>('list_sessions')
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
    const session = await invoke<SessionInfo>('create_session', { name: null })
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
