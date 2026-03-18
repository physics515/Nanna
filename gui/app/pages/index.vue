<template>
  <div class="flex flex-col h-full">
    <!-- Chat header -->
    <header class="px-4 sm:px-6 py-3 sm:py-4">
      <div class="flex items-center justify-between gap-2">
        <h2 class="text-base sm:text-lg font-semibold text-nanna-text truncate">
          {{ currentSession?.name || 'New Chat' }}
        </h2>
        <div class="flex items-center gap-2">
          <!-- Active work indicator -->
          <SessionActivityBadge v-if="currentSession" :session-id="currentSession.id" />
          <!-- Daemon-level queue depth (messages from other channels waiting) -->
          <span
            v-if="daemonQueueCount > 0"
            class="flex items-center gap-1 text-xs text-nanna-text-muted bg-nanna-bg-surface border border-white/[0.06] rounded-full px-2 py-0.5"
            :title="`${daemonQueueCount} message${daemonQueueCount > 1 ? 's' : ''} queued at daemon`"
          >
            <svg class="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
                d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
            </svg>
            {{ daemonQueueCount }} queued
          </span>
          <ModelStatusBadge />
        </div>
      </div>
      <p class="text-xs text-nanna-text-dim mt-1">
        <span v-if="config?.available_tools?.length">
          {{ config.available_tools.length }} tools available
        </span>
      </p>
    </header>

    <!-- Messages area -->
    <div ref="messagesContainer" @scroll="handleScroll" class="flex-1 overflow-y-auto p-4 sm:p-6 space-y-4">
      <!-- Welcome message -->
      <div v-if="messages.length === 0 && !hasActiveWork" class="flex items-center justify-center h-full">
        <div class="text-center max-w-md px-4">
          <img src="/logo.svg" alt="Nanna" class="w-32 sm:w-40 mx-auto mb-4" />
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
      <template v-for="(msg, idx) in messages" :key="msg.id || idx">
        <!-- Tool calls rendered BEFORE the assistant response (between user msg and response) -->
        <div v-if="msg.role === 'assistant' && msg.tool_calls?.length" class="max-w-[1800px] mx-auto">
          <div class="space-y-1 mx-4 sm:mx-12 my-2">
            <ToolCallCard
              v-for="tool in msg.tool_calls"
              :key="tool.id"
              :tool-call="tool"
              :status="tool.success ? 'completed' : 'error'"
            />
          </div>
        </div>

        <!-- Message bubble -->
        <div class="max-w-[1800px] mx-auto" :class="msg.role === 'user' ? 'ml-4 sm:ml-12' : 'mr-4 sm:mr-12'">
          <MessageBubble :variant="msg.role">
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
                <MarkdownContent :content="msg.content" />

                <!-- Reasoning/thinking block (collapsible) -->
                <details v-if="msg.reasoning" class="mt-2 text-xs">
                  <summary class="cursor-pointer text-nanna-text-dim hover:text-nanna-text-secondary">
                    Thinking
                  </summary>
                  <div class="mt-1 p-2 bg-nanna-bg-deep rounded whitespace-pre-wrap max-h-[200px] overflow-y-auto text-nanna-text-dim">
                    {{ msg.reasoning }}
                  </div>
                </details>
              </div>
            </div>
          </MessageBubble>
        </div>
      </template>

      <!-- Active tool calls during streaming -->
      <div v-if="activeToolCalls.length > 0" class="max-w-[1800px] mx-auto mr-4 sm:mr-12">
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
      <div v-if="isStreaming" class="max-w-[1800px] mx-auto mr-4 sm:mr-12">
        <MessageBubble variant="assistant">
          <div class="flex items-start gap-2 sm:gap-3">
            <UiAvatar variant="accent" fallback="☽" size="sm" class="flex-shrink-0 sm:hidden" />
            <UiAvatar variant="accent" fallback="☽" class="flex-shrink-0 hidden sm:flex" />
            <div class="flex-1">
              <div class="text-xs text-nanna-text-dim mb-1">☽ Nanna</div>
              <!-- Live thinking indicator -->
              <details v-if="streamingThinking" class="mb-2 text-xs" open>
                <summary class="cursor-pointer text-nanna-text-dim hover:text-nanna-text-secondary">
                  Thinking...
                </summary>
                <div class="mt-1 p-2 bg-nanna-bg-deep rounded whitespace-pre-wrap max-h-[150px] overflow-y-auto text-nanna-text-dim">
                  {{ streamingThinking }}
                </div>
              </details>
              <div v-if="streamingContent" class="prose prose-invert prose-sm max-w-none">
                <MarkdownContent :content="streamingContent" />
                <span class="cursor-blink inline-block ml-0.5">▋</span>
              </div>
              <div v-else-if="!streamingThinking" class="text-nanna-text-muted flex items-center gap-2">
                <span class="animate-pulse">●</span>
                <span class="animate-pulse" style="animation-delay: 0.2s">●</span>
                <span class="animate-pulse" style="animation-delay: 0.4s">●</span>
              </div>
            </div>
          </div>
        </MessageBubble>
      </div>

      <!-- Loading indicator (before streaming starts) -->
      <div v-if="isLoading && !isStreaming && activeToolCalls.length === 0" class="max-w-[1800px] mx-auto">
        <MessageSkeleton :lines="2" />
      </div>

      <!-- Message queue indicator -->
      <div v-if="hasQueuedMessages" class="max-w-[1800px] mx-auto">
        <QueueIndicator
          :count="queueCount"
          :messages="messageQueue"
          @clear="clearQueue"
          @remove="removeFromQueue"
        />
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
    <div class="p-3 sm:p-4">
      <div class="max-w-[1800px] mx-auto">
        <!-- Queue mode indicator -->
        <div v-if="hasActiveWork && input.trim()" class="mb-2 text-xs text-nanna-primary flex items-center gap-1">
          <svg class="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M13 10V3L4 14h7v7l9-11h-7z" />
          </svg>
          Press Enter to queue this message
        </div>
        <ChatInput
          ref="chatInputRef"
          v-model="input"
          :placeholder="hasActiveWork ? 'Type to queue a message...' : 'Type your message...'"
          :disabled="false"
          :is-active="hasActiveWork"
          @submit="sendMessage"
          @stop="stopSession"
        />
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, nextTick, onMounted, onUnmounted, computed } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, emit as tauriEmit, type UnlistenFn } from '@tauri-apps/api/event'
import { useSessionState } from '~/composables/useSessionState'

// Notifications
const { notifyToolComplete, notifyError, notifyMessage } = useNotifications()
const { addNotification } = useNotificationCenter()

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
  tool_calls?: ToolCallInfo[]
  reasoning?: string
}

interface SessionInfo {
  id: string
  name: string
  created_at: string
  updated_at: string
  message_count: number
  workspace_id?: string
  workspace_name?: string
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

interface RunState {
  is_running: boolean
  accumulated_text: string
  active_tool_calls: { call_id: string, name: string, started_at: string }[]
  completed_tool_calls: { call_id: string, name: string, output: string, success: boolean, duration_ms: number }[]
  started_at: string | null
  message_count: number
  last_message_id?: string | null
  queued_count?: number
}

// Get session ID from URL query param
const route = useRoute()

// Local state for messages (loaded from storage)
const messages = ref<Message[]>([])
const input = ref('')
const chatInputRef = ref<any>(null)
const messagesContainer = ref<HTMLElement | null>(null)
const currentSession = ref<SessionInfo | null>(null)
const config = ref<AppConfig | null>(null)
const connectionError = ref<string | null>(null)
const isRetrying = ref(false)
const lastUserMessage = ref<string>('')

// Session ID as computed ref for the composable
const sessionId = computed(() => currentSession.value?.id ?? null)

// Use session state composable - state survives navigation
const {
  isLoading,
  isStreaming,
  streamingContent,
  streamingThinking,
  activeToolCalls,
  messageQueue,
  hasActiveWork,
  hasQueuedMessages,
  queueCount,
  daemonQueueCount,
  queueMessage,
  dequeueMessage,
  removeFromQueue,
  addToolCall,
  updateToolCall,
  clearStreamingState,
} = useSessionState(sessionId)

function truncateText(text: string, maxLen: number): string {
  if (!text || text.length <= maxLen) return text || ''
  return text.substring(0, maxLen) + '...'
}

let unlistenChunk: UnlistenFn | null = null
let unlistenTool: UnlistenFn | null = null
let unlistenThinking: UnlistenFn | null = null
let unlistenModelStatus: UnlistenFn | null = null
let unlistenDaemonError: UnlistenFn | null = null
let daemonQueuePollTimer: ReturnType<typeof setInterval> | null = null

// Poll daemon run state while session is active to keep queue depth fresh
watch(hasActiveWork, (active) => {
  if (active && !daemonQueuePollTimer) {
    daemonQueuePollTimer = setInterval(async () => {
      if (!currentSession.value) return
      try {
        const runState = await invoke<RunState>('get_session_run_state', {
          sessionId: currentSession.value.id,
        })
        daemonQueueCount.value = runState.queued_count ?? 0
      } catch {
        // Ignore poll errors
      }
    }, 2000)
  } else if (!active && daemonQueuePollTimer) {
    clearInterval(daemonQueuePollTimer)
    daemonQueuePollTimer = null
    daemonQueueCount.value = 0
  }
})

// Process queued messages when current request completes
watch(isLoading, async (loading) => {
  if (!loading && hasQueuedMessages.value) {
    // Small delay before processing next message
    await new Promise(resolve => setTimeout(resolve, 100))
    processNextQueuedMessage()
  }
})

async function processNextQueuedMessage() {
  const next = dequeueMessage()
  if (next && currentSession.value) {
    // Add user message to display
    messages.value.push({
      id: next.id,
      role: 'user',
      content: next.content,
      timestamp: next.timestamp,
    })
    scrollToBottom(true)

    // Send to backend
    await sendMessageToBackend(next.content, [])
  }
}

function clearQueue() {
  if (sessionId.value) {
    const state = useSessionState(sessionId)
    state.messageQueue.value = []
  }
}

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
        const sid = currentSession.value.id

        // Fetch history and run state in parallel.
        // Don't clear streaming state before fetching — listeners may already be
        // receiving events for an active stream (they're set up before loadSession).
        // We'll set authoritative state from the daemon AFTER the fetch.
        const [historyResult, runState] = await Promise.all([
          invoke<Message[]>('get_session_history', { sessionId: sid }),
          invoke<RunState>('get_session_run_state', { sessionId: sid })
            .catch(() => ({ is_running: false, accumulated_text: '', active_tool_calls: [], completed_tool_calls: [], started_at: null, message_count: 0 } as RunState))
        ])

        messages.value = historyResult

        // Sync verification: if daemon reports different count, re-fetch
        if (runState.message_count !== undefined &&
            messages.value.length !== runState.message_count &&
            runState.message_count > messages.value.length) {
          console.warn(`Sync mismatch: local=${messages.value.length} daemon=${runState.message_count}, re-fetching`)
          messages.value = await invoke<Message[]>('get_session_history', { sessionId: sid })
        }

        // Set streaming state from daemon's authoritative run state.
        // This overwrites any partial state listeners may have accumulated during
        // the fetch, ensuring consistency with what the daemon knows.
        // Update daemon queue count from run state
        daemonQueueCount.value = runState.queued_count ?? 0

        if (runState.is_running) {
          isLoading.value = true
          isStreaming.value = runState.accumulated_text.length > 0
          streamingContent.value = runState.accumulated_text

          // Replace tool calls with daemon's authoritative list
          activeToolCalls.value = []

          // Restore completed tool calls (from current run, not yet in history)
          for (const tc of runState.completed_tool_calls) {
            addToolCall({
              id: tc.call_id,
              name: tc.name,
              input: null,
              output: tc.output,
              success: tc.success,
              duration_ms: tc.duration_ms,
              status: tc.success ? 'completed' : 'error',
            })
          }

          // Restore active tool calls
          for (const tc of runState.active_tool_calls) {
            addToolCall({
              id: tc.call_id,
              name: tc.name,
              input: null,
              output: '',
              success: false,
              duration_ms: 0,
              status: 'started',
            })
          }
        } else {
          // Session is not running — clear any stale streaming state.
          // This handles: user navigated away during streaming, stream completed
          // while gone, and the global sessionStates Map still had isStreaming=true.
          clearStreamingState()
        }

        await nextTick()
        scrollToBottom(true)
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

  // Set up event listeners BEFORE loading session data.
  // This ensures stream events aren't lost during the async loadSession() work.
  // loadSession() will restore state from the daemon, overwriting any partial
  // state accumulated by these listeners during the fetch.
  unlistenChunk = await listen<StreamChunk>('stream-chunk', (event) => {
    // Update the correct session's state (may not be current session)
    const eventSessionId = event.payload.session_id
    const sessionState = useSessionState(ref(eventSessionId))

    if (event.payload.done) {
      // Streaming complete - finalize message
      const finalContent = sessionState.streamingContent.value
      const finalToolCalls = [...sessionState.activeToolCalls.value]
      const finalThinking = sessionState.streamingThinking.value || undefined

      // Clear streaming state
      sessionState.clearStreamingState()

      // If this is the current session, add to messages
      if (eventSessionId === currentSession.value?.id) {
        if (finalContent || finalToolCalls.length > 0 || finalThinking) {
          messages.value.push({
            id: Date.now().toString(),
            role: 'assistant',
            content: finalContent || (finalThinking ? '*[thinking only — no visible response]*' : ''),
            timestamp: new Date().toISOString(),
            tool_calls: finalToolCalls.map(t => ({
              id: t.id,
              name: t.name,
              input: t.input,
              output: t.output,
              success: t.success,
              duration_ms: t.duration_ms,
            })),
            reasoning: finalThinking,
          })

          // Notify if window is not focused
          if (document.hidden && finalContent) {
            notifyMessage(finalContent)
          }

          // Auto-name session after first response (2 messages = 1 user + 1 assistant)
          if (messages.value.length === 2 && currentSession.value?.name?.startsWith('Chat ')) {
            autoNameSession(eventSessionId)
          }
        }
        scrollToBottom()
      }
    } else {
      // Append chunk
      sessionState.isStreaming.value = true
      sessionState.streamingContent.value += event.payload.chunk

      if (eventSessionId === currentSession.value?.id) {
        scrollToBottom()
      }
    }
  })

  // Listen for tool call events - also global
  unlistenTool = await listen<ToolCallEvent>('tool-call', (event) => {
    const eventSessionId = event.payload.session_id
    const sessionState = useSessionState(ref(eventSessionId))
    const { tool_call, status } = event.payload

    if (status === 'started') {
      // Add new tool call
      sessionState.addToolCall({ ...tool_call, status: 'started' })
    } else {
      // Update existing tool call
      sessionState.updateToolCall(tool_call.id, { ...tool_call, status })

      // Push tool errors to notification center
      if (status === 'error' || tool_call.success === false) {
        addNotification({
          type: 'error',
          title: `Tool Failed: ${tool_call.name}`,
          summary: truncateText(tool_call.output || 'Unknown error', 120),
          detail: tool_call.output || 'No error details available',
          source: `tool:${tool_call.name}`,
          sessionId: eventSessionId,
          metadata: {
            callId: tool_call.id,
            toolName: tool_call.name,
          },
        })
      }

      // Notify on tool completion if window not focused
      if (document.hidden) {
        notifyToolComplete(tool_call.name, tool_call.success)
      }
    }

    if (eventSessionId === currentSession.value?.id) {
      scrollToBottom()
    }
  })

  // Listen for thinking/reasoning chunks
  unlistenThinking = await listen<{ session_id: string; delta: string }>('thinking-chunk', (event) => {
    const eventSessionId = event.payload.session_id
    const sessionState = useSessionState(ref(eventSessionId))
    sessionState.streamingThinking.value += event.payload.delta
  })

  // Listen for model status changes (fallbacks)
  unlistenModelStatus = await listen<{ active_model: string; fallback_reason?: string }>('model-status', (event) => {
    const { active_model, fallback_reason } = event.payload
    if (fallback_reason) {
      addNotification({
        type: 'warning',
        title: `Model Fallback: ${active_model}`,
        summary: `Switched model due to: ${truncateText(fallback_reason, 100)}`,
        detail: `Fell back to ${active_model}.\n\nPrevious model failed: ${fallback_reason}`,
        source: 'llm:model-routing',
        metadata: { model: active_model, reason: fallback_reason },
      })
    }
  })

  // Listen for daemon-level errors
  unlistenDaemonError = await listen<{ code?: string; message: string }>('error', (event) => {
    addNotification({
      type: 'error',
      title: event.payload.code ? `Error: ${event.payload.code}` : 'Daemon Error',
      summary: truncateText(event.payload.message, 120),
      detail: event.payload.message,
      source: 'daemon',
      metadata: event.payload.code ? { code: event.payload.code } : undefined,
    })
  })

  // Load initial session (listeners are already active to capture any events)
  await loadSession()
})

// Auto-scroll when new content is added to the messages container
let scrollObserver: MutationObserver | null = null

watch(messagesContainer, (el) => {
  if (scrollObserver) scrollObserver.disconnect()
  if (el) {
    scrollObserver = new MutationObserver(() => {
      scrollToBottom()
    })
    scrollObserver.observe(el, { childList: true, subtree: true, characterData: true })
  }
}, { immediate: true })

onUnmounted(() => {
  if (scrollObserver) scrollObserver.disconnect()
  if (unlistenChunk) unlistenChunk()
  if (unlistenTool) unlistenTool()
  if (unlistenThinking) unlistenThinking()
  if (unlistenModelStatus) unlistenModelStatus()
  if (unlistenDaemonError) unlistenDaemonError()
  if (daemonQueuePollTimer) {
    clearInterval(daemonQueuePollTimer)
    daemonQueuePollTimer = null
  }
})

// Watch for session changes in URL
watch(() => route.query.session, async (newSessionId) => {
  if (newSessionId && newSessionId !== currentSession.value?.id) {
    // Clear local display state
    messages.value = []
    connectionError.value = null

    // Immediately switch session ID so the template reads from the NEW session's
    // state in the global Map. Without this, there's an async gap during loadSession()
    // where sessionId still points to the old session, causing the old session's
    // streaming bubble to appear in the new session's view.
    currentSession.value = {
      id: newSessionId as string,
      name: '',
      created_at: '',
      updated_at: '',
      message_count: 0,
    }

    // Load new session (will restore in-flight state from daemon and overwrite placeholder)
    await loadSession()
  }
})

async function stopSession() {
  if (!currentSession.value) return
  try {
    const cancelled = await invoke<boolean>('cancel_session', {
      sessionId: currentSession.value.id,
    })
    if (cancelled) {
      // Append cancellation note to streaming content
      if (streamingContent.value) {
        streamingContent.value += '\n\n[Stopped by user]'
      }
      clearStreamingState()
    }
  } catch (e) {
    console.error('Failed to cancel session:', e)
  }
}

async function sendMessage() {
  if (!input.value.trim() || !currentSession.value) return

  const userMessage = input.value.trim()
  const imageAttachments = chatInputRef.value?.getAttachments?.() || []
  input.value = ''
  lastUserMessage.value = userMessage
  connectionError.value = null

  // If already working, queue the message
  if (hasActiveWork.value) {
    queueMessage(userMessage)
    return
  }

  // Add user message immediately
  messages.value.push({
    id: Date.now().toString(),
    role: 'user',
    content: userMessage,
    timestamp: new Date().toISOString(),
  })

  // Reset scroll state and force scroll to bottom when sending new message
  userScrolledUp.value = false
  await nextTick()
  scrollToBottom(true)

  await sendMessageToBackend(userMessage, imageAttachments)
}

async function sendMessageToBackend(message: string, attachments: Array<{filename: string, content_type: string, data: string}> = []) {
  // Start loading
  isLoading.value = true
  isStreaming.value = false
  streamingContent.value = ''
  activeToolCalls.value = []

  try {
    // Send message and wait for response (streaming happens via events)
    await invoke('send_message', {
      sessionId: currentSession.value!.id,
      message,
      attachments
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

    // Push to notification center (always, not just when hidden)
    addNotification({
      type: 'error',
      title: 'Message Send Failed',
      summary: connectionError.value || errorMsg,
      detail: errorMsg,
      source: 'llm:send',
      sessionId: currentSession.value?.id,
    })

    // Notify on error if window not focused
    if (document.hidden) {
      notifyError('Message Failed', connectionError.value ?? undefined)
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

// Generate a session title from the first user message
function generateSessionTitle(message: string): string {
  const cleaned = message.trim().replace(/\n/g, ' ').replace(/\s+/g, ' ')
  if (cleaned.length <= 50) return cleaned
  const truncated = cleaned.substring(0, 50)
  const lastSpace = truncated.lastIndexOf(' ')
  return lastSpace > 20 ? truncated.substring(0, lastSpace) + '...' : truncated + '...'
}

// Auto-name the session after the first response
async function autoNameSession(sessionId: string) {
  const firstUserMsg = messages.value.find(m => m.role === 'user')
  if (!firstUserMsg) return

  const title = generateSessionTitle(firstUserMsg.content)
  try {
    await invoke('rename_session', { sessionId, name: title })
    // Update local state
    if (currentSession.value && currentSession.value.id === sessionId) {
      currentSession.value = { ...currentSession.value, name: title }
    }
    // Notify sidebar to update
    await tauriEmit('session-renamed', { id: sessionId, name: title })
  } catch (e) {
    console.error('Failed to auto-name session:', e)
  }
}

// Track if user has manually scrolled up
const userScrolledUp = ref(false)

function handleScroll() {
  if (!messagesContainer.value) return
  const { scrollTop, scrollHeight, clientHeight } = messagesContainer.value
  // Consider "at bottom" if within 100px of bottom
  const atBottom = scrollHeight - scrollTop - clientHeight < 100
  userScrolledUp.value = !atBottom
}

function scrollToBottom(force = false) {
  nextTick(() => {
    if (messagesContainer.value && (force || !userScrolledUp.value)) {
      messagesContainer.value.scrollTop = messagesContainer.value.scrollHeight
    }
  })
}
</script>

