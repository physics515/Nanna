<template>
  <div class="chat-input-container">
    <!-- Preview pane (collapsible) -->
    <Transition name="preview">
      <div
        v-if="showPreview && modelValue.trim()"
        class="preview-pane mb-2 rounded-xl overflow-hidden glass-editor-wrap"
      >
        <div class="flex items-center justify-between px-3 py-1.5 border-b border-white/[0.04]">
          <span class="text-[10px] uppercase tracking-wider text-nanna-text-dim font-medium">Preview</span>
          <button
            @click="showPreview = false"
            class="text-nanna-text-dim hover:text-nanna-text p-0.5"
          >
            <X class="w-3.5 h-3.5" />
          </button>
        </div>
        <div class="p-3 max-h-[200px] overflow-y-auto">
          <MarkdownContent :content="modelValue" />
        </div>
      </div>
    </Transition>

    <!-- Combined input card -->
    <div class="input-card" :class="{ 'input-card--focused': isFocused }">

      <!-- ═══ Top: Editor with Splatter ═══ -->
      <div class="input-editor-zone">
        <!-- Splatter layer -->
        <span class="input-editor-zone__splatter" :style="{ background: splatterBg }" />

        <!-- Rich text editor (extracted Tiptap core) -->
        <RichTextEditor
          ref="richEditorRef"
          :model-value="modelValue"
          :editable="!disabled"
          :placeholder="placeholder || 'Type a message... (Ctrl+Enter to send, / for commands)'"
          floating-toolbar
          slash-commands
          images
          editor-class="prose prose-invert prose-sm max-w-none focus:outline-none"
          class="chat-editor"
          :class="{ 'opacity-50 cursor-not-allowed': disabled }"
          @update:model-value="emit('update:modelValue', $event)"
          @focus="isFocused = true; splatterEnter()"
          @blur="isFocused = false; splatterLeave()"
          @keydown="handleKeyDown"
          @image-paste="addImageFile"
          @image-drop="addImageFile"
        />
      </div>

      <!-- ═══ Mobile formatting toolbar ═══ -->
      <div v-if="isFocused && tiptapEditor" class="mobile-toolbar sm:hidden">
        <button
          @click="tiptapEditor.chain().focus().toggleBold().run()"
          :class="{ active: tiptapEditor.isActive('bold') }"
          class="mobile-toolbar__btn"
        >
          <Bold class="w-4 h-4" />
        </button>
        <button
          @click="tiptapEditor.chain().focus().toggleItalic().run()"
          :class="{ active: tiptapEditor.isActive('italic') }"
          class="mobile-toolbar__btn"
        >
          <Italic class="w-4 h-4" />
        </button>
        <button
          @click="tiptapEditor.chain().focus().toggleStrike().run()"
          :class="{ active: tiptapEditor.isActive('strike') }"
          class="mobile-toolbar__btn"
        >
          <Strikethrough class="w-4 h-4" />
        </button>
        <button
          @click="tiptapEditor.chain().focus().toggleCode().run()"
          :class="{ active: tiptapEditor.isActive('code') }"
          class="mobile-toolbar__btn"
        >
          <Code class="w-4 h-4" />
        </button>
        <span class="mobile-toolbar__divider" />
        <button
          @click="tiptapEditor.chain().focus().toggleHeading({ level: 2 }).run()"
          :class="{ active: tiptapEditor.isActive('heading') }"
          class="mobile-toolbar__btn"
        >
          <Heading2 class="w-4 h-4" />
        </button>
        <button
          @click="tiptapEditor.chain().focus().toggleBulletList().run()"
          :class="{ active: tiptapEditor.isActive('bulletList') }"
          class="mobile-toolbar__btn"
        >
          <List class="w-4 h-4" />
        </button>
        <button
          @click="tiptapEditor.chain().focus().toggleBlockquote().run()"
          :class="{ active: tiptapEditor.isActive('blockquote') }"
          class="mobile-toolbar__btn"
        >
          <Quote class="w-4 h-4" />
        </button>
        <button
          @click="tiptapEditor.chain().focus().toggleTaskList().run()"
          :class="{ active: tiptapEditor.isActive('taskList') }"
          class="mobile-toolbar__btn"
        >
          <ListChecks class="w-4 h-4" />
        </button>
      </div>

      <!-- ═══ Bottom: Toolbar with Ground Glass ═══ -->
    <!-- Attachment previews -->
    <div v-if="pendingAttachments.length > 0" class="attachment-strip">
      <div v-for="att in pendingAttachments" :key="att.id" class="attachment-thumb">
        <img :src="att.preview" :alt="att.filename" />
        <button class="attachment-remove" @click="removeAttachment(att.id)">
          <X class="w-3 h-3" />
        </button>
      </div>
    </div>
      <div
        class="input-toolbar"
        :style="glassStyle"
        @mouseenter="handleToolbarEnter"
        @mouseleave="handleToolbarLeave"
      >
        <!-- Glass mesh layer -->
        <span class="input-toolbar__mesh" :style="{ background: meshBg }" />

        <!-- Glass noise overlay -->
        <span class="input-toolbar__noise" />

        <!-- Toolbar content -->
        <div class="input-toolbar__content">
          <!-- Left: formatting hints + preview toggle -->
          <div class="flex items-center gap-2 text-xs" style="color: rgba(203, 213, 225, 0.85);">
            <UiGlassButton
              pill
              size="sm"
              :color="showPreview ? 'accent' : 'default'"
              @click="showPreview = !showPreview"
              title="Toggle preview (Ctrl+P)"
            >
              <Eye class="w-3.5 h-3.5 sm:mr-1" />
              <span class="hidden sm:inline">Preview</span>
            </UiGlassButton>
            <UiGlassButton
              pill
              size="sm"
              @click="openFilePicker"
              title="Attach image"
            >
              <ImagePlus class="w-3.5 h-3.5 sm:mr-1" />
              <span class="hidden sm:inline">Image</span>
            </UiGlassButton>
            <span class="text-nanna-text-muted hidden sm:inline">&middot;</span>
            <span class="hidden md:inline">
              <kbd class="px-1 py-0.5 rounded bg-white/[0.04] text-[10px]">Ctrl+Enter</kbd> send
              <span class="text-nanna-text-muted ml-1">&middot;</span>
              <kbd class="px-1 py-0.5 rounded bg-white/[0.04] text-[10px] ml-1">/</kbd> commands
            </span>
          </div>

          <!-- Right: stop or send button -->
          <UiGlassButton
            v-if="isActive"
            @click="emit('stop')"
            size="sm"
            color="danger"
            pill
            class="shrink-0"
          >
            <Square class="w-3.5 h-3.5 sm:mr-1 fill-current" />
            <span class="hidden sm:inline">Stop</span>
          </UiGlassButton>
          <UiGlassButton
            v-else
            @click="submit"
            :disabled="isEmpty || disabled"
            size="sm"
            color="accent"
            pill
            class="shrink-0"
          >
            <Send class="w-4 h-4 sm:mr-1" />
            <span class="hidden sm:inline">Send</span>
          </UiGlassButton>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount, onMounted } from 'vue'
import {
  Send, Eye, X, Square, ImagePlus,
  Bold, Italic, Strikethrough, Code,
  Heading2, List, Quote, ListChecks,
} from 'lucide-vue-next'
import { useSplatter } from '~/composables/useSplatter'
import { useGroundGlass } from '~/composables/useGroundGlass'

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


// Splatter for the editor area (focus-driven)
const {
  splatterBg,
  onEnter: splatterEnter,
  onLeave: splatterLeave,
} = useSplatter({
  opacityRanges: [[0.08, 0.12], [0.06, 0.10], [0.04, 0.08]],
  sizes: ['65%', '60%', '50%'],
  lerpSpeed: 0.008,
  interval: 3000,
})

// Ground glass for the toolbar (hover-driven)
const {
  meshBg,
  containerStyle: glassStyle,
  onEnter: glassEnter,
  onLeave: glassLeave,
} = useGroundGlass({
  opacity: 1.8,
  sizes: ['55%', '50%', '45%'],
  lerpSpeed: 0.008,
  interval: 2200,
  blur: 8,
})

// Prevent mount animation
const ready = ref(false)
onMounted(() => { setTimeout(() => { ready.value = true }, 200) })

function handleToolbarEnter() {
  if (ready.value) glassEnter()
}
function handleToolbarLeave() {
  if (ready.value) glassLeave()
}

// ── RichTextEditor ref ──
const richEditorRef = ref<any>(null)
const tiptapEditor = computed(() => richEditorRef.value?.editor)

const isEmpty = computed(() => richEditorRef.value?.isEmpty ?? !props.modelValue.trim())

function handleKeyDown(event: KeyboardEvent) {
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

/* ═══ Input Card (outer shell) ═══ */
.input-card {
  border-radius: 0 0 0.75rem 0.75rem;
  overflow: hidden;
}

/* ═══ Editor Zone (splatter) ═══ */
.input-editor-zone {
  position: relative;
  isolation: isolate;
}

.input-editor-zone__splatter {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  transition: opacity 0.4s ease;
  opacity: 0.5;
}
.input-card--focused .input-editor-zone__splatter {
  opacity: 1;
}

/* ═══ Mobile formatting toolbar ═══ */
.mobile-toolbar {
  display: flex;
  align-items: center;
  gap: 0.125rem;
  padding: 0.375rem 0.5rem;
  border-top: 1px solid rgba(255, 255, 255, 0.04);
  background: rgba(15, 23, 42, 0.3);
  overflow-x: auto;
  -webkit-overflow-scrolling: touch;
}

.mobile-toolbar__btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  border-radius: 0.375rem;
  color: rgba(203, 213, 225, 0.7);
  transition: all 0.15s ease;
  flex-shrink: 0;
}

.mobile-toolbar__btn:hover,
.mobile-toolbar__btn:active {
  background: rgba(99, 102, 241, 0.15);
  color: #e2e8f0;
}

.mobile-toolbar__btn.active {
  background: rgba(99, 102, 241, 0.25);
  color: rgba(165, 180, 252, 1);
}

.mobile-toolbar__divider {
  width: 1px;
  height: 1.25rem;
  margin: 0 0.25rem;
  background: rgba(71, 85, 105, 0.4);
  flex-shrink: 0;
}

/* ═══ Toolbar (ground glass) ═══ */
.input-toolbar {
  position: relative;
  isolation: isolate;
  overflow: hidden;
  background: rgba(30, 41, 59, 0.25);
  border-top: 1px solid rgba(255, 255, 255, 0.04);
}

.input-toolbar__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
}

.input-toolbar__noise {
  position: absolute;
  inset: 0;
  z-index: 2;
  pointer-events: none;
  opacity: 0.14;
  background-blend-mode: soft-light;
  background: repeating-radial-gradient(
    circle,
    #1a2035,
    #1a2035 2px,
    #253050 2px 4px,
    #1a2035 4px 6px,
    #253050 6px 8px,
    #1a2035 8px 10px,
    #253050 10px 12px
  ) 0 0 / 100% 100%;
}

.input-toolbar__content {
  position: relative;
  z-index: 3;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 0.75rem;
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

.preview-pane {
  @apply text-sm;
}

/* ═══ Glass preview wrap ═══ */
.glass-editor-wrap {
  background: rgba(15, 23, 42, 0.5);
  border: 1px solid rgba(71, 85, 105, 0.2);
}

/* ═══ Chat-specific editor overrides (base styles live in RichTextEditor) ═══ */
.chat-editor {
  position: relative;
  z-index: 1;
  @apply min-h-[60px] max-h-[400px] overflow-y-auto;
}

.chat-editor :deep(.rich-text-editor__content .ProseMirror) {
  @apply px-4 py-3;
  min-height: 60px;
}

/* ═══ Drag handles for blocks ═══ */
.chat-editor :deep(.ProseMirror > *) {
  position: relative;
}

.chat-editor :deep(.ProseMirror > *:not(p:first-child)::before) {
  content: '⠿';
  position: absolute;
  left: -1.25rem;
  top: 0.125rem;
  font-size: 0.75rem;
  color: transparent;
  cursor: grab;
  user-select: none;
  transition: color 0.15s ease;
  line-height: 1.5;
}

.chat-editor :deep(.ProseMirror > *:not(p:first-child):hover::before) {
  color: rgba(148, 163, 184, 0.35);
}

/* === Attachment strip === */
.attachment-strip {
  display: flex;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem;
  border-top: 1px solid rgba(255, 255, 255, 0.04);
  overflow-x: auto;
}

.attachment-thumb {
  position: relative;
  flex-shrink: 0;
  width: 4rem;
  height: 4rem;
  border-radius: 0.5rem;
  overflow: hidden;
  border: 1px solid rgba(99, 102, 241, 0.3);
}

.attachment-thumb img {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.attachment-remove {
  position: absolute;
  top: 0.125rem;
  right: 0.125rem;
  width: 1.25rem;
  height: 1.25rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  background: rgba(0, 0, 0, 0.7);
  color: rgba(248, 113, 113, 0.9);
  transition: all 0.15s ease;
}

.attachment-remove:hover {
  background: rgba(220, 38, 38, 0.8);
  color: white;
}

</style>
