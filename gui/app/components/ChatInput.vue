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

        <!-- Editor content -->
        <EditorContent
          :editor="editor"
          class="chat-editor"
          :class="{ 'opacity-50 cursor-not-allowed': disabled }"
        />
      </div>

      <!-- ═══ Bottom: Toolbar with Ground Glass ═══ -->
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
            <span class="text-nanna-text-muted hidden sm:inline">&middot;</span>
            <span class="hidden md:inline">
              <kbd class="px-1 py-0.5 rounded bg-white/[0.04] text-[10px]">Ctrl+Enter</kbd> send
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
import { useEditor, EditorContent } from '@tiptap/vue-3'
import StarterKit from '@tiptap/starter-kit'
import Placeholder from '@tiptap/extension-placeholder'
import Link from '@tiptap/extension-link'
import { MonacoCodeBlock } from '~/extensions/MonacoCodeBlock'
import { Send, Eye, X, Square } from 'lucide-vue-next'
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

// Initialize Tiptap editor with Monaco code blocks
const editor = useEditor({
  content: props.modelValue,
  extensions: [
    StarterKit.configure({
      codeBlock: false,
      heading: false,
      horizontalRule: false,
    }),
    MonacoCodeBlock,
    Link.configure({
      openOnClick: false,
      HTMLAttributes: {
        class: 'text-nanna-accent hover:underline',
      },
    }),
    Placeholder.configure({
      placeholder: props.placeholder || 'Type a message... (Ctrl+Enter to send)',
      emptyEditorClass: 'is-empty',
    }),
  ],
  editorProps: {
    attributes: {
      class: 'prose prose-invert prose-sm max-w-none focus:outline-none',
    },
    handleKeyDown: (view, event) => {
      if (event.key === 'p' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault()
        showPreview.value = !showPreview.value
        return true
      }
      if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault()
        submit()
        return true
      }
      return false
    },
  },
  onUpdate: ({ editor }) => {
    const text = getMarkdownContent(editor)
    emit('update:modelValue', text)
  },
  onFocus: () => {
    isFocused.value = true
    splatterEnter()
  },
  onBlur: () => {
    isFocused.value = false
    splatterLeave()
  },
})

function getMarkdownContent(editorInstance: any): string {
  if (!editorInstance) return ''
  const json = editorInstance.getJSON()
  return jsonToMarkdown(json)
}

function jsonToMarkdown(doc: any): string {
  if (!doc.content) return ''

  return doc.content.map((node: any) => {
    switch (node.type) {
      case 'paragraph':
        return nodeContentToText(node)
      case 'monacoCodeBlock':
        const lang = node.attrs?.language || ''
        const code = node.content?.[0]?.text || node.attrs?.content || ''
        return '```' + lang + '\n' + code + '\n```'
      case 'bulletList':
        return node.content?.map((item: any) => '- ' + nodeContentToText(item.content?.[0])).join('\n') || ''
      case 'orderedList':
        return node.content?.map((item: any, i: number) => `${i + 1}. ` + nodeContentToText(item.content?.[0])).join('\n') || ''
      case 'blockquote':
        return node.content?.map((p: any) => '> ' + nodeContentToText(p)).join('\n') || ''
      default:
        return nodeContentToText(node)
    }
  }).join('\n\n').trim()
}

function nodeContentToText(node: any): string {
  if (!node?.content) return ''

  return node.content.map((item: any) => {
    let text = item.text || ''

    if (item.marks) {
      for (const mark of item.marks) {
        switch (mark.type) {
          case 'bold':
            text = `**${text}**`
            break
          case 'italic':
            text = `*${text}*`
            break
          case 'code':
            text = '`' + text + '`'
            break
          case 'link':
            text = `[${text}](${mark.attrs?.href || ''})`
            break
        }
      }
    }

    return text
  }).join('')
}

const isEmpty = computed(() => {
  if (!editor.value) return true
  return editor.value.isEmpty
})

function submit() {
  if (isEmpty.value || props.disabled) return
  emit('submit')
  editor.value?.commands.clearContent()
  showPreview.value = false
}

function focus() {
  editor.value?.commands.focus()
}

defineExpose({ focus })

watch(() => props.modelValue, (newValue) => {
  if (!editor.value) return
  const currentContent = getMarkdownContent(editor.value)
  if (newValue !== currentContent && newValue === '') {
    editor.value.commands.clearContent()
  }
})

onBeforeUnmount(() => {
  editor.value?.destroy()
})
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

/* ═══ Tiptap editor styles ═══ */
.chat-editor {
  position: relative;
  z-index: 1;
  @apply min-h-[60px] max-h-[400px] overflow-y-auto;
}

.chat-editor .ProseMirror {
  @apply px-4 py-3 text-sm text-nanna-text;
  min-height: 60px;
}

.chat-editor .ProseMirror:focus {
  @apply outline-none;
}

/* Placeholder */
.chat-editor .ProseMirror p.is-empty:first-child::before {
  @apply text-nanna-text-dim pointer-events-none float-left h-0;
  content: attr(data-placeholder);
}

/* Inline code */
.chat-editor code {
  @apply px-1.5 py-0.5 rounded bg-nanna-bg-elevated text-nanna-accent font-mono text-xs;
}

/* Bold */
.chat-editor strong {
  @apply font-bold text-nanna-text;
}

/* Italic */
.chat-editor em {
  @apply italic;
}

/* Lists */
.chat-editor ul {
  @apply list-disc list-inside my-1;
}

.chat-editor ol {
  @apply list-decimal list-inside my-1;
}

.chat-editor li {
  @apply text-nanna-text;
}

/* Blockquotes */
.chat-editor blockquote {
  @apply border-l-2 border-nanna-accent/50 pl-3 my-2 text-nanna-text-muted italic;
}

/* Links */
.chat-editor a {
  @apply text-nanna-accent hover:underline;
}

/* Scrollbar */
.chat-editor::-webkit-scrollbar {
  @apply w-2;
}

.chat-editor::-webkit-scrollbar-track {
  @apply bg-transparent;
}

.chat-editor::-webkit-scrollbar-thumb {
  @apply bg-nanna-primary/20 rounded-full;
}

.chat-editor::-webkit-scrollbar-thumb:hover {
  @apply bg-nanna-primary/30;
}
</style>
