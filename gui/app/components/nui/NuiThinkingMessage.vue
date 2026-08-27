<script setup lang="ts">
import { computed, ref } from 'vue'
import { stripHarnessMarkers } from '~/lib/harnessMarkers'

const props = defineProps<{
  content: string
  author?: string
  /** True while the burst is still streaming — pulsing dot, live cursor. */
  active?: boolean
  defaultExpanded?: boolean
}>()

const expanded = ref(props.defaultExpanded ?? false)

// Counted over the same text the timeline gates the card on, so the count
// and the gate never disagree. Splitting an empty string yields [''] — that
// is what labelled whitespace-only bursts "1 words" before they were gated.
const meta = computed(() => {
  if (props.active) return ''
  const visible = stripHarnessMarkers(props.content ?? '').trim()
  const words = visible ? visible.split(/\s+/).length : 0
  return words > 0 ? `${words} words` : ''
})
</script>

<template>
  <NuiMessage role="assistant" accent="yellow" :author="props.author">
    <template #header>
      <NuiCollapseTrigger v-model="expanded" label="Thinking" :meta="meta" />
      <span v-if="props.active" class="h-2 w-2 shrink-0 animate-pulse rounded-full bg-nui-yellow" />
    </template>
    <div v-show="expanded" class="flex w-full flex-col items-start">
      <p class="w-full whitespace-pre-wrap break-words text-xs leading-normal text-nui-fg">{{ props.content }}<span v-if="props.active" class="cursor-blink">▋</span></p>
    </div>
  </NuiMessage>
</template>
