<template>
  <div class="flex flex-col h-full">
    <!-- Chat header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <h2 class="text-base sm:text-lg font-semibold text-nanna-text truncate">
        {{ currentSession?.name || 'New Chat' }}
      </h2>
      <p class="text-xs sm:text-sm text-nanna-text-muted truncate">
        Model: {{ config?.model || 'Loading...' }}
        <span v-if="config?.available_tools?.length" class="ml-2 text-nanna-secondary">
          • {{ config.available_tools.length }} tools
        </span>
      </p>
    </header>
    
    <!-- Messages area -->
    <div ref="messagesContainer" class="flex-1 overflow-y-auto p-4 sm:p-6 space-y-4">
      <!-- Welcome message -->
      <div v-if="messages.length === 0" class="flex items-center justify-center h-full">
        <div class="text-center max-w-md px-4">
          <div class="text-5xl sm:text-6xl mb-4">🌙</div>
          <h3 class="text-xl sm:text-2xl font-bold text-nanna-accent crt-glow mb-2">
            Nanna
          </h3>
          <p class="text-nanna-text-muted italic mb-2 text-sm sm:text-base">
            Patron deity of Ur
          </p>
          <p class="text-nanna-text-dim text-xs sm:text-sm">
            The moon is here. What would you illuminate?
          </p>
          <div v-if="config?.available_tools?.length" class="mt-6 text-xs text-nanna-text-dim opacity-60">
            {{ config.available_tools.length }} tools await
          </div>
        </div>
      </div>
      
      <!-- Messages -->
      <div v-for="(msg, idx) in messages" :key="msg.id || idx" class="max-w-4xl mx-auto">
        <div 
          :class="[
            'p-3 sm:p-4 rounded-lg',
            msg.role === 'user' ? 'message-user ml-4 sm:ml-12' : 'message-assistant mr-4 sm:mr-12'
          ]"
        >
          <div class="flex items-start gap-2 sm:gap-3">
            <UiAvatar 
              :variant="msg.role === 'user' ? 'primary' : 'accent'"
              :fallback="msg.role === 'user' ? 'U' : '☽'"
              size="sm"
              class="flex-shrink-0 sm:hidden"
            />
            <UiAvatar 
              :variant="msg.role === 'user' ? 'primary' : 'accent'"
              :fallback="msg.role === 'user' ? 'U' : '☽'"
              class="flex-shrink-0 hidden sm:flex"
            />
            <div class="flex-1 min-w-0">
              <div class="text-xs text-nanna-text-dim mb-1">
                {{ msg.role === 'user' ? 'You' : '☽ Nanna' }}
              </div>
              <div 
                v-if="msg.role === 'assistant'"
                class="prose prose-invert prose-sm max-w-none break-words"
                v-html="renderMarkdown(msg.content)"
              />
              <div v-else class="text-nanna-text text-sm sm:text-base whitespace-pre-wrap break-words">
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
      <div v-if="activeToolCalls.length > 0" class="max-w-4xl mx-auto mr-4 sm:mr-12">
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
        <div class="message-assistant p-3 sm:p-4 rounded-lg mr-4 sm:mr-12">
          <div class="flex items-start gap-2 sm:gap-3">
            <UiAvatar variant="accent" fallback="☽" size="sm" class="flex-shrink-0 sm:hidden" />
            <UiAvatar variant="accent" fallback="☽" class="flex-shrink-0 hidden sm:flex" />
            <div class="flex-1">
              <div class="text-xs text-nanna-text-dim mb-1">☽ Nanna</div>
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
    <div class="p-3 sm:p-4 border-t border-nanna-primary/10 bg-nanna-bg-surface/50">
      <form @submit.prevent="sendMessage" class="max-w-4xl mx-auto">
        <div class="flex gap-2 sm:gap-3">
          <UiInput
            v-model="input"
            type="text"
            placeholder="Type your message..."
            :disabled="isLoading"
            class="flex-1"
            @keydown.enter.exact.prevent="sendMessage"
          />
          <UiButton 
            type="submit" 
            :disabled="!input.trim() || isLoading"
            class="shrink-0"
          >
            <Send class="w-4 h-4 sm:hidden" />
            <span class="hidden sm:inline">Send</span>
          </UiButton>
        </div>
        <div class="mt-2 text-xs text-nanna-text-dim hidden sm:block">
          Press Enter to send
        </div>
      </form>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, nextTick, onMounted, onUnmounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { marked } from 'marked'
import { Send } from 'lucide-vue-next'

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

// Notifications
const { notifyToolComplete, notifyError, notifyMessage } = useNotifications()

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

// Get session ID from URL query param
const route = useRoute()

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

// Load session data based on current route
async function loadSession() {
  try {
    const sessions = await invoke<SessionInfo[]>('list_sessions')
    const targetSessionId = route.query.session as string | undefined
    
    if (sessions.length > 0) {
      // Find the session matching URL query, or fall back to first
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
      messages.value = []
    }
  } catch (e) {
    console.error('Failed to load sessions:', e)
  }
}

onMounted(async () => {
  // Load config
  try {
    config.value = await invoke<AppConfig>('get_config')
  } catch (e) {
    console.error('Failed to load config:', e)
  }
  
  // Load initial session
  await loadSession()
  
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
          
          // Notify if window is not focused
          if (document.hidden && streamingContent.value) {
            notifyMessage(streamingContent.value)
          }
          
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
        
        // Notify on tool completion if window not focused
        if (document.hidden) {
          notifyToolComplete(tool_call.name, tool_call.success)
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

// Watch for session changes in URL
watch(() => route.query.session, async (newSessionId) => {
  if (newSessionId && newSessionId !== currentSession.value?.id) {
    // Reset state for new session
    messages.value = []
    streamingContent.value = ''
    activeToolCalls.value = []
    isLoading.value = false
    isStreaming.value = false
    connectionError.value = null
    
    // Load new session
    await loadSession()
  }
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
    
    // Notify on error if window not focused
    if (document.hidden) {
      notifyError('Message Failed', connectionError.value)
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
