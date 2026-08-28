<script setup lang="ts">
import { ref } from 'vue'

/**
 * The dash-bordered code well from the design: optional actions row (format
 * select + copy) above, monospace content inside, accent scrollbar when the
 * content is taller than maxHeight. Pass `code` for plain text or use the
 * default slot for pre-highlighted markup.
 */
const props = defineProps<{
  code?: string
  /** Cap the well's height (px) and scroll the overflow. */
  maxHeight?: number
  /** Offering formats renders the actions row with a format select. */
  formats?: string[]
  copyable?: boolean
}>()

const format = defineModel<string>('format')

const emit = defineEmits<{
  copy: []
}>()

const copied = ref(false)

async function copy() {
  try {
    await navigator.clipboard.writeText(props.code ?? '')
    copied.value = true
    setTimeout(() => { copied.value = false }, 1500)
  } catch (e) {
    console.error('Copy failed:', e)
  }
  emit('copy')
}
</script>

<template>
  <div class="flex w-full flex-col items-end gap-4 px-8 pb-4">
    <div v-if="props.formats?.length || props.copyable" class="flex flex-wrap items-center gap-4">
      <NuiSelect
        v-if="props.formats?.length"
        v-model="format"
        :options="props.formats.map(f => ({ value: f, label: f }))"
        class="w-64"
      />
      <NuiIconButton
        v-if="props.copyable"
        :icon="copied ? 'circle-check' : 'copy'"
        :label="copied ? 'Copied' : 'Copy'"
        @click="copy"
      />
    </div>
    <div
      class="nui-scroll w-full border-b border-t border-dashed border-nui-muted p-2"
      :style="props.maxHeight ? { maxHeight: `${props.maxHeight}px`, overflowY: 'auto' } : undefined"
    >
      <slot>
        <pre class="whitespace-pre-wrap break-words font-nui text-xs font-[450] leading-normal text-nui-fg">{{ props.code }}</pre>
      </slot>
    </div>
  </div>
</template>
