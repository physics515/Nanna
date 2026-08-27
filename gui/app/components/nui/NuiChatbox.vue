<script setup lang="ts">
import { ref } from 'vue'

const props = withDefaults(defineProps<{
  placeholder?: string
  disabled?: boolean
  /** Hide the shortcut hints row (e.g. in narrow embeds). */
  hints?: boolean
}>(), {
  placeholder: 'Type your message...',
  hints: true,
})

const text = defineModel<string>({ default: '' })

const emit = defineEmits<{
  send: []
  attach: []
  /** '/' typed at the start of an empty box — hook for a slash-command menu. */
  slash: []
}>()

const textareaEl = ref<HTMLTextAreaElement | null>(null)

function onKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault()
    emit('send')
  } else if (e.key === '/' && text.value === '') {
    emit('slash')
  }
}

function focus() {
  textareaEl.value?.focus()
}

defineExpose({ focus })
</script>

<template>
  <div class="flex h-64 w-full items-start gap-2.5">
    <div class="flex h-full min-w-0 flex-1 flex-col border-b-[32px] border-solid border-nui-accent p-4">
      <textarea
        ref="textareaEl"
        v-model="text"
        class="min-h-0 w-full flex-1 resize-none bg-transparent font-nui text-xs font-[450] leading-normal text-nui-fg outline-none placeholder:text-nui-fg/70"
        :placeholder="props.placeholder"
        :disabled="props.disabled"
        @keydown="onKeydown"
      />
      <div v-if="props.hints" class="flex w-full items-center gap-4 pt-2.5">
        <div class="flex items-center">
          <NuiKbd keys="Ctrl+Enter" />
          <span class="whitespace-nowrap text-xs leading-normal text-nui-muted">&nbsp;to send</span>
        </div>
        <NuiIcon name="dot" :size="16" class="text-nui-muted" />
        <div class="flex items-center">
          <NuiKbd keys="/" />
          <span class="whitespace-nowrap text-xs leading-normal text-nui-muted">&nbsp;commands</span>
        </div>
      </div>
    </div>
    <div class="flex h-full w-12 shrink-0 flex-col gap-1">
      <NuiIconButton icon="send" label="Send" :size="32" class="w-full text-nui-pink hover:text-nui-pink/80" :disabled="props.disabled" @click="emit('send')" />
      <NuiIconButton icon="attach" label="Attach" :size="32" class="w-full" :disabled="props.disabled" @click="emit('attach')" />
    </div>
  </div>
</template>
