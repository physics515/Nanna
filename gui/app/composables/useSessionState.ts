/**
 * Session state management that survives navigation
 *
 * Tracks per-session:
 * - Streaming state (isStreaming, streamingContent)
 * - Active tool calls
 * - Message queue
 * - Loading state
 */

import { ref, reactive, computed, type Ref } from 'vue'

interface ToolCallInfo {
  id: string
  name: string
  input: any
  output: string
  success: boolean
  duration_ms: number
  status?: 'started' | 'completed' | 'error'
}

interface QueuedMessage {
  id: string
  content: string
  timestamp: string
}

interface SessionState {
  isLoading: boolean
  isStreaming: boolean
  streamingContent: string
  activeToolCalls: (ToolCallInfo & { status: 'started' | 'completed' | 'error' })[]
  messageQueue: QueuedMessage[]
  lastError: string | null
  daemonQueueCount: number
}

// Global state store - persists across component lifecycle
const sessionStates = reactive<Map<string, SessionState>>(new Map())

// Get or create state for a session
function getSessionState(sessionId: string): SessionState {
  if (!sessionStates.has(sessionId)) {
    sessionStates.set(sessionId, {
      isLoading: false,
      isStreaming: false,
      streamingContent: '',
      activeToolCalls: [],
      messageQueue: [],
      lastError: null,
      daemonQueueCount: 0,
    })
  }
  return sessionStates.get(sessionId)!
}

export function useSessionState(sessionId: Ref<string | null>) {
  // Computed refs that auto-switch based on current session
  const state = computed(() => {
    if (!sessionId.value) return null
    return getSessionState(sessionId.value)
  })

  const isLoading = computed({
    get: () => state.value?.isLoading ?? false,
    set: (val: boolean) => {
      if (state.value) state.value.isLoading = val
    }
  })

  const isStreaming = computed({
    get: () => state.value?.isStreaming ?? false,
    set: (val: boolean) => {
      if (state.value) state.value.isStreaming = val
    }
  })

  const streamingContent = computed({
    get: () => state.value?.streamingContent ?? '',
    set: (val: string) => {
      if (state.value) state.value.streamingContent = val
    }
  })

  const activeToolCalls = computed({
    get: () => state.value?.activeToolCalls ?? [],
    set: (val: (ToolCallInfo & { status: 'started' | 'completed' | 'error' })[]) => {
      if (state.value) state.value.activeToolCalls = val
    }
  })

  const messageQueue = computed({
    get: () => state.value?.messageQueue ?? [],
    set: (val: QueuedMessage[]) => {
      if (state.value) state.value.messageQueue = val
    }
  })

  const lastError = computed({
    get: () => state.value?.lastError ?? null,
    set: (val: string | null) => {
      if (state.value) state.value.lastError = val
    }
  })

  const daemonQueueCount = computed({
    get: () => state.value?.daemonQueueCount ?? 0,
    set: (val: number) => {
      if (state.value) state.value.daemonQueueCount = val
    }
  })

  // Queue a message
  function queueMessage(content: string): string {
    const id = `queue-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`
    if (state.value) {
      state.value.messageQueue.push({
        id,
        content,
        timestamp: new Date().toISOString(),
      })
    }
    return id
  }

  // Dequeue next message
  function dequeueMessage(): QueuedMessage | null {
    if (state.value && state.value.messageQueue.length > 0) {
      return state.value.messageQueue.shift()!
    }
    return null
  }

  // Remove message from queue
  function removeFromQueue(id: string) {
    if (state.value) {
      state.value.messageQueue = state.value.messageQueue.filter(m => m.id !== id)
    }
  }

  // Add tool call
  function addToolCall(toolCall: ToolCallInfo & { status: 'started' | 'completed' | 'error' }) {
    if (state.value) {
      state.value.activeToolCalls.push(toolCall)
    }
  }

  // Update tool call
  function updateToolCall(id: string, update: Partial<ToolCallInfo & { status: 'started' | 'completed' | 'error' }>) {
    if (state.value) {
      const idx = state.value.activeToolCalls.findIndex(t => t.id === id)
      if (idx !== -1) {
        state.value.activeToolCalls[idx] = { ...state.value.activeToolCalls[idx], ...update }
      }
    }
  }

  // Clear streaming state (called when stream completes)
  function clearStreamingState() {
    if (state.value) {
      state.value.isStreaming = false
      state.value.streamingContent = ''
      state.value.activeToolCalls = []
      state.value.isLoading = false
    }
  }

  // Reset all state for session
  function resetState() {
    if (state.value) {
      state.value.isLoading = false
      state.value.isStreaming = false
      state.value.streamingContent = ''
      state.value.activeToolCalls = []
      state.value.messageQueue = []
      state.value.lastError = null
      state.value.daemonQueueCount = 0
    }
  }

  // Check if session has active work
  const hasActiveWork = computed(() => {
    return isLoading.value || isStreaming.value || activeToolCalls.value.length > 0
  })

  // Check if there are queued messages
  const hasQueuedMessages = computed(() => {
    return messageQueue.value.length > 0
  })

  // Get queue count
  const queueCount = computed(() => messageQueue.value.length)

  return {
    // State
    isLoading,
    isStreaming,
    streamingContent,
    activeToolCalls,
    messageQueue,
    lastError,
    daemonQueueCount,

    // Computed
    hasActiveWork,
    hasQueuedMessages,
    queueCount,

    // Methods
    queueMessage,
    dequeueMessage,
    removeFromQueue,
    addToolCall,
    updateToolCall,
    clearStreamingState,
    resetState,
  }
}

// Export for checking state from outside (e.g., layout)
export function getSessionStateMap() {
  return sessionStates
}

// Check if any session has active work
export function hasAnyActiveWork(): boolean {
  for (const state of sessionStates.values()) {
    if (state.isLoading || state.isStreaming || state.activeToolCalls.length > 0) {
      return true
    }
  }
  return false
}
