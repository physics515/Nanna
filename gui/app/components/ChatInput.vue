<template>
  <div class="chat-input-container">
    <!-- Preview pane (collapsible) -->
    <Transition name="preview">
      <div 
        v-if="showPreview && modelValue.trim()"
        class="preview-pane mb-2 rounded-xl border border-nanna-primary/20 bg-nanna-bg-surface/50 overflow-hidden"
      >
        <div class="flex items-center justify-between px-3 py-1.5 border-b border-nanna-primary/10 bg-nanna-bg-elevated/30">
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
    
    <!-- TipTap Editor -->
    <div 
      :class="[
        'relative rounded-xl border transition-all duration-200',
        isFocused 
          ? 'border-nanna-accent/50 ring-2 ring-nanna-accent/20' 
          : 'border-nanna-primary/20 hover:border-nanna-primary/30',
        disabled && 'opacity-50 cursor-not-allowed'
      ]"
    >
      <EditorContent 
        :editor="editor" 
        class="chat-editor"
      />
      
      <!-- Bottom toolbar -->
      <div class="flex items-center justify-between px-3 py-2 border-t border-nanna-primary/10">
        <!-- Left: formatting hints + preview toggle -->
        <div class="flex items-center gap-2 text-xs text-nanna-text-dim">
          <button
            @click="showPreview = !showPreview"
            :class="[
              'flex items-center gap-1 px-1.5 py-0.5 rounded transition-colors',
              showPreview 
                ? 'bg-nanna-primary/20 text-nanna-accent' 
                : 'hover:bg-nanna-primary/10 hover:text-nanna-text'
            ]"
            title="Toggle preview (Ctrl+P)"
          >
            <Eye class="w-3.5 h-3.5" />
            <span class="hidden sm:inline">Preview</span>
          </button>
          <span class="text-nanna-text-muted hidden sm:inline">·</span>
          <span class="hidden md:inline">
            <kbd class="px-1 py-0.5 rounded bg-nanna-bg-elevated text-[10px]">Ctrl+Enter</kbd> send
          </span>
        </div>
        
        <!-- Right: send button -->
        <UiButton 
          @click="submit"
          :disabled="isEmpty || disabled"
          size="sm"
          class="shrink-0"
        >
          <Send class="w-4 h-4 sm:mr-1" />
          <span class="hidden sm:inline">Send</span>
        </UiButton>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onBeforeUnmount } from 'vue'
import { useEditor, EditorContent } from '@tiptap/vue-3'
import StarterKit from '@tiptap/starter-kit'
import Placeholder from '@tiptap/extension-placeholder'
import Link from '@tiptap/extension-link'
import { MonacoCodeBlock } from '~/extensions/MonacoCodeBlock'
import { Send, Eye, X } from 'lucide-vue-next'

const props = defineProps<{
  modelValue: string
  placeholder?: string
  disabled?: boolean
  maxHeight?: number
}>()

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
  (e: 'submit'): void
}>()

const isFocused = ref(false)
const showPreview = ref(false)

// Initialize Tiptap editor with Monaco code blocks
const editor = useEditor({
  content: props.modelValue,
  extensions: [
    StarterKit.configure({
      codeBlock: false, // Disable default, use Monaco
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
      // Ctrl+P to toggle preview
      if (event.key === 'p' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault()
        showPreview.value = !showPreview.value
        return true
      }
      
      // Ctrl+Enter to submit
      if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault()
        submit()
        return true
      }
      
      // Let TipTap handle:
      // - Enter = new paragraph
      // - Shift+Enter = line break (hard break)
      return false
    },
  },
  onUpdate: ({ editor }) => {
    const text = getMarkdownContent(editor)
    emit('update:modelValue', text)
  },
  onFocus: () => { isFocused.value = true },
  onBlur: () => { isFocused.value = false },
})

// Convert editor content to markdown
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
        // Get text content from the node
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

/* Preview transition */
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

/* Tiptap editor styles */
.chat-editor {
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
