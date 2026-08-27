<script setup lang="ts">
import { computed, ref } from 'vue'

export interface NuiToolCall {
  id: string
  name: string
  input: any
  output: string
  success: boolean
  duration_ms: number
  model?: string
  data?: Record<string, any>
}

const props = withDefaults(defineProps<{
  toolCall: NuiToolCall
  /** `steering` is a breaker replay: the harness answered the call and the
   *  tool never ran. Distinct from `error` on purpose — a wall of replays
   *  reading as failures is what made steering look like breakage. */
  status?: 'started' | 'completed' | 'error' | 'steering'
  /** ISO timestamp of when the call started; shown in the header tooltip. */
  timestamp?: string
  /** Tokens spent by the LLM request that issued this call. */
  tokens?: number
  /** Run-total tokens spent when this call was issued. */
  totalTokens?: number
  author?: string
  defaultExpanded?: boolean
  outputMaxHeight?: number
}>(), {
  status: 'completed',
  outputMaxHeight: 496,
})

const expanded = ref(props.defaultExpanded ?? false)

const inputSummary = computed(() => {
  const input = props.toolCall.input
  if (!input || typeof input !== 'object') return ''
  const path = input.file_path || input.filePath || input.path || input.command || input.query || input.url
  if (path) {
    const s = String(path)
    return s.length > 60 ? '…' + s.slice(-55) : s
  }
  return ''
})

/**
 * The write-content placeholder the daemon substitutes for a write call's
 * bytes, in the two shapes that ASSERT the bytes landed. Neither was gated on
 * the outcome, so a card marked failed showed an Input claiming success
 * beside an Output reading "WRITE HELD". Anchored and whole-string: it may
 * only ever match the placeholder itself.
 */
const LANDED_WRITE_PLACEHOLDER =
  /^\[content omitted from context — (\d+) bytes were written(?: successfully)? to disk\]$/

/**
 * The Input as it can honestly be shown: where the stored placeholder asserts
 * a write this card is simultaneously marking as not-completed, the assertion
 * is replaced by the fact that survives (the byte count) plus a pointer to
 * the Output, which is the authoritative record.
 */
const displayInput = computed(() => {
  const input = props.toolCall.input
  if (props.status === 'completed') return input
  if (!input || typeof input !== 'object' || Array.isArray(input)) return input
  const content = (input as Record<string, unknown>).content
  if (typeof content !== 'string') return input
  const claim = LANDED_WRITE_PLACEHOLDER.exec(content)
  if (!claim) return input
  return {
    ...input,
    content: `[content omitted from context — ${claim[1]} bytes were sent to this tool; `
      + 'the Output below is the authoritative record of what happened on disk]',
  }
})

/** Written content from write_file (data.written). Only on a call that
 * SUCCEEDED: this pane's whole claim is that these bytes are on disk. */
const writtenContent = computed(() => {
  if (props.status !== 'completed') return ''
  return props.toolCall.data?.written ?? ''
})

const statusGlyph = computed(() => {
  switch (props.status) {
    case 'started': return '⟳'
    case 'completed': return '✓'
    case 'steering': return '⇄'
    default: return '✗'
  }
})

const statusClass = {
  'started': 'text-nui-yellow animate-spin',
  'completed': 'text-nui-green',
  'error': 'text-nui-pink',
  'steering': 'text-nui-info',
}

const durationLabel = computed(() => {
  const ms = props.toolCall.duration_ms
  if (!ms) return ''
  if (ms < 1000) return `${ms}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60000).toFixed(1)}m`
})

// "342/48213 tok" — the action's spend over the run's running total.
const tokenStamp = computed(() => {
  if (props.tokens == null || props.totalTokens == null) return ''
  return `${props.tokens}/${props.totalTokens} tok`
})

const preciseTime = computed(() => {
  if (!props.timestamp) return ''
  const d = new Date(props.timestamp)
  if (isNaN(d.getTime())) return ''
  return d.toLocaleString()
})

function formatJson(obj: any): string {
  try { return JSON.stringify(obj, null, 2) } catch { return String(obj) }
}
</script>

<template>
  <NuiMessage role="assistant" accent="pink" :author="props.author">
    <template #header>
      <NuiCollapseTrigger v-model="expanded" :label="props.toolCall.name" :meta="inputSummary" />
      <span class="min-w-0 flex-1" />
      <span v-if="props.status === 'started'" class="h-2 w-2 shrink-0 animate-pulse rounded-full bg-nui-yellow" />
      <span v-if="props.status === 'steering'" class="whitespace-nowrap text-xs text-nui-info">steering</span>
      <span v-if="tokenStamp" class="whitespace-nowrap text-xs text-nui-muted" title="tokens on this action / run total">{{ tokenStamp }}</span>
      <span v-if="durationLabel" class="whitespace-nowrap text-xs text-nui-muted" :title="preciseTime">{{ durationLabel }}</span>
      <span
        class="shrink-0 text-xs"
        :class="statusClass[props.status]"
        :data-status="props.status"
        :title="props.status"
      >{{ statusGlyph }}</span>
    </template>
    <template v-if="expanded">
      <div class="flex w-full items-start gap-2">
        <NuiIcon name="input-circle" :size="16" class="text-nui-fg" />
        <p class="min-w-0 flex-1 text-xs leading-normal text-nui-fg">Input</p>
      </div>
      <NuiCodeBlock :max-height="200">
        <pre class="whitespace-pre-wrap break-words font-nui text-xs font-[450] leading-normal text-nui-fg">{{ formatJson(displayInput) }}</pre>
      </NuiCodeBlock>

      <template v-if="props.toolCall.output || props.status === 'started'">
        <div class="flex w-full items-start gap-2">
          <NuiIcon name="output-circle" :size="16" class="text-nui-fg" />
          <p class="min-w-0 flex-1 text-xs leading-normal text-nui-fg">
            {{ props.status === 'steering' ? 'Steering notice' : 'Output' }}
          </p>
        </div>
        <p v-if="props.status === 'steering'" class="w-full px-8 text-xs leading-normal text-nui-info">
          This call was answered by the harness, not executed — the tool never ran, so
          nothing failed and nothing changed on disk.
        </p>
        <NuiCodeBlock v-if="props.toolCall.output" :max-height="props.outputMaxHeight">
          <pre
            class="whitespace-pre-wrap break-words font-nui text-xs font-[450] leading-normal"
            :class="props.status === 'error' ? 'text-nui-pink' : 'text-nui-fg'"
          >{{ props.toolCall.output }}</pre>
        </NuiCodeBlock>
        <p v-else class="w-full px-8 text-xs italic leading-normal text-nui-muted">Waiting for result...</p>
      </template>

      <template v-if="writtenContent">
        <div class="flex w-full items-start gap-2">
          <NuiIcon name="edit-task" :size="16" class="text-nui-green" />
          <p class="min-w-0 flex-1 text-xs leading-normal text-nui-fg">Written Content</p>
        </div>
        <NuiCodeBlock :max-height="400">
          <pre class="whitespace-pre-wrap break-words font-nui text-xs font-[450] leading-normal text-nui-green">{{ writtenContent }}</pre>
        </NuiCodeBlock>
      </template>
    </template>
  </NuiMessage>
</template>
