<template>
  <span
    v-if="hasActivity"
    class="flex items-center gap-1"
    :class="compact ? '' : 'text-xs text-nanna-accent'"
    :title="beatTitle"
  >
    <span class="animate-pulse text-nanna-accent">●</span>
    <span v-if="!compact">{{ statusText }}</span>
  </span>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useSessionState } from '~/composables/useSessionState'

const props = withDefaults(defineProps<{
  sessionId: string
  compact?: boolean
}>(), {
  compact: false,
})

const sessionIdRef = computed(() => props.sessionId)

const {
  isLoading,
  isStreaming,
  activeToolCalls,
  hasActiveWork,
  liveness,
} = useSessionState(sessionIdRef)

const hasActivity = computed(() => hasActiveWork.value)

/**
 * What the badge would say from its own local flags alone. `isStreaming`
 * latches on the first text chunk and clears only when the whole run ends, so
 * on its own this label goes on claiming "Streaming..." through an arbitrarily
 * long silence — a wedged turn and a working one read identically.
 */
const activityLabel = computed(() => {
  // Check for active tool calls first
  const runningTools = (activeToolCalls.value ?? []).filter(t => t.status === 'started')
  if (runningTools.length > 0) {
    return `Running ${runningTools[0].name}...`
  }

  if (isStreaming.value) {
    return 'Streaming...'
  }

  if (isLoading.value) {
    return 'Thinking...'
  }

  return 'Working...'
})

/**
 * What the daemon last said the turn was waiting on. This is the only OBSERVED
 * fact on the badge — the labels above are inferences from latched flags — so
 * it qualifies every branch, not just the idle one.
 *
 * The daemon's own sentence is preferred verbatim; it already names the
 * provider and the silence. Falling back to the quiet time keeps the badge
 * honest on a beat that reports one without prose.
 */
const beatNote = computed(() => {
  const beat = liveness.value
  if (!beat) return ''
  if (beat.awaiting) return beat.awaiting
  if (beat.quietS !== null) return `${beat.quietS}s since last output`
  return ''
})

const statusText = computed(() =>
  beatNote.value ? `${activityLabel.value} — ${beatNote.value}` : activityLabel.value
)

/**
 * The hover text carries the beat's full context, including for the compact
 * dot in the session list — which has no room for words but is exactly where
 * a user looks to ask whether a background chat is still alive.
 *
 * The figures are stated as of the beat rather than as of now: they are only
 * refreshed when the next beat lands, and ageing them client-side would be the
 * badge asserting time it did not observe.
 */
const beatTitle = computed(() => {
  const beat = liveness.value
  if (!beat) return activityLabel.value
  const quiet = beat.quietS !== null ? `, quiet ${beat.quietS}s` : ''
  return `${activityLabel.value}\n${beat.awaiting || beat.phase}\n`
    + `As of liveness beat ${beat.beat}: ${beat.phase}, elapsed ${beat.elapsedS}s${quiet}`
})
</script>
