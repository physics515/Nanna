<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { ChevronDown } from 'lucide-vue-next'
import { modelDisplayName } from '~/lib/modelSpecs'
import {
  beginChatModelChange,
  endChatModelChange,
  useSessionState,
} from '~/composables/useSessionState'
import { useToast } from '~/composables/useToast'

const props = defineProps<{ sessionId: string }>()

const sessionIdRef = computed(() => props.sessionId)
const { chatModel, chatModelPendingNextTurn, hasActiveWork } = useSessionState(sessionIdRef)
const toast = useToast()

/** Sentinel for "no pin" — a `<select>` carries strings, and null is not one. */
const FOLLOW_GLOBAL = ''

/**
 * The models this chat may be pinned to: exactly the chat-model priority list
 * from Settings. That list is what the user has already vetted for chat and
 * what the daemon's router is configured to serve, so the picker cannot offer
 * a model whose only outcome is the daemon refusing the turn. Anything else is
 * added in Settings → Chat Models first.
 */
const priority = ref<string[]>([])
let unlistenConfig: UnlistenFn | null = null

async function loadPriority() {
  try {
    priority.value = await invoke<string[]>('get_chat_model_priority')
  } catch (e) {
    console.error('Failed to load chat models:', e)
  }
}

/**
 * A pin outlives the list it was picked from — dropping a model from Settings
 * does not unpin the chats already running on it. Carrying the pin as its own
 * option is what keeps the pill naming the model this chat actually uses
 * instead of rendering blank.
 */
const options = computed(() => {
  const specs = [...priority.value]
  const pinned = chatModel.value
  if (pinned && !specs.includes(pinned)) specs.unshift(pinned)
  return specs.map(spec => ({ spec, label: modelDisplayName(spec) }))
})

const isPinned = computed(() => chatModel.value !== null)
const selectValue = computed(() => chatModel.value ?? FOLLOW_GLOBAL)

/**
 * A turn resolves its model once, when it is prepared, so a pin changed while
 * a reply is already running does not reach that reply — it lands on the next
 * message. The pill says so for as long as that turn lasts; without it the
 * header names a model the answer on screen was demonstrably not produced by.
 */
const isPendingNextTurn = computed(() => chatModelPendingNextTurn.value && hasActiveWork.value)

/** The head of the priority list is what `[llm]` falls back to for every chat. */
const globalDefault = computed(() => priority.value[0] ?? null)

/** Appended to every title while a change is waiting out the running turn. */
const pendingNote = computed(() =>
  isPendingNextTurn.value
    ? '\nTakes effect on your next message — the reply in progress resolved its model when it started.'
    : ''
)

const pillTitle = computed(() => {
  if (chatModel.value) {
    const global = globalDefault.value ? `; the global default is ${globalDefault.value}` : ''
    return `This chat is pinned to ${chatModel.value}${global}.\nChat replies only — sub-agent, summarization and embedding models stay in Settings.${pendingNote.value}`
  }
  if (priority.value.length === 0) {
    return `This chat follows the global default.\nAdd chat models in Settings → Chat Models to pin this chat to one.${pendingNote.value}`
  }
  const global = globalDefault.value ? ` (${globalDefault.value})` : ''
  return `This chat follows the global default${global}.\nPick a model to pin this chat to it — chat replies only, sub-agent and summarization models stay in Settings.${pendingNote.value}`
})

/**
 * A burst of pin changes for one chat — every request the user set off before
 * the previous ones were answered.
 *
 * The daemon is the authority on what this chat is pinned to, and its state is
 * derivable: requests reach it in the order they were sent, a refusal changes
 * nothing there, so after a burst the daemon holds the value of the
 * HIGHEST-numbered request that succeeded, or the pre-burst pin if none did.
 * `agreed`/`agreedTicket` track exactly that, and the pill lands on it once
 * the burst is over. Deciding from the last request to SETTLE instead is what
 * went wrong: a refusal is a non-event at the daemon, but it was allowed to
 * revert the pill past a success that had already been accepted — the header
 * then named a model the chat was demonstrably not using.
 *
 * `outstanding` is what says the burst is over; `newest` is what says whose
 * refusal the user is owed a toast for (a pick they have already replaced is
 * not news). `baseline` is the pin the running turn resolved with, so a burst
 * that ends where it started leaves nothing waiting on that turn.
 *
 * Keyed by session because the picker outlives the chat it is showing: the
 * user can switch chats mid-request, and each chat's pin is its own.
 */
interface PinBurst {
  newest: number
  outstanding: number
  agreed: string | null
  agreedTicket: number
  baseline: string | null
}
const bursts = new Map<string, PinBurst>()
let nextTicket = 0

async function onSelect(event: Event) {
  const picked = (event.target as HTMLSelectElement).value
  const next = picked === FOLLOW_GLOBAL ? null : picked
  if (next === chatModel.value) return

  // Everything below names the session that was on screen when the user picked,
  // not whichever chat is open when the daemon answers — a revert that landed
  // on the wrong session would silently re-model someone else's chat.
  const target = props.sessionId
  const state = useSessionState(ref(target))
  const pin = state.chatModel

  const ticket = ++nextTicket
  const open = bursts.get(target)
  if (open) {
    open.newest = ticket
    open.outstanding += 1
  } else {
    // Ticket 0 for the pre-burst value: any real request outranks it.
    bursts.set(target, {
      newest: ticket,
      outstanding: 1,
      agreed: pin.value,
      agreedTicket: 0,
      baseline: pin.value,
    })
  }

  // While this is unanswered, a reloaded session list is older news than the
  // click — see `seedChatModel`, which stands off rather than seeding the pin
  // back to the model the daemon has not been asked about yet.
  beginChatModelChange(target)

  // Optimistic: the pill answers the click, and the daemon's rejection is what
  // puts the old value back — a picker that waits for the round-trip reads as
  // broken on a busy daemon.
  pin.value = next
  // A turn already running resolved its model before this pick existed, so the
  // pill has to admit the change lands on the next message instead.
  state.chatModelPendingNextTurn.value = state.hasActiveWork.value

  let accepted = false
  let refusal: unknown = null
  try {
    await invoke('set_session_model', { sessionId: target, model: next })
    accepted = true
  } catch (e) {
    refusal = e
  }

  const burst = bursts.get(target)
  endChatModelChange(target)
  // The burst can only be missing if something cleared the map underneath us;
  // there is then no record to decide from, so leave the pill alone.
  if (!burst) return

  // An acceptance moves what the daemon agrees to — that is the half the old
  // guard never recorded. Later requests outrank earlier ones because the
  // daemon applied them in that order.
  if (accepted && ticket > burst.agreedTicket) {
    burst.agreed = next
    burst.agreedTicket = ticket
  }
  // Only the user's latest intent is worth interrupting them about; a refusal
  // of a pick they have already replaced is not news.
  const worthTelling = !accepted && ticket === burst.newest

  burst.outstanding -= 1
  if (burst.outstanding === 0) {
    bursts.delete(target)
    // Nothing is guessed any more: the pill takes the daemon's own state.
    // A burst that was fully accepted lands on what the user last picked, so
    // this is a no-op there and a correction everywhere else.
    pin.value = burst.agreed
    // Ending where it started means the running turn's model is still this
    // chat's model, so nothing is waiting on that turn.
    if (burst.agreed === burst.baseline) state.chatModelPendingNextTurn.value = false
  }

  if (worthTelling) {
    toast.error(
      'Could not change this chat\'s model',
      refusal instanceof Error ? refusal.message : String(refusal),
    )
  }
}

onMounted(async () => {
  await loadPriority()
  // The list is config, and config is shared: the settings page can add or drop
  // a chat model while this header is open. Payload-free event → re-fetch,
  // exactly like ModelStatusBadge.
  unlistenConfig = await listen('config-changed', () => {
    void loadPriority()
  })
})

onUnmounted(() => {
  if (unlistenConfig) unlistenConfig()
})
</script>

<template>
  <div
    class="chat-model-pill"
    :class="{ 'chat-model-pill--pinned': isPinned, 'chat-model-pill--pending': isPendingNextTurn }"
    :title="pillTitle"
  >
    <span v-if="isPinned" class="chat-model-pill__dot" aria-hidden="true" />
    <select
      class="chat-model-pill__select"
      aria-label="Model for this chat"
      :value="selectValue"
      @change="onSelect"
    >
      <option :value="FOLLOW_GLOBAL">Default (global)</option>
      <option v-for="option in options" :key="option.spec" :value="option.spec">
        {{ option.label }}
      </option>
    </select>
    <!-- Reads as text, not decoration: the reply on screen came from the model
         this chat had when the turn started, and the pick applies after it. -->
    <span v-if="isPendingNextTurn" class="chat-model-pill__pending">next turn</span>
    <ChevronDown class="chat-model-pill__chevron" aria-hidden="true" />
  </div>
</template>

<style scoped>
/* Sits beside ModelStatusBadge, so it wears the same glass slab. */
.chat-model-pill {
  position: relative;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border-radius: 9999px;
  padding: 4px 8px 4px 10px;
  font-size: 12px;
  font-weight: 500;
  background: rgba(30, 41, 59, 0.30);
  border-top: 1px solid rgba(255, 255, 255, 0.06);
  border-left: 1px solid rgba(255, 255, 255, 0.04);
  border-bottom: 1.5px solid rgba(71, 85, 105, 0.18);
  border-right: 1px solid rgba(71, 85, 105, 0.10);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.04),
    0 1.5px 1px -0.5px rgba(0, 0, 0, 0.18),
    0 3px 8px -3px rgba(0, 0, 0, 0.12);
  transition: box-shadow 0.2s ease, border-color 0.2s ease;
}
.chat-model-pill:hover {
  border-top-color: rgba(255, 255, 255, 0.10);
  border-left-color: rgba(255, 255, 255, 0.07);
  box-shadow:
    inset 0 1px 0 0 rgba(255, 255, 255, 0.06),
    0 2px 4px -1px rgba(0, 0, 0, 0.20),
    0 4px 12px -4px rgba(0, 0, 0, 0.15);
}

/* A pinned chat is the exception, and reads as one. */
.chat-model-pill--pinned {
  border-color: rgba(139, 92, 246, 0.30);
  background: rgba(139, 92, 246, 0.12);
}

.chat-model-pill__dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #a78bfa;
  flex-shrink: 0;
}

.chat-model-pill__select {
  appearance: none;
  -webkit-appearance: none;
  border: none;
  outline: none;
  background: transparent;
  color: rgba(255, 255, 255, 0.95);
  font: inherit;
  cursor: pointer;
  padding: 0 14px 0 0;
  max-width: 140px;
  text-overflow: ellipsis;
}
.chat-model-pill--pinned .chat-model-pill__select {
  color: #ddd6fe;
}
.chat-model-pill__select:focus-visible {
  outline: 1px solid rgba(139, 92, 246, 0.6);
  outline-offset: 2px;
  border-radius: 4px;
}
/* The dropdown itself is drawn by the OS, which does not inherit the pill. */
.chat-model-pill__select option {
  background: #0f172a;
  color: #e2e8f0;
}

/* A change that has not reached the running turn yet. Amber, not violet: it
   is the same "this is not what you are looking at" note the fallback badge
   wears, and the pill must not read as settled while it isn't. */
.chat-model-pill--pending {
  border-color: rgba(251, 191, 36, 0.30);
  background: rgba(251, 191, 36, 0.10);
  /* Room for the tag AND the absolutely-positioned chevron beside it. */
  padding-right: 20px;
}
.chat-model-pill--pending .chat-model-pill__dot {
  background: #fbbf24;
}
.chat-model-pill--pending .chat-model-pill__select {
  color: #fde68a;
  padding-right: 0;
}

.chat-model-pill__pending {
  flex-shrink: 0;
  font-size: 10px;
  font-weight: 600;
  letter-spacing: 0.02em;
  text-transform: uppercase;
  color: #fbbf24;
  white-space: nowrap;
}

.chat-model-pill__chevron {
  position: absolute;
  right: 6px;
  width: 10px;
  height: 10px;
  pointer-events: none;
  color: rgba(148, 163, 184, 0.7);
}
</style>
