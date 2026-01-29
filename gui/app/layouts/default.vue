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
        <div class="p-4 border-t border-nanna-primary/10 space-y-2">
          <NuxtLink 
            to="/settings" 
            class="block w-full text-left px-3 py-2 rounded-lg text-sm text-nanna-text-muted hover:text-nanna-text hover:bg-nanna-bg-elevated transition-colors"
          >
            ⚙️ Settings
          </NuxtLink>
          <div class="flex items-center justify-between text-xs text-nanna-text-dim px-3">
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
import { ref, onMounted, provide } from 'vue'
import { invoke } from '@tauri-apps/api/core'

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

const sessions = ref<SessionInfo[]>([])
const currentSessionId = ref<string | null>(null)
const apiKeySet = ref(false)

// Provide session switching to child components
provide('currentSessionId', currentSessionId)
provide('sessions', sessions)

onMounted(async () => {
  await loadSessions()
  await loadConfig()
})

async function loadSessions() {
  try {
    sessions.value = await invoke<SessionInfo[]>('list_sessions')
    if (sessions.value.length > 0 && !currentSessionId.value) {
      currentSessionId.value = sessions.value[0].id
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
    sessions.value.unshift(session)
    currentSessionId.value = session.id
    // Navigate to chat
    navigateTo('/')
    // Force page reload to switch context
    if (window.location.pathname === '/') {
      window.location.reload()
    }
  } catch (e) {
    console.error('Failed to create session:', e)
  }
}

function switchSession(session: SessionInfo) {
  currentSessionId.value = session.id
  // Navigate and reload
  if (window.location.pathname !== '/') {
    navigateTo('/')
  } else {
    window.location.reload()
  }
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
</script>
