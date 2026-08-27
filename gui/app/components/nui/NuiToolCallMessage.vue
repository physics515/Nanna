<script setup lang="ts">
import { ref } from 'vue'

const props = defineProps<{
  name: string
  /** Short summary shown next to the tool name (e.g. the query). */
  description?: string
  input?: string
  output?: string
  author?: string
  /** Cap the output well before it scrolls. */
  outputMaxHeight?: number
}>()

const expanded = ref(true)
</script>

<template>
  <NuiMessage role="assistant" accent="pink" :author="props.author">
    <template #header>
      <NuiCollapseTrigger v-model="expanded" :label="props.name" :meta="props.description" />
    </template>
    <template v-if="expanded">
      <template v-if="props.input !== undefined || $slots.input">
        <div class="flex w-full items-start gap-2">
          <NuiIcon name="input-circle" :size="16" class="text-nui-fg" />
          <p class="min-w-0 flex-1 text-xs leading-normal text-nui-fg">Input</p>
        </div>
        <NuiCodeBlock :code="props.input">
          <slot name="input" />
        </NuiCodeBlock>
      </template>
      <template v-if="props.output !== undefined || $slots.output">
        <div class="flex w-full items-start gap-2">
          <NuiIcon name="output-circle" :size="16" class="text-nui-fg" />
          <p class="min-w-0 flex-1 text-xs leading-normal text-nui-fg">Output</p>
        </div>
        <NuiCodeBlock :code="props.output" :max-height="props.outputMaxHeight ?? 496">
          <slot name="output" />
        </NuiCodeBlock>
      </template>
    </template>
  </NuiMessage>
</template>
