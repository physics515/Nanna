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
  /** P22 Tier 4 breaker replay: the harness answered this call itself and the
   *  tool never ran. `success` is false because there is no tool result, but
   *  nothing failed — steering, not an error. Live-stream only; the daemon's
   *  own journal does not carry the marker, so a restored timeline renders
   *  these as plain failures until it does. */
  short_circuited?: boolean | null
}

/**
 * The daemon's last liveness beat for a running turn — what it is waiting on
 * and for how long. Beats arrive at the daemon's derived cadence (~30s), so
 * these figures are as of `beat`, not as of now; nothing here is extrapolated
 * between beats, because a badge inventing elapsed time is the same dishonesty
 * as a badge asserting activity it never observed.
 */
export interface LivenessBeat {
  /** Seconds since the turn started, as of this beat. */
  elapsedS: number
  /** Seconds since the last observed output; `null` = nothing to report yet. */
  quietS: number | null
  /** Coarse phase: planning | step_pending | streaming | thinking | tool. */
  phase: string
  /** The daemon's own sentence for what the turn is waiting on. */
  awaiting: string
  /** Monotone beat counter within the turn — a gap means beats stopped. */
  beat: number
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
  /**
   * The model THIS chat is pinned to (`null` = follow the global `[llm]`
   * default). Seeded from the daemon's `SessionInfo.chat_model` when the
   * session loads; the picker writes it so the header updates on selection
   * instead of on the round-trip. Chat replies only — sub-agent,
   * summarization and embedding models are global and live in Settings.
   */
  chatModel: string | null
  /**
   * Whether `chatModel` above has actually been ASSERTED — seeded from the
   * daemon's `SessionInfo`, or written by the picker — rather than merely
   * defaulted by creating this entry. A stream event for a chat this window
   * never opened creates state for it, and that entry's `null` means "we have
   * not looked", not "no pin"; a list row that trusted it would drop the pin
   * of a chat that has one.
   */
  chatModelKnown: boolean
  /**
   * Whether the pin changed while a turn was already running. A turn resolves
   * its model once, when it is prepared, so a change made mid-turn does NOT
   * re-model the reply on screen — it lands on the next message. Cleared when
   * that turn ends, because from then on the pin is simply what this chat uses.
   */
  chatModelPendingNextTurn: boolean
  /**
   * How many `set_session_model` requests this window still has out for this
   * chat. A session LIST is the daemon's answer as of when it was fetched, so
   * while a pin change is unanswered that list cannot know about it — seeding
   * from it would put the old model back on screen and mark it KNOWN. Zero
   * means nothing of ours is in flight and the daemon's answer is the newer
   * of the two.
   */
  chatModelInFlight: number
  /** Last liveness beat for the turn in flight; `null` = none seen. */
  liveness: LivenessBeat | null
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
      chatModel: null,
      chatModelKnown: false,
      chatModelPendingNextTurn: false,
      chatModelInFlight: 0,
      liveness: null,
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

  // Writing the pin is also what makes it KNOWN: both writers — the seed from
  // the daemon's SessionInfo and the picker — are assertions about this chat.
  const chatModel = computed({
    get: () => state.value?.chatModel ?? null,
    set: (val: string | null) => {
      if (state.value) {
        state.value.chatModel = val
        state.value.chatModelKnown = true
      }
    }
  })

  const chatModelPendingNextTurn = computed({
    get: () => state.value?.chatModelPendingNextTurn ?? false,
    set: (val: boolean) => {
      if (state.value) state.value.chatModelPendingNextTurn = val
    }
  })

  const liveness = computed({
    get: () => state.value?.liveness ?? null,
    set: (val: LivenessBeat | null) => {
      if (state.value) state.value.liveness = val
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
  function timelineToolEnd(
    id: string,
    name: string,
    output: string,
    success: boolean,
    durationMs: number,
    shortCircuited = false,
  ) {
    if (!state.value) return
    const items = state.value.liveTimeline
    for (let i = items.length - 1; i >= 0; i--) {
      const item = items[i]
      if (item && item.kind === 'tool' && item.call_id === id && item.output == null) {
        item.output = output
        item.success = success
        item.duration_ms = durationMs
        item.short_circuited = shortCircuited
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
      short_circuited: shortCircuited,
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
      // The turn the pin could not reach is over, so the pin is no longer
      // waiting on anything — the next message resolves against it.
      state.value.chatModelPendingNextTurn = false
      // Beats belong to the turn that produced them. Keeping the last one
      // would have the badge go on reporting what a finished turn was
      // waiting on.
      state.value.liveness = null
    }
  }

  // Reset all state for session.
  // `chatModel` deliberately survives: it is the chat's durable pin, not run
  // state, and clearing it here would make the header claim the session
  // follows the global default while the daemon still runs it on the pin.
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
      state.value.chatModelPendingNextTurn = false
      state.value.liveness = null
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
    chatModel,
    chatModelPendingNextTurn,
    liveness,

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

/**
 * This window's own answer for a session's pin, or `undefined` when it has
 * none. Session LISTS come from a snapshot the layout reloads only at mount,
 * on `sessions-cleared`, and on a workspace-tab change — so a row that trusted
 * the snapshot alone kept asserting a pin the user had already removed. Where
 * this window knows the pin first-hand, that is the fresher of the two answers
 * and the one a list row should render.
 */
export function knownChatModel(sessionId: string): string | null | undefined {
  const state = sessionStates.get(sessionId)
  return state?.chatModelKnown ? state.chatModel : undefined
}

/**
 * Record the daemon's answer for a session's pin. A reloaded session list is
 * daemon truth about every chat at once, so it re-seeds the store the rows
 * read from; otherwise this window's own older answer would outlive a pin
 * changed in another window, which is the same staleness the other way round.
 */
export function seedChatModel(sessionId: string, model: string | null) {
  // ...EXCEPT while this window has a pin change of its own unanswered. The
  // list was fetched at some point before the daemon replied, so it cannot
  // carry a change the daemon has not confirmed yet; taking it would put the
  // old model back on screen AND mark it known, which is worse than either
  // half. The request's own outcome writes the truth when it lands.
  if (chatModelChangesInFlight(sessionId) > 0) return
  // With no pin and no state for this chat there is nothing to correct — the
  // row already falls back to the list's own value — and creating an entry per
  // listed session would grow the map for chats never opened here.
  if (model === null && !sessionStates.has(sessionId)) return
  const state = getSessionState(sessionId)
  state.chatModel = model
  state.chatModelKnown = true
}

/** How many pin changes this window still has out for `sessionId`. */
export function chatModelChangesInFlight(sessionId: string): number {
  return sessionStates.get(sessionId)?.chatModelInFlight ?? 0
}

/**
 * Mark a `set_session_model` request as out for this chat. Counted rather than
 * flagged because a user can pick twice before the first answer arrives, and
 * the daemon's list only stops being stale once the LAST of them has settled.
 */
export function beginChatModelChange(sessionId: string) {
  getSessionState(sessionId).chatModelInFlight += 1
}

/** Mark one such request as settled — succeeded or refused, both are answers. */
export function endChatModelChange(sessionId: string) {
  const state = sessionStates.get(sessionId)
  if (!state) return
  state.chatModelInFlight = Math.max(0, state.chatModelInFlight - 1)
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
