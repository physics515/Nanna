<template>
  <div class="flex flex-col h-full">
    <!-- Chat header -->
    <header class="px-6 py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <h2 class="text-lg font-semibold text-nanna-text">
        {{ currentSession?.name || 'New Chat' }}
      </h2>
      <p class="text-sm text-nanna-text-muted">Model: {{ config?.model || 'Loading...' }}</p>
    </header>
    
    <!-- Messages area -->
    <div ref="messagesContainer" class="flex-1 overflow-y-auto p-6 space-y-4">
      <!-- Welcome message -->
      <div v-if="messages.length === 0" class="flex items-center justify-center h-full">
        <div class="text-center max-w-md">
          <div class="text-6xl mb-4">🌙</div>
          <h3 class="text-2xl font-bold text-nanna-accent crt-glow mb-2">
            Welcome to Nanna
          </h3>
          <p class="text-nanna-text-muted">
            Your AI assistant is ready. Type a message to begin.
          </p>
        </div>
      </div>
      
      <!-- Messages -->
      <div v-for="(msg, idx) in messages" :key="msg.id || idx" class="max-w-4xl mx-auto">
        <div 
          :class="[
            'p-4 rounded-lg',
            msg.role === 'user' ? 'message-user ml-12' : 'message-assistant mr-12'
          ]"
        >
          <div class="flex items-start gap-3">
            <div 
              :class="[
                'w-8 h-8 rounded-full flex items-center justify-center text-sm font-bold flex-shrink-0',
                msg.role === 'user' 
                  ? 'bg-nanna-primary text-white' 
                  : 'bg-nanna-accent text-nanna-bg-deep'
              ]"
            >
              {{ msg.role === 'user' ? 'U' : 'N' }}
            </div>
            <div class="flex-1 min-w-0">
              <div class="text-xs text-nanna-text-dim mb-1">
                {{ msg.role === 'user' ? 'You' : 'Nanna' }}
              </div>
              <div 
                v-if="msg.role === 'assistant'"
                class="prose prose-invert prose-sm max-w-none"
                v-html="renderMarkdown(msg.content)"
              />
              <div v-else class="text-nanna-text whitespace-pre-wrap break-words">
                {{ msg.content }}
              </div>
              
              <!-- Tool calls -->
              <div v-if="msg.toolCalls?.length" class="mt-3 space-y-2">
                <div v-for="(tool, tidx) in msg.toolCalls" :key="tidx" class="tool-call">
                  <div class="text-nanna-secondary font-semibold">
                    🔧 {{ tool.name }}
                  </div>
                  <pre class="text-xs text-nanna-text-muted mt-1 overflow-x-auto">{{ JSON.stringify(tool.input, null, 2) }}</pre>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
      
      <!-- Streaming indicator -->
      <div v-if="isStreaming" class="max-w-4xl mx-auto">
        <div class="message-assistant p-4 rounded-lg mr-12">
          <div class="flex items-start gap-3">
            <div class="w-8 h-8 rounded-full bg-nanna-accent text-nanna-bg-deep flex items-center justify-center flex-shrink-0">
              N
            </div>
            <div class="flex-1">
              <div class="text-xs text-nanna-text-dim mb-1">Nanna</div>
              <div class="prose prose-invert prose-sm max-w-none">
                <span v-html="renderMarkdown(streamingContent)"></span>
                <span class="cursor-blink">▋</span>
              </div>
            </div>
          </div>
        </div>
      </div>
      
      <!-- Loading indicator (before streaming starts) -->
      <div v-if="isLoading && !isStreaming" class="max-w-4xl mx-auto">
        <div class="message-assistant p-4 rounded-lg mr-12">
          <div class="flex items-center gap-3">
            <div class="w-8 h-8 rounded-full bg-nanna-accent text-nanna-bg-deep flex items-center justify-center">
              N
            </div>
            <div class="flex items-center gap-2 text-nanna-text-muted">
              <span class="animate-pulse">●</span>
              <span class="animate-pulse" style="animation-delay: 0.2s">●</span>
              <span class="animate-pulse" style="animation-delay: 0.4s">●</span>
            </div>
          </div>
        </div>
      </div>
    </div>
    
    <!-- Input area -->
    <div class="p-4 border-t border-nanna-primary/10 bg-nanna-bg-surface/50">
      <form @submit.prevent="sendMessage" class="max-w-4xl mx-auto">
        <div class="flex gap-3">
          <input
            v-model="input"
            type="text"
            placeholder="Type your message..."
            class="input flex-1"
            :disabled="isLoading"
            @keydown.enter.exact.prevent="sendMessage"
          />
          <button 
            type="submit" 
            class="btn-primary"
            :disabled="!input.trim() || isLoading"
          >
            Send
          </button>
        </div>
        <div class="mt-2 text-xs text-nanna-text-dim">
          Press Enter to send
        </div>
      </form>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick, onMounted, onUnmounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { marked } from 'marked'

// Configure marked for safe rendering
marked.setOptions({
  breaks: true,
  gfm: true,
})

function renderMarkdown(content: string): string {
  try {
    return marked.parse(content) as string
  } catch {
    return content
  }
}

interface ToolCall {
  id: string
  name: string
  input: any
  output: string
  success: boolean
}

interface Message {
  id: string
  role: 'user' | 'assistant'
  content: string
  timestamp: string
  toolCalls?: ToolCall[]
}

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

interface StreamChunk {
  session_id: string
  chunk: string
  done: boolean
}

const messages = ref<Message[]>([])
const input = ref('')
const isLoading = ref(false)
const isStreaming = ref(false)
const streamingContent = ref('')
const messagesContainer = ref<HTMLElement | null>(null)
const currentSession = ref<SessionInfo | null>(null)
const config = ref<AppConfig | null>(null)

let unlisten: UnlistenFn | null = null

onMounted(async () => {
  // Load config
  try {
    config.value = await invoke<AppConfig>('get_config')
  } catch (e) {
    console.error('Failed to load config:', e)
  }
  
  // Try to get or create a session
  try {
    const sessions = await invoke<SessionInfo[]>('list_sessions')
    if (sessions.length > 0) {
      currentSession.value = sessions[0]
      // Load history
      messages.value = await invoke<Message[]>('get_session_history', { 
        sessionId: currentSession.value.id 
      })
      scrollToBottom()
    } else {
      // Create new session
      currentSession.value = await invoke<SessionInfo>('create_session', { name: null })
    }
  } catch (e) {
    console.error('Failed to load sessions:', e)
  }
  
  // Listen for streaming chunks
  unlisten = await listen<StreamChunk>('stream-chunk', (event) => {
    if (event.payload.session_id === currentSession.value?.id) {
      if (event.payload.done) {
        // Streaming complete
        isStreaming.value = false
        // Add the final message
        if (streamingContent.value) {
          messages.value.push({
            id: Date.now().toString(),
            role: 'assistant',
            content: streamingContent.value,
            timestamp: new Date().toISOString(),
            toolCalls: [],
          })
          streamingContent.value = ''
        }
        isLoading.value = false
        scrollToBottom()
      } else {
        // Append chunk
        streamingContent.value += event.payload.chunk
        scrollToBottom()
      }
    }
  })
})

onUnmounted(() => {
  if (unlisten) {
    unlisten()
  }
})

async function sendMessage() {
  if (!input.value.trim() || isLoading.value || !currentSession.value) return
  
  const userMessage = input.value.trim()
  input.value = ''
  
  // Add user message immediately
  messages.value.push({
    id: Date.now().toString(),
    role: 'user',
    content: userMessage,
    timestamp: new Date().toISOString(),
  })
  
  // Scroll to bottom
  await nextTick()
  scrollToBottom()
  
  // Start loading
  isLoading.value = true
  isStreaming.value = true
  streamingContent.value = ''
  
  try {
    // Send message and wait for response (streaming happens via events)
    await invoke('send_message', { 
      sessionId: currentSession.value.id,
      message: userMessage 
    })
  } catch (error: any) {
    console.error('Failed to send message:', error)
    isLoading.value = false
    isStreaming.value = false
    // Show error
    messages.value.push({
      id: Date.now().toString(),
      role: 'assistant',
      content: `Error: ${error.message || error}`,
      timestamp: new Date().toISOString(),
    })
  }
  
  scrollToBottom()
}

function scrollToBottom() {
  if (messagesContainer.value) {
    messagesContainer.value.scrollTop = messagesContainer.value.scrollHeight
  }
}
</script>
