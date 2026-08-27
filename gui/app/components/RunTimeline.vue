<template>
  <div class="flex w-full flex-col gap-4">
    <template v-for="(item, idx) in items" :key="timelineKey(item, idx)">
      <!-- Thinking burst: its own block, inline where it happened. Gated
           exactly like the text branch below — a burst that closed empty
           (a lone newline delta between two tool calls) otherwise drew its
           own "Thinking · 1 words" card. -->
      <NuiThinkingMessage
        v-if="item.kind === 'thinking' && hasRenderableText(item.content)"
        :content="item.content ?? ''"
        :active="isLive && idx === items.length - 1"
      />

      <!-- Tool call, with its wall-clock timestamp and token spend -->
      <NuiToolCallMessage
        v-else-if="item.kind === 'tool'"
        :tool-call="toToolCall(item)"
        :status="toolStatus(item)"
        :timestamp="item.at"
        :tokens="item.tokens ?? undefined"
        :total-tokens="item.total_tokens ?? undefined"
      />

      <!-- Visible assistant text between tool calls. The trailing OPEN text
           segment is skipped while live — the streaming bubble renders it.
           Marker-only and whitespace-only segments are skipped too: models
           emit stray newlines and bare TASK COMPLETE lines between tool
           calls, and each opens its own segment (the previous item is a
           tool), which otherwise draws an empty card. -->
      <NuiMessage
        v-else-if="item.kind === 'text' && !(isLive && idx === items.length - 1) && hasRenderableText(item.content)"
        role="assistant"
      >
        <MarkdownContent :content="stripHarnessMarkers(item.content ?? '')" />
      </NuiMessage>

      <!-- Healed provider fault: a seam in the run. Recorded so restarted
           thinking/text after it reads as "new attempt", not corruption. -->
      <div
        v-else-if="item.kind === 'fault'"
        class="mx-32 flex items-center gap-2 border-l-8 border-solid border-nui-yellow py-1 pl-4 text-xs leading-normal text-nui-yellow"
        :title="item.message"
      >
        <span class="shrink-0">⚡</span>
        <span>stream fault — healed and continued</span>
        <span class="ml-auto shrink-0 text-nui-muted">{{ dayStamp(item.at) }}</span>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import type { TimelineEntry } from '~/composables/useSessionState'
// A card renders only if it has visible content once the harness plumbing is
// stripped — the same invariant the page applies to its historical and
// streaming bubbles.
import { hasRenderableText, stripHarnessMarkers } from '~/lib/harnessMarkers'

const props = defineProps<{
  items: TimelineEntry[]
  /** Live view: the trailing open text segment is rendered by the streaming
   *  bubble instead, and the trailing thinking segment shows as active. */
  isLive?: boolean
}>()

// Index-based keys: the journal is append-only (items never reorder or
// vanish), and call_ids are NOT unique — Ollama synthesizes them per
// response (toolu_00000000, …), so the same id recurs across iterations.
function timelineKey(item: TimelineEntry, idx: number): string {
  return `${idx}-${item.kind}`
}

function toToolCall(item: TimelineEntry) {
  return {
    id: item.call_id ?? '',
    name: item.name ?? 'unknown',
    input: item.input ?? null,
    output: item.output ?? '',
    success: item.success ?? false,
    duration_ms: item.duration_ms ?? 0,
  }
}

function toolStatus(item: TimelineEntry): 'started' | 'completed' | 'error' | 'steering' {
  // Breaker replay: the harness answered the call itself and the tool never
  // ran. `success` is false only because there is no tool result — nothing
  // failed, so it must not wear the error styling.
  if (item.short_circuited === true) return 'steering'
  if (item.success === null || item.success === undefined) {
    // Live: genuinely still running. Finalized history: the run ended
    // without this call completing (stop/crash/fault mid-call) — a
    // forever-spinning chip would be a lie.
    return props.isLive ? 'started' : 'error'
  }
  return item.success ? 'completed' : 'error'
}

function dayStamp(iso: string): string {
  const d = new Date(iso)
  if (isNaN(d.getTime())) return ''
  const weekday = d.toLocaleDateString('en-US', { weekday: 'long' })
  const month = d.toLocaleDateString('en-US', { month: 'long' })
  return `${weekday}, ${month} ${d.getDate()} ${d.getFullYear()}`
}
</script>
