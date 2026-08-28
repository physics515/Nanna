<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { modelDisplayName } from '~/lib/modelSpecs'
import { useSessionState } from '~/composables/useSessionState'
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
 * The change still out for a chat. `ticket` is the newest one, and only the
 * newest may write the pill: a slow refusal that revert-wrote unconditionally
 * would undo a NEWER pick that had already succeeded, leaving the header
 * naming a model the chat is not using. `revertTo` is what the daemon last
 * agreed to, captured before the first request went out — with a second pick
 * made while the first is still in flight, the value on screen is itself a
 * guess, and reverting to it would name a model no request ever landed.
 *
 * Keyed by session because the picker outlives the chat it is showing: the
 * user can switch chats mid-request, and each chat's pin is its own.
 */
interface PendingPin {
  ticket: number
  revertTo: string | null
}
const pending = new Map<string, PendingPin>()
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
  const outstanding = pending.get(target)
  const revertTo = outstanding ? outstanding.revertTo : pin.value
  pending.set(target, { ticket, revertTo })

  // Optimistic: the pill answers the click, and the daemon's rejection is what
  // puts the old value back — a picker that waits for the round-trip reads as
  // broken on a busy daemon.
  pin.value = next
  // A turn already running resolved its model before this pick existed, so the
  // pill has to admit the change lands on the next message instead.
  state.chatModelPendingNextTurn.value = state.hasActiveWork.value
  try {
    await invoke('set_session_model', { sessionId: target, model: next })
  } catch (e) {
    // Superseded: a newer pick owns the pill now, and its own outcome is what
    // gets to write there. Reverting on top of it is the clobber this guards.
    if (pending.get(target)?.ticket !== ticket) return
    pin.value = revertTo
    // Back on the value the running turn resolved with, so nothing is waiting.
    state.chatModelPendingNextTurn.value = false
    toast.error('Could not change this chat\'s model', e instanceof Error ? e.message : String(e))
  } finally {
    // Only the newest request clears the slot; an older one settling later
    // finds a ticket that is not its own and leaves the newer state alone.
    if (pending.get(target)?.ticket === ticket) pending.delete(target)
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

/** What the pill's trigger reads: the pin's short name, or the default. */
const selectLabel = computed(() =>
  chatModel.value ? modelDisplayName(chatModel.value) : 'Default',
)

/**
 * The model this chat's next reply actually resolves against: the pin, or
 * the head of the priority list. Shown as the full spec — which provider
 * serves the reply is exactly what the tag is for.
 */
const effectiveModel = computed(() => chatModel.value ?? globalDefault.value)
</script>

<template>
  <div
    class="chat-model-pill flex min-w-0 shrink items-center gap-4"
    :title="pillTitle"
  >
    <!-- Pin picker: the nui purple pill wrapping the native select.
         Amber while a mid-turn change waits for the running reply to end. -->
    <div
      class="relative flex h-10 w-64 shrink-0 items-center gap-2.5 overflow-clip rounded-lg px-4"
      :class="isPendingNextTurn ? 'bg-nui-yellow' : 'bg-nui-accent'"
    >
      <span v-if="isPinned" class="h-2 w-2 shrink-0 rounded-full" :class="isPendingNextTurn ? 'bg-nui-bg' : 'bg-nui-fg'" aria-hidden="true" />
      <p class="min-w-0 flex-1 truncate text-xs leading-normal" :class="isPendingNextTurn ? 'text-nui-bg' : 'text-nui-fg'">
        {{ selectLabel }}
      </p>
      <!-- Reads as text, not decoration: the reply on screen came from the model
           this chat had when the turn started, and the pick applies after it. -->
      <span v-if="isPendingNextTurn" class="chat-model-pill__pending whitespace-nowrap text-xs font-semibold text-nui-bg">next turn</span>
      <NuiIcon name="chevron-down" :size="16" :class="isPendingNextTurn ? 'text-nui-bg' : 'text-nui-fg'" />
      <select
        class="absolute inset-0 h-full w-full cursor-pointer opacity-0"
        aria-label="Model for this chat"
        :value="selectValue"
        @change="onSelect"
      >
        <option :value="FOLLOW_GLOBAL">Default (global)</option>
        <option v-for="option in options" :key="option.spec" :value="option.spec">
          {{ option.label }}
        </option>
      </select>
    </div>

    <!-- The model the next reply actually resolves against (pin or global head). -->
    <NuiTag
      v-if="effectiveModel"
      :label="effectiveModel"
      color="info"
      class="min-w-0 shrink"
    />
  </div>
</template>

