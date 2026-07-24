<template>
  <div class="space-y-1">
    <template v-for="(item, idx) in items" :key="timelineKey(item, idx)">
      <!-- Thinking burst: its own block, inline where it happened -->
      <ThinkingCard
        v-if="item.kind === 'thinking'"
        :content="item.content ?? ''"
        :is-active="isLive && idx === items.length - 1"
      />

      <!-- Tool call, with its wall-clock timestamp and token spend -->
      <ToolCallCard
        v-else-if="item.kind === 'tool'"
        :tool-call="toToolCall(item)"
        :status="toolStatus(item)"
        :timestamp="item.at"
        :tokens="item.tokens ?? undefined"
        :total-tokens="item.total_tokens ?? undefined"
      />

      <!-- Visible assistant text between tool calls. The trailing OPEN text
           segment is skipped while live — the streaming bubble renders it. -->
      <MessageBubble
        v-else-if="item.kind === 'text' && !(isLive && idx === items.length - 1)"
        variant="assistant"
      >
        <div class="flex items-start gap-2 sm:gap-3">
          <UiAvatar variant="accent" fallback="☽" size="sm" class="flex-shrink-0 sm:hidden" />
          <UiAvatar variant="accent" fallback="☽" class="flex-shrink-0 hidden sm:flex" />
          <div class="flex-1 min-w-0">
            <div class="text-xs text-nanna-text-dim mb-1">☽ Nanna</div>
            <MarkdownContent :content="item.content ?? ''" />
          </div>
        </div>
      </MessageBubble>

      <!-- Healed provider fault: a seam in the run. Recorded so restarted
           thinking/text after it reads as "new attempt", not corruption. -->
      <div
        v-else-if="item.kind === 'fault'"
        class="fault-marker"
        :title="item.message"
      >
        <span class="fault-icon">⚡</span>
        <span class="fault-text">stream fault — healed and continued</span>
        <span class="fault-time">{{ dayStamp(item.at) }}</span>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import type { TimelineEntry } from '~/composables/useSessionState'

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

function toolStatus(item: TimelineEntry): 'started' | 'completed' | 'error' {
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

<style scoped>
.fault-marker {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 3px 10px;
  font-size: 11px;
  color: rgba(251, 191, 36, 0.7);
  background: rgba(251, 191, 36, 0.05);
  border-left: 2px solid rgba(251, 191, 36, 0.3);
  border-radius: 0.25rem;
}
.fault-icon {
  flex-shrink: 0;
}
.fault-text {
  font-family: var(--font-mono, monospace);
}
.fault-time {
  margin-left: auto;
  font-size: 10px;
  color: rgba(148, 163, 184, 0.5);
  flex-shrink: 0;
}
</style>
