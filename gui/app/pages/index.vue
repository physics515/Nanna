<template>
  <div class="flex flex-col h-full">
    <!-- Chat header -->
    <header class="px-6 py-4 border-b border-nanna-primary/10 bg-nanna-bg-surface/50">
      <h2 class="text-lg font-semibold text-nanna-text">Chat</h2>
      <p class="text-sm text-nanna-text-muted">Model: claude-sonnet-4-20250514</p>
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
      <div v-for="(msg, idx) in messages" :key="idx" class="max-w-4xl mx-auto">
        <div 
          :class="[
            'p-4 rounded-lg',
            msg.role === 'user' ? 'message-user ml-12' : 'message-assistant mr-12'
          ]"
        >
          <div class="flex items-start gap-3">
            <div 
              :class="[
                'w-8 h-8 rounded-full flex items-center justify-center text-sm font-bold',
                msg.role === 'user' 
                  ? 'bg-nanna-primary text-white' 
                  : 'bg-nanna-accent text-nanna-bg-deep'
              ]"
            >
              {{ msg.role === 'user' ? 'U' : 'N' }}
            </div>
            <div class="flex-1">
              <div class="text-xs text-nanna-text-dim mb-1">
                {{ msg.role === 'user' ? 'You' : 'Nanna' }}
              </div>
              <div class="text-nanna-text whitespace-pre-wrap">
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
      
      <!-- Loading indicator -->
      <div v-if="isLoading" class="max-w-4xl mx-auto">
        <div class="message-assistant p-4 rounded-lg mr-12">
          <div class="flex items-center gap-3">
            <div class="w-8 h-8 rounded-full bg-nanna-accent text-nanna-bg-deep flex items-center justify-center">
              N
            </div>
            <div class="flex items-center gap-2 text-nanna-text-muted">
              <span class="animate-pulse">●</span>
              <span class="animate-pulse delay-100">●</span>
              <span class="animate-pulse delay-200">●</span>
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
          Press Enter to send • Shift+Enter for new line
        </div>
      </form>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick } from 'vue'

interface ToolCall {
  name: string
  input: any
  output: string
}

interface Message {
  role: 'user' | 'assistant'
  content: string
  toolCalls?: ToolCall[]
}

const messages = ref<Message[]>([])
const input = ref('')
const isLoading = ref(false)
const messagesContainer = ref<HTMLElement | null>(null)

async function sendMessage() {
  if (!input.value.trim() || isLoading.value) return
  
  const userMessage = input.value.trim()
  input.value = ''
  
  // Add user message
  messages.value.push({
    role: 'user',
    content: userMessage,
  })
  
  // Scroll to bottom
  await nextTick()
  scrollToBottom()
  
  // Send to backend
  isLoading.value = true
  try {
    // TODO: Use Tauri invoke
    // const response = await invoke('send_message', { message: userMessage })
    
    // Simulate response for now
    await new Promise(resolve => setTimeout(resolve, 1000))
    
    messages.value.push({
      role: 'assistant',
      content: `Echo: ${userMessage}`,
      toolCalls: [],
    })
  } catch (error) {
    console.error('Failed to send message:', error)
    messages.value.push({
      role: 'assistant',
      content: 'Sorry, there was an error processing your request.',
    })
  } finally {
    isLoading.value = false
    await nextTick()
    scrollToBottom()
  }
}

function scrollToBottom() {
  if (messagesContainer.value) {
    messagesContainer.value.scrollTop = messagesContainer.value.scrollHeight
  }
}
</script>
