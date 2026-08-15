<script setup lang="ts">
/**
 * RichTextEditor — reusable Tiptap editor with Monaco code blocks.
 *
 * Extracted from ChatInput so the same rich-text experience (inline formatting,
 * code blocks, task lists, typography, markdown round-trip) can be used anywhere:
 * chat input, memory cards, note editors, etc.
 *
 * Accepts markdown via v-model, converts internally to Tiptap HTML, and emits
 * markdown back on changes.
 */
import { ref, computed, watch, onBeforeUnmount, onMounted } from 'vue'
import { useEditor, EditorContent } from '@tiptap/vue-3'
import StarterKit from '@tiptap/starter-kit'
import Placeholder from '@tiptap/extension-placeholder'
import Link from '@tiptap/extension-link'
import TaskList from '@tiptap/extension-task-list'
import TaskItem from '@tiptap/extension-task-item'
import Image from '@tiptap/extension-image'
import Typography from '@tiptap/extension-typography'
import { MonacoCodeBlock } from '~/extensions/MonacoCodeBlock'
// Markdown ↔ Tiptap conversion lives in lib/ so the outbound path (the one
// that decides what the daemon actually receives) is unit-testable.
import { jsonToMarkdown, markdownToHtml } from '~/lib/tiptapMarkdown'

const props = withDefaults(defineProps<{
  modelValue?: string
  placeholder?: string
  editable?: boolean
  /** Show floating toolbar on text selection */
  floatingToolbar?: boolean
  /** Allow slash commands */
  slashCommands?: boolean
  /** Allow image paste/drop */
  images?: boolean
  /** CSS class for the ProseMirror element */
  editorClass?: string
}>(), {
  modelValue: '',
  placeholder: '',
  editable: true,
  floatingToolbar: false,
  slashCommands: false,
  images: false,
  editorClass: 'prose prose-invert prose-sm max-w-none focus:outline-none',
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'focus': []
  'blur': []
  'keydown': [event: KeyboardEvent, view: any]
  'image-drop': [file: File]
  'image-paste': [file: File]
}>()

// ── Build extensions list ──
function buildExtensions() {
  const exts: any[] = [
    StarterKit.configure({ codeBlock: false }),
    MonacoCodeBlock,
    Link.configure({
      openOnClick: !props.editable,
      // Content integrity: what leaves the composer must be what the user
      // typed or pasted. Autolink rewrites bare text as it is entered — a
      // pasted "test_01.sh" came out as "test_[01.sh](http://01.sh)" because
      // ".sh" is a live TLD, and that corrupted mission text reached the
      // model. Only the READ-ONLY view (rendering markdown we received) keeps
      // autolinking, where there is no outbound text to corrupt.
      autolink: !props.editable,
      HTMLAttributes: { class: 'text-nanna-accent hover:underline' },
    }),
    Placeholder.configure({
      placeholder: props.placeholder,
      emptyEditorClass: 'is-empty',
    }),
    TaskList,
    TaskItem.configure({ nested: true }),
    Typography,
  ]

  if (props.images) {
    exts.push(Image.configure({ inline: true, allowBase64: true }))
  }

  // Lazy-load SlashCommands only if needed
  if (props.slashCommands) {
    try {
      const { SlashCommands } = require('~/extensions/SlashCommands')
      exts.push(SlashCommands)
    } catch { /* slash commands extension not available */ }
  }

  return exts
}

// ── Editor init ──
const initialising = ref(true)

const editor = useEditor({
  content: markdownToHtml(props.modelValue),
  editable: props.editable,
  extensions: buildExtensions(),
  editorProps: {
    attributes: {
      class: props.editorClass,
    },
    handlePaste: (view, event) => {
      if (!props.images) return false
      const items = event.clipboardData?.items
      if (!items) return false
      for (const item of items) {
        if (item.type.startsWith('image/')) {
          event.preventDefault()
          const file = item.getAsFile()
          if (file) emit('image-paste', file)
          return true
        }
      }
      return false
    },
    handleDrop: (view, event) => {
      if (!props.images) return false
      const files = event.dataTransfer?.files
      if (!files) return false
      for (const file of files) {
        if (file.type.startsWith('image/')) {
          event.preventDefault()
          emit('image-drop', file)
          return true
        }
      }
      return false
    },
    handleKeyDown: (view, event) => {
      emit('keydown', event, view)
      // Return false — let parent decide via event handler
      return false
    },
  },
  onUpdate: ({ editor: ed }) => {
    if (initialising.value) return
    emit('update:modelValue', getMarkdownContent(ed))
  },
  onFocus: () => emit('focus'),
  onBlur: () => emit('blur'),
})

onMounted(() => {
  // Allow initial content to settle before emitting updates
  nextTick(() => { initialising.value = false })
})

// ── Sync editable prop ──
watch(() => props.editable, (val) => {
  editor.value?.setEditable(val)
})

// ── Sync content from outside ──
watch(() => props.modelValue, (val) => {
  if (!editor.value || initialising.value) return
  const current = getMarkdownContent(editor.value)
  if (val !== current) {
    initialising.value = true
    editor.value.commands.setContent(markdownToHtml(val))
    nextTick(() => { initialising.value = false })
  }
})

onBeforeUnmount(() => {
  editor.value?.destroy()
})

// ── Public API ──
const isEmpty = computed(() => editor.value?.isEmpty ?? true)

function focus() {
  editor.value?.commands.focus()
}

function clear() {
  editor.value?.commands.clearContent()
}

function getContent(): string {
  return editor.value ? getMarkdownContent(editor.value) : ''
}

defineExpose({ editor, isEmpty, focus, clear, getContent })

// ═══════════════════════════════════════════════════════════
// Markdown ↔ Tiptap conversion
// ═══════════════════════════════════════════════════════════
// See ~/lib/tiptapMarkdown — markdownToHtml / jsonToMarkdown are imported
// above so the outbound conversion can be tested without an editor instance.

function getMarkdownContent(ed: any): string {
  if (!ed) return ''
  return jsonToMarkdown(ed.getJSON())
}
</script>

<template>
  <div class="rich-text-editor" :class="{ 'rich-text-editor--editable': editable }">
    <FloatingToolbar v-if="floatingToolbar && editor" :editor="editor" />
    <EditorContent :editor="editor" class="rich-text-editor__content" />
  </div>
</template>

<style>
@reference "../assets/css/main.css";

/* ═══ Base ═══ */
.rich-text-editor {
  position: relative;
}

.rich-text-editor__content .ProseMirror {
  @apply px-3 py-2 text-sm text-nanna-text;
  outline: none;
  min-height: 1.5em;
}

.rich-text-editor--editable .rich-text-editor__content .ProseMirror {
  min-height: 3em;
}

/* ═══ Placeholder ═══ */
.rich-text-editor__content .ProseMirror p.is-empty:first-child::before {
  @apply text-nanna-text-dim pointer-events-none float-left h-0;
  content: attr(data-placeholder);
}

/* ═══ Inline code ═══ */
.rich-text-editor__content code {
  @apply px-1.5 py-0.5 rounded glass-tag text-nanna-accent font-mono text-xs;
}

/* ═══ Bold / Italic / Strike ═══ */
.rich-text-editor__content strong { @apply font-bold text-nanna-text; }
.rich-text-editor__content em { @apply italic; }
.rich-text-editor__content s { @apply text-nanna-text-muted; }

/* ═══ Headings ═══ */
.rich-text-editor__content h1 { @apply text-xl font-bold text-nanna-text mt-2 mb-1; }
.rich-text-editor__content h2 { @apply text-lg font-semibold text-nanna-text mt-2 mb-1; }
.rich-text-editor__content h3 { @apply text-base font-semibold text-nanna-text mt-1.5 mb-1; }

/* ═══ Lists ═══ */
.rich-text-editor__content ul { @apply list-disc list-inside my-1; }
.rich-text-editor__content ol { @apply list-decimal list-inside my-1; }
.rich-text-editor__content li { @apply text-nanna-text; }

/* ═══ Task lists ═══ */
.rich-text-editor__content ul[data-type="taskList"] { @apply list-none pl-0 my-1; }
.rich-text-editor__content ul[data-type="taskList"] li { @apply flex items-start gap-2; }
.rich-text-editor__content ul[data-type="taskList"] li label { @apply flex items-center; }
.rich-text-editor__content ul[data-type="taskList"] li label input[type="checkbox"] {
  @apply w-3.5 h-3.5 rounded border-nanna-primary/40 bg-transparent mt-0.5;
  accent-color: rgba(99, 102, 241, 0.8);
}
.rich-text-editor__content ul[data-type="taskList"] li div { @apply flex-1; }

/* ═══ Blockquote ═══ */
.rich-text-editor__content blockquote {
  @apply border-l-2 border-nanna-accent/50 pl-3 my-2 text-nanna-text-muted italic;
}

/* ═══ HR ═══ */
.rich-text-editor__content hr { @apply border-nanna-primary/20 my-3; }

/* ═══ Images ═══ */
.rich-text-editor__content img {
  @apply max-w-full rounded-lg my-2;
  max-height: 200px;
}

/* ═══ Links ═══ */
.rich-text-editor__content a { @apply text-nanna-accent hover:underline; }

/* ═══ Scrollbar ═══ */
.rich-text-editor__content::-webkit-scrollbar { @apply w-2; }
.rich-text-editor__content::-webkit-scrollbar-track { @apply bg-transparent; }
.rich-text-editor__content::-webkit-scrollbar-thumb { @apply bg-nanna-primary/20 rounded-full; }
.rich-text-editor__content::-webkit-scrollbar-thumb:hover { @apply bg-nanna-primary/30; }
</style>
