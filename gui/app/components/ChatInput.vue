<template>
  <div class="chat-input-container w-full">
    <!-- Preview pane (Ctrl+P) -->
    <Transition name="preview">
      <div
        v-if="showPreview && modelValue.trim()"
        class="mb-2 border-b border-t border-dashed border-nui-muted"
      >
        <div class="flex items-center justify-between px-3 py-1.5">
          <span class="text-xs font-semibold leading-normal text-nui-muted">Preview</span>
          <button class="p-1 text-nui-muted hover:text-nui-fg" aria-label="Close preview" @click="showPreview = false">
            <NuiIcon name="close" :size="16" />
          </button>
        </div>
        <div class="max-h-[200px] overflow-y-auto p-3 nui-scroll">
          <MarkdownContent :content="modelValue" />
        </div>
      </div>
    </Transition>

    <!-- Chatbox: editor column + purple base bar, action buttons beside it -->
    <div class="flex h-64 w-full items-start gap-2.5">
      <div
        class="flex h-full min-w-0 flex-1 flex-col border-b-[32px] border-solid p-4 transition-colors"
        :class="isFocused ? 'border-nui-accent' : 'border-nui-accent/80'"
      >
        <!-- Attachment previews -->
        <div v-if="pendingAttachments.length > 0" class="flex gap-2 overflow-x-auto pb-2">
          <div v-for="att in pendingAttachments" :key="att.id" class="relative h-16 w-16 shrink-0 overflow-hidden border border-solid border-nui-accent/60">
            <img :src="att.preview" :alt="att.filename" class="h-full w-full object-cover" />
            <button
              class="absolute right-0.5 top-0.5 flex h-5 w-5 items-center justify-center rounded-full bg-nui-bg/80 text-nui-pink hover:bg-nui-pink hover:text-nui-bg"
              :aria-label="`Remove ${att.filename}`"
              @click="removeAttachment(att.id)"
            >
              <NuiIcon name="close" :size="12" />
            </button>
          </div>
        </div>

        <!-- Rich text editor (extracted Tiptap core) -->
        <RichTextEditor
          ref="richEditorRef"
          :model-value="modelValue"
          :editable="!disabled"
          :placeholder="placeholder || 'Type your message...'"
          floating-toolbar
          slash-commands
          images
          editor-class="nui-editor-content focus:outline-none"
          class="nui-chat-editor min-h-0 flex-1 overflow-y-auto nui-scroll"
          :class="{ 'opacity-50 cursor-not-allowed': disabled }"
          @update:model-value="emit('update:modelValue', $event)"
          @focus="isFocused = true"
          @blur="isFocused = false"
          @keydown="handleKeyDown"
          @image-paste="addImageFile"
          @image-drop="addImageFile"
        />

        <!-- Shortcut hints -->
        <div class="flex w-full items-center gap-4 pt-2.5">
          <div class="flex items-center">
            <NuiKbd keys="Ctrl+Enter" />
            <span class="whitespace-nowrap text-xs leading-normal text-nui-muted">&nbsp;to send</span>
          </div>
          <NuiIcon name="dot" :size="16" class="text-nui-muted" />
          <div class="flex items-center">
            <NuiKbd keys="/" />
            <span class="whitespace-nowrap text-xs leading-normal text-nui-muted">&nbsp;commands</span>
          </div>
          <NuiIcon name="dot" :size="16" class="hidden text-nui-muted sm:inline-block" />
          <div class="hidden items-center sm:flex">
            <NuiKbd keys="Ctrl+P" />
            <span class="whitespace-nowrap text-xs leading-normal text-nui-muted">&nbsp;preview</span>
          </div>
          <span class="min-w-0 flex-1" />
          <span v-if="isActive" class="flex items-center gap-1 whitespace-nowrap text-xs leading-normal text-nui-yellow">
            <span class="h-2 w-2 animate-pulse rounded-full bg-nui-yellow" />
            working — Esc stops
          </span>
        </div>
      </div>

      <!-- Side actions: stop-or-send, attach -->
      <div class="flex h-full w-12 shrink-0 flex-col gap-1">
        <button
          v-if="isActive"
          type="button"
          class="flex w-full items-center justify-center p-2 text-nui-pink transition-colors hover:text-nui-fg"
          title="Stop"
          data-testid="stop-generation"
          @click="emit('stop')"
        >
          <NuiIcon name="close" :size="32" />
          <span class="sr-only">Stop</span>
        </button>
        <button
          v-else
          type="button"
          class="flex w-full items-center justify-center p-2 text-nui-pink transition-colors hover:text-nui-fg disabled:pointer-events-none disabled:opacity-40"
          title="Send"
          :disabled="isEmpty || disabled"
          @click="submit"
        >
          <NuiIcon name="send" :size="32" />
          <span class="sr-only">Send</span>
        </button>
        <button
          type="button"
          class="flex w-full items-center justify-center p-2 text-nui-fg transition-colors hover:text-white"
          title="Attach image"
          @click="openFilePicker"
        >
          <NuiIcon name="attach" :size="32" />
          <span class="sr-only">Attach image</span>
        </button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onBeforeUnmount, onMounted } from 'vue'

const props = defineProps<{
  modelValue: string
  placeholder?: string
  disabled?: boolean
  maxHeight?: number
  isActive?: boolean
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
  (e: 'submit'): void
  (e: 'stop'): void
}>()

const isFocused = ref(false)
const showPreview = ref(false)

interface ImageAttachment {
  id: string
  filename: string
  content_type: string
  data: string
  preview: string
}

const pendingAttachments = ref<ImageAttachment[]>([])
const MAX_IMAGE_SIZE = 5 * 1024 * 1024

function addImageFile(file: File) {
  if (file.size > MAX_IMAGE_SIZE) {
    console.warn('Image too large (max 5MB)')
    return
  }
  const reader = new FileReader()
  reader.onload = () => {
    const dataUrl = reader.result as string
    const base64 = dataUrl.split(',')[1]
    pendingAttachments.value.push({
      id: crypto.randomUUID(),
      filename: file.name,
      content_type: file.type,
      data: base64,
      preview: dataUrl,
    })
  }
  reader.readAsDataURL(file)
}

function removeAttachment(id: string) {
  pendingAttachments.value = pendingAttachments.value.filter(a => a.id !== id)
}

function openFilePicker() {
  const inp = document.createElement('input')
  inp.type = 'file'
  inp.accept = 'image/png,image/jpeg,image/gif,image/webp'
  inp.multiple = true
  inp.onchange = () => {
    if (inp.files) {
      for (const file of inp.files) {
        addImageFile(file)
      }
    }
  }
  inp.click()
}

function getAttachments() {
  const atts = pendingAttachments.value.map(a => ({
    filename: a.filename,
    content_type: a.content_type,
    data: a.data,
  }))
  pendingAttachments.value = []
  return atts
}

function onFocusEvent() { focus() }
function onStopEvent() { if (props.isActive) emit('stop') }

onMounted(() => {
  window.addEventListener('nanna:focus-chat-input', onFocusEvent)
  window.addEventListener('nanna:stop-generation', onStopEvent)
})

onBeforeUnmount(() => {
  window.removeEventListener('nanna:focus-chat-input', onFocusEvent)
  window.removeEventListener('nanna:stop-generation', onStopEvent)
})

// ── RichTextEditor ref ──
const richEditorRef = ref<any>(null)

const isEmpty = computed(() => richEditorRef.value?.isEmpty ?? !props.modelValue.trim())

function handleKeyDown(event: KeyboardEvent) {
  // Escape stops generation when a turn is active (documented Stop shortcut).
  if (event.key === 'Escape' && props.isActive) {
    event.preventDefault()
    event.stopPropagation()
    emit('stop')
    return
  }
  if (event.key === 'p' && (event.ctrlKey || event.metaKey)) {
    event.preventDefault()
    showPreview.value = !showPreview.value
  }
  if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
    event.preventDefault()
    submit()
  }
}

function submit() {
  if (isEmpty.value || props.disabled) return
  emit('submit')
  richEditorRef.value?.clear()
  showPreview.value = false
}

function focus() {
  richEditorRef.value?.focus()
}

defineExpose({ focus, getAttachments })
</script>

<style>
@reference "../assets/css/main.css";

/* ═══ Chat-specific editor overrides (base styles live in RichTextEditor) ═══ */
.nui-chat-editor .rich-text-editor__content .ProseMirror {
  padding: 0;
  min-height: 60px;
  font-family: var(--font-nui);
  font-size: 12px;
  font-weight: 450;
  line-height: normal;
  color: var(--color-nui-fg);
}

.nui-chat-editor .rich-text-editor__content .ProseMirror p.is-empty:first-child::before {
  color: color-mix(in srgb, var(--color-nui-fg) 70%, transparent);
}

/* ═══ Preview transition ═══ */
.preview-enter-active,
.preview-leave-active {
  transition: all 0.2s ease;
}
.preview-enter-from,
.preview-leave-to {
  opacity: 0;
  transform: translateY(8px);
}
</style>
