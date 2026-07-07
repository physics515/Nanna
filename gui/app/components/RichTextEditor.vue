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

function markdownToHtml(md: string): string {
  if (!md) return ''

  const lines = md.split('\n')
  const html: string[] = []
  let i = 0

  while (i < lines.length) {
    const line = lines[i]

    // Fenced code blocks → MonacoCodeBlock node
    if (line.startsWith('```')) {
      const lang = line.slice(3).trim()
      const codeLines: string[] = []
      i++
      while (i < lines.length && !lines[i].startsWith('```')) {
        codeLines.push(lines[i])
        i++
      }
      i++ // skip closing ```
      const content = codeLines.join('\n')
      // MonacoCodeBlock is an atom node — set attrs via data attributes
      html.push(`<monaco-code-block language="${escAttr(lang)}" content="${escAttr(content)}"></monaco-code-block>`)
      continue
    }

    // Headings
    const headingMatch = line.match(/^(#{1,6})\s+(.*)/)
    if (headingMatch) {
      const level = headingMatch[1].length
      html.push(`<h${level}>${inlineMd(headingMatch[2])}</h${level}>`)
      i++
      continue
    }

    // Blockquote
    if (line.startsWith('> ')) {
      const quoteLines: string[] = []
      while (i < lines.length && lines[i].startsWith('> ')) {
        quoteLines.push(lines[i].slice(2))
        i++
      }
      html.push(`<blockquote><p>${inlineMd(quoteLines.join('<br>'))}</p></blockquote>`)
      continue
    }

    // Task list
    if (line.match(/^- \[([ x])\]\s/)) {
      const items: string[] = []
      while (i < lines.length) {
        const tm = lines[i].match(/^- \[([ x])\]\s+(.*)/)
        if (!tm) break
        const checked = tm[1] === 'x' ? ' data-checked="true"' : ''
        items.push(`<li data-type="taskItem"${checked}><p>${inlineMd(tm[2])}</p></li>`)
        i++
      }
      html.push(`<ul data-type="taskList">${items.join('')}</ul>`)
      continue
    }

    // Unordered list
    if (line.match(/^[-*]\s+/)) {
      const items: string[] = []
      while (i < lines.length && lines[i].match(/^[-*]\s+/)) {
        items.push(`<li><p>${inlineMd(lines[i].replace(/^[-*]\s+/, ''))}</p></li>`)
        i++
      }
      html.push(`<ul>${items.join('')}</ul>`)
      continue
    }

    // Ordered list
    if (line.match(/^\d+\.\s+/)) {
      const items: string[] = []
      while (i < lines.length && lines[i].match(/^\d+\.\s+/)) {
        items.push(`<li><p>${inlineMd(lines[i].replace(/^\d+\.\s+/, ''))}</p></li>`)
        i++
      }
      html.push(`<ol>${items.join('')}</ol>`)
      continue
    }

    // HR
    if (line.match(/^---+$/)) {
      html.push('<hr>')
      i++
      continue
    }

    // Image
    const imgMatch = line.match(/^!\[([^\]]*)\]\(([^)]+)\)$/)
    if (imgMatch) {
      html.push(`<img src="${escAttr(imgMatch[2])}" alt="${escAttr(imgMatch[1])}" />`)
      i++
      continue
    }

    // Empty line
    if (!line.trim()) { i++; continue }

    // Paragraph
    html.push(`<p>${inlineMd(line)}</p>`)
    i++
  }

  return html.join('')
}

function inlineMd(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
    .replace(/\*(.+?)\*/g, '<em>$1</em>')
    .replace(/~~(.+?)~~/g, '<s>$1</s>')
    .replace(/`([^`]+)`/g, '<code>$1</code>')
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2">$1</a>')
}

function escAttr(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

// ── Tiptap JSON → Markdown ──
function getMarkdownContent(ed: any): string {
  if (!ed) return ''
  return jsonToMarkdown(ed.getJSON())
}

function jsonToMarkdown(doc: any): string {
  if (!doc.content) return ''

  return doc.content.map((node: any) => {
    switch (node.type) {
      case 'paragraph':
        return nodeContentToText(node)
      case 'heading': {
        const level = node.attrs?.level || 1
        return '#'.repeat(level) + ' ' + nodeContentToText(node)
      }
      case 'monacoCodeBlock': {
        const lang = node.attrs?.language || ''
        const code = node.content?.[0]?.text || node.attrs?.content || ''
        return '```' + lang + '\n' + code + '\n```'
      }
      case 'bulletList':
        return node.content?.map((item: any) => '- ' + nodeContentToText(item.content?.[0])).join('\n') || ''
      case 'orderedList':
        return node.content?.map((item: any, i: number) => `${i + 1}. ` + nodeContentToText(item.content?.[0])).join('\n') || ''
      case 'taskList':
        return node.content?.map((item: any) => {
          const checked = item.attrs?.checked ? 'x' : ' '
          return `- [${checked}] ` + nodeContentToText(item.content?.[0])
        }).join('\n') || ''
      case 'blockquote':
        return node.content?.map((p: any) => '> ' + nodeContentToText(p)).join('\n') || ''
      case 'horizontalRule':
        return '---'
      case 'image':
        return `![${node.attrs?.alt || ''}](${node.attrs?.src || ''})`
      default:
        return nodeContentToText(node)
    }
  }).reduce((acc: string, block: string, i: number, arr: string[]) => {
    if (i === 0) return block
    const prev = arr[i - 1]
    const isCodeBlock = block.startsWith('```')
    const prevIsCodeBlock = prev.endsWith('```')
    if (isCodeBlock || prevIsCodeBlock) return acc + '\n' + block
    return acc + '\n\n' + block
  }, '').trim()
}

function nodeContentToText(node: any): string {
  if (!node?.content) return ''
  return node.content.map((item: any) => {
    if (item.type === 'image') {
      return `![${item.attrs?.alt || ''}](${item.attrs?.src || ''})`
    }
    let text = item.text || ''
    if (item.marks) {
      for (const mark of item.marks) {
        switch (mark.type) {
          case 'bold': text = `**${text}**`; break
          case 'italic': text = `*${text}*`; break
          case 'strike': text = `~~${text}~~`; break
          case 'code': text = '`' + text + '`'; break
          case 'link': text = `[${text}](${mark.attrs?.href || ''})`; break
        }
      }
    }
    return text
  }).join('')
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
  @apply px-1.5 py-0.5 rounded bg-nanna-bg-elevated text-nanna-accent font-mono text-xs;
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
