<script setup lang="ts">
import { computed, ref } from 'vue'

const props = defineProps<{
  content: string
  author?: string
}>()

const expanded = ref(true)

const meta = computed(() => {
  const trimmed = props.content.trim()
  const words = trimmed ? trimmed.split(/\s+/).length : 0
  return `${words} words`
})
</script>

<template>
  <NuiMessage role="assistant" accent="yellow" :author="props.author">
    <template #header>
      <NuiCollapseTrigger v-model="expanded" label="Thinking" :meta="meta" />
    </template>
    <div v-show="expanded" class="flex w-full flex-col items-start">
      <p class="w-full whitespace-pre-wrap break-words text-xs leading-normal text-nui-fg">{{ props.content }}</p>
    </div>
  </NuiMessage>
</template>
