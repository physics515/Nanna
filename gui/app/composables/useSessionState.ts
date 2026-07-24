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
  model?: string
  data?: Record<string, any>
  /** Tokens spent by the LLM request that issued this call. */
  tokens?: number
  /** Run-total tokens spent when this call was issued. */
  total_tokens?: number
}

interface QueuedMessage {
  id: string
  content: string
  timestamp: string
}

/**
 * One entry in the run's chronological journal, mirroring the daemon's
 * TimelineItem enum (serde tag "kind"). The live view appends to this as
 * events stream in; on remount it is re-seeded from the daemon's
 * authoritative run state, which is run-scoped and therefore survives the
 * daemon's internal healing restarts.
 */
export interface TimelineEntry {
  kind: 'thinking' | 'text' | 'tool' | 'fault'
  at: string
  // thinking / text / fault
  content?: string
  message?: string
  // tool
  call_id?: string
  name?: string
  input?: any
  output?: string | null
  success?: boolean | null
  duration_ms?: number | null
  /** Tokens spent by the request that issued this call / run total then. */
  tokens?: number | null
  total_tokens?: number | null
}

interface SessionState {
  isLoading: boolean
  isStreaming: boolean
  streamingContent: string
  streamingThinking: string
  activeToolCalls: (ToolCallInfo & { status: 'started' | 'completed' | 'error' })[]
  liveTimeline: TimelineEntry[]
  messageQueue: QueuedMessage[]
  lastError: string | null
  daemonQueueCount: number
  /** Live context usage: last request's prompt tokens / enforced window. */
  contextUsed: number
  contextWindow: number
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
      streamingThinking: '',
      activeToolCalls: [],
      liveTimeline: [],
      messageQueue: [],
      lastError: null,
      daemonQueueCount: 0,
      contextUsed: 0,
      contextWindow: 0,
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

  const streamingThinking = computed({
    get: () => state.value?.streamingThinking ?? '',
    set: (val: string) => {
      if (state.value) state.value.streamingThinking = val
    }
  })

  const activeToolCalls = computed({
    get: () => state.value?.activeToolCalls ?? [],
    set: (val: (ToolCallInfo & { status: 'started' | 'completed' | 'error' })[]) => {
      if (state.value) state.value.activeToolCalls = val
    }
  })

  const liveTimeline = computed({
    get: () => state.value?.liveTimeline ?? [],
    set: (val: TimelineEntry[]) => {
      if (state.value) state.value.liveTimeline = val
    }
  })

  const contextUsed = computed({
    get: () => state.value?.contextUsed ?? 0,
    set: (val: number) => {
      if (state.value) state.value.contextUsed = val
    }
  })

  const contextWindow = computed({
    get: () => state.value?.contextWindow ?? 0,
    set: (val: number) => {
      if (state.value) state.value.contextWindow = val
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

  // Update tool call (only merge defined, non-undefined values to preserve
  // existing fields). Ids are NOT unique across a run — Ollama synthesizes
  // them per response — so prefer the newest still-running entry with this
  // id, falling back to the newest match.
  function updateToolCall(id: string, update: Partial<ToolCallInfo & { status: 'started' | 'completed' | 'error' }>) {
    if (state.value) {
      const calls = state.value.activeToolCalls
      let idx = -1
      for (let i = calls.length - 1; i >= 0; i--) {
        const entry = calls[i]
        if (entry && entry.id === id) {
          if (entry.status === 'started') { idx = i; break }
          if (idx === -1) idx = i
        }
      }
      if (idx !== -1) {
        const existing = calls[idx]
        const filtered: Record<string, any> = {}
        for (const [key, value] of Object.entries(update)) {
          if (value !== undefined) {
            filtered[key] = value
          }
        }
        calls[idx] = { ...existing, ...filtered }
      }
    }
  }

  // --- Live timeline (chronological journal mirror) ---

  /** Append a streamed delta to the open trailing segment, or open a new one. */
  function timelineAppendSegment(kind: 'thinking' | 'text', chunk: string) {
    if (!state.value || !chunk) return
    const items = state.value.liveTimeline
    const last = items[items.length - 1]
    if (last && last.kind === kind) {
      last.content = (last.content ?? '') + chunk
    } else {
      items.push({ kind, content: chunk, at: new Date().toISOString() })
    }
  }

  /** Record a tool call starting. */
  function timelineToolStart(toolCall: ToolCallInfo) {
    if (!state.value) return
    state.value.liveTimeline.push({
      kind: 'tool',
      call_id: toolCall.id,
      name: toolCall.name,
      input: toolCall.input ?? null,
      output: null,
      success: null,
      duration_ms: null,
      tokens: toolCall.tokens ?? null,
      total_tokens: toolCall.total_tokens ?? null,
      at: new Date().toISOString(),
    })
  }

  /** Back-fill a tool call's outcome. Only OPEN items (no output yet)
   *  match — call ids recur across iterations (Ollama synthesizes them per
   *  response), and matching a completed record would overwrite an earlier
   *  call's outcome. With no open match, a fresh item records the outcome
   *  so the call can never vanish from the journal. */
  function timelineToolEnd(id: string, name: string, output: string, success: boolean, durationMs: number) {
    if (!state.value) return
    const items = state.value.liveTimeline
    for (let i = items.length - 1; i >= 0; i--) {
      const item = items[i]
      if (item && item.kind === 'tool' && item.call_id === id && item.output == null) {
        item.output = output
        item.success = success
        item.duration_ms = durationMs
        return
      }
    }
    items.push({
      kind: 'tool',
      call_id: id,
      name,
      input: null,
      output,
      success,
      duration_ms: durationMs,
      tokens: null,
      total_tokens: null,
      at: new Date().toISOString(),
    })
  }

  /** Record a healed provider fault so the journal explains restarts. */
  function timelineFault(message: string) {
    if (!state.value) return
    state.value.liveTimeline.push({ kind: 'fault', message, at: new Date().toISOString() })
  }

  /** Replace the journal wholesale (remount restore from daemon run state). */
  function setLiveTimeline(items: TimelineEntry[]) {
    if (state.value) state.value.liveTimeline = items
  }

  // Clear streaming state (called when stream completes)
  function clearStreamingState() {
    if (state.value) {
      state.value.isStreaming = false
      state.value.streamingContent = ''
      state.value.streamingThinking = ''
      state.value.activeToolCalls = []
      state.value.liveTimeline = []
      state.value.isLoading = false
    }
  }

  // Reset all state for session
  function resetState() {
    if (state.value) {
      state.value.isLoading = false
      state.value.isStreaming = false
      state.value.streamingContent = ''
      state.value.streamingThinking = ''
      state.value.activeToolCalls = []
      state.value.liveTimeline = []
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
    streamingThinking,
    activeToolCalls,
    liveTimeline,
    messageQueue,
    lastError,
    daemonQueueCount,
    contextUsed,
    contextWindow,

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
    timelineAppendSegment,
    timelineToolStart,
    timelineToolEnd,
    timelineFault,
    setLiveTimeline,
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
