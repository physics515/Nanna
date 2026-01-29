<template>
  <div class="flex flex-col h-full">
    <!-- Chat header -->
    <header class="px-6 py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <h2 class="text-lg font-semibold text-nanna-text">
        {{ currentSession?.name || 'New Chat' }}
      </h2>
      <p class="text-sm text-nanna-text-muted">
        Model: {{ config?.model || 'Loading...' }}
        <span v-if="config?.available_tools?.length" class="ml-2 text-nanna-secondary">
          • {{ config.available_tools.length }} tools
        </span>
      </p>
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
          <div v-if="config?.available_tools?.length" class="mt-4 text-sm text-nanna-text-dim">
            Available tools: {{ config.available_tools.join(', ') }}
          </div>
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
              
              <!-- Tool calls for this message -->
              <div v-if="msg.toolCalls?.length" class="mt-3 space-y-2">
                <ToolCallCard 
                  v-for="tool in msg.toolCalls" 
                  :key="tool.id"
                  :tool-call="tool"
                  :status="tool.success ? 'completed' : 'error'"
                />
              </div>
            </div>
          </div>
        </div>
      </div>
      
      <!-- Active tool calls during streaming -->
      <div v-if="activeToolCalls.length > 0" class="max-w-4xl mx-auto mr-12">
        <div class="space-y-2">
          <ToolCallCard 
            v-for="tool in activeToolCalls" 
            :key="tool.id"
            :tool-call="tool"
            :status="tool.status"
          />
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
              <div v-if="streamingContent" class="prose prose-invert prose-sm max-w-none">
                <span v-html="renderMarkdown(streamingContent)"></span>
                <span class="cursor-blink">▋</span>
              </div>
              <div v-else class="text-nanna-text-muted flex items-center gap-2">
                <span class="animate-pulse">●</span>
                <span class="animate-pulse" style="animation-delay: 0.2s">●</span>
                <span class="animate-pulse" style="animation-delay: 0.4s">●</span>
              </div>
            </div>
          </div>
        </div>
      </div>
      
      <!-- Loading indicator (before streaming starts) -->
      <div v-if="isLoading && !isStreaming && activeToolCalls.length === 0" class="max-w-4xl mx-auto">
        <MessageSkeleton :lines="2" />
      </div>
      
      <!-- Error message -->
      <ConnectionStatus
        :status="connectionError ? 'error' : 'connected'"
        :message="connectionError ?? undefined"
        :visible="!!connectionError"
        :can-retry="true"
        :can-dismiss="true"
        :is-retrying="isRetrying"
        @retry="retryLastMessage"
        @dismiss="dismissError"
      />
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
import { ref, inject, nextTick, onMounted, onUnmounted, type Ref } from 'vue'
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

interface ToolCallInfo {
  id: string
  name: string
  input: any
  output: string
  success: boolean
  duration_ms: number
  status?: 'started' | 'completed' | 'error'
}

interface Message {
  id: string
  role: 'user' | 'assistant'
  content: string
  timestamp: string
  toolCalls?: ToolCallInfo[]
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
  available_tools: string[]
}

interface StreamChunk {
  session_id: string
  chunk: string
  done: boolean
}

interface ToolCallEvent {
  session_id: string
  tool_call: ToolCallInfo
  status: 'started' | 'completed' | 'error'
}

// Inject currentSessionId from layout
const injectedSessionId = inject<Ref<string | null>>('currentSessionId')

const messages = ref<Message[]>([])
const input = ref('')
const isLoading = ref(false)
const isStreaming = ref(false)
const streamingContent = ref('')
const messagesContainer = ref<HTMLElement | null>(null)
const currentSession = ref<SessionInfo | null>(null)
const config = ref<AppConfig | null>(null)
const activeToolCalls = ref<(ToolCallInfo & { status: 'started' | 'completed' | 'error' })[]>([])
const connectionError = ref<string | null>(null)
const isRetrying = ref(false)
const lastUserMessage = ref<string>('')

let unlistenChunk: UnlistenFn | null = null
let unlistenTool: UnlistenFn | null = null

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
    const targetSessionId = injectedSessionId?.value
    
    if (sessions.length > 0) {
      // Find the session matching injectedSessionId, or fall back to first
      const targetSession = targetSessionId 
        ? sessions.find(s => s.id === targetSessionId) 
        : null
      currentSession.value = targetSession || sessions[0] || null
      
      if (currentSession.value) {
        // Load history
        messages.value = await invoke<Message[]>('get_session_history', { 
          sessionId: currentSession.value.id 
        })
        scrollToBottom()
      }
    } else {
      // No sessions exist, create one
      currentSession.value = await invoke<SessionInfo>('create_session', { name: null })
    }
  } catch (e) {
    console.error('Failed to load sessions:', e)
  }
  
  // Listen for streaming chunks
  unlistenChunk = await listen<StreamChunk>('stream-chunk', (event) => {
    console.log('stream-chunk event:', event.payload, 'current:', currentSession.value?.id)
    if (event.payload.session_id === currentSession.value?.id) {
      if (event.payload.done) {
        // Streaming complete
        isStreaming.value = false
        // Add the final message with tool calls
        if (streamingContent.value || activeToolCalls.value.length > 0) {
          messages.value.push({
            id: Date.now().toString(),
            role: 'assistant',
            content: streamingContent.value,
            timestamp: new Date().toISOString(),
            toolCalls: activeToolCalls.value.map(t => ({
              id: t.id,
              name: t.name,
              input: t.input,
              output: t.output,
              success: t.success,
              duration_ms: t.duration_ms,
            })),
          })
          streamingContent.value = ''
          activeToolCalls.value = []
        }
        isLoading.value = false
        scrollToBottom()
      } else {
        // Append chunk
        isStreaming.value = true
        streamingContent.value += event.payload.chunk
        scrollToBottom()
      }
    }
  })
  
  // Listen for tool call events
  unlistenTool = await listen<ToolCallEvent>('tool-call', (event) => {
    if (event.payload.session_id === currentSession.value?.id) {
      const { tool_call, status } = event.payload
      
      if (status === 'started') {
        // Add new tool call
        activeToolCalls.value.push({
          ...tool_call,
          status: 'started',
        })
      } else {
        // Update existing tool call
        const idx = activeToolCalls.value.findIndex(t => t.id === tool_call.id)
        if (idx !== -1) {
          activeToolCalls.value[idx] = {
            ...tool_call,
            status,
          }
        }
      }
      scrollToBottom()
    }
  })
})

onUnmounted(() => {
  if (unlistenChunk) unlistenChunk()
  if (unlistenTool) unlistenTool()
})

async function sendMessage() {
  if (!input.value.trim() || isLoading.value || !currentSession.value) return
  
  const userMessage = input.value.trim()
  input.value = ''
  lastUserMessage.value = userMessage
  connectionError.value = null
  
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
  
  await sendMessageToBackend(userMessage)
}

async function sendMessageToBackend(message: string) {
  // Start loading
  isLoading.value = true
  isStreaming.value = false
  streamingContent.value = ''
  activeToolCalls.value = []
  
  try {
    // Send message and wait for response (streaming happens via events)
    await invoke('send_message', { 
      sessionId: currentSession.value!.id,
      message 
    })
  } catch (error: any) {
    console.error('Failed to send message:', error)
    isLoading.value = false
    isStreaming.value = false
    activeToolCalls.value = []
    
    // Extract meaningful error message
    const errorMsg = error.message || String(error)
    if (errorMsg.includes('API key') || errorMsg.includes('authentication')) {
      connectionError.value = 'Invalid or missing API key. Please check your settings.'
    } else if (errorMsg.includes('rate limit')) {
      connectionError.value = 'Rate limited. Please wait a moment and try again.'
    } else if (errorMsg.includes('network') || errorMsg.includes('fetch')) {
      connectionError.value = 'Network error. Please check your connection.'
    } else {
      connectionError.value = errorMsg
    }
  }
  
  scrollToBottom()
}

async function retryLastMessage() {
  if (!lastUserMessage.value || isLoading.value) return
  
  isRetrying.value = true
  connectionError.value = null
  
  // Remove the last error message if present
  const lastMsg = messages.value[messages.value.length - 1]
  if (lastMsg && lastMsg.role === 'assistant' && lastMsg.content.startsWith('Error:')) {
    messages.value.pop()
  }
  
  await sendMessageToBackend(lastUserMessage.value)
  isRetrying.value = false
}

function dismissError() {
  connectionError.value = null
}

function scrollToBottom() {
  if (messagesContainer.value) {
    messagesContainer.value.scrollTop = messagesContainer.value.scrollHeight
  }
}
</script>
