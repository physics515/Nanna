<template>
  <NodeViewWrapper class="tiptap-monaco-wrapper my-2" data-type="monacoCodeBlock">
    <div class="monaco-code-block rounded-lg border border-nanna-primary/30 bg-nanna-bg-surface overflow-hidden">
      <!-- Header with language selector -->
      <div class="flex items-center justify-between px-3 py-1.5 bg-nanna-bg-elevated/50 border-b border-nanna-primary/20">
        <input
          ref="langInput"
          v-model="localLanguage"
          type="text"
          placeholder="language"
          spellcheck="false"
          class="bg-transparent text-[10px] uppercase tracking-wider text-nanna-text-dim font-medium w-24 outline-none placeholder:text-nanna-text-dim/50"
          @change="updateLanguage"
          @keydown.enter.prevent="focusEditor"
        />
        <div class="flex items-center gap-1">
          <button 
            @click="copyCode"
            class="p-1 rounded text-nanna-text-dim hover:text-nanna-text hover:bg-nanna-primary/20 transition-colors"
            :title="copied ? 'Copied!' : 'Copy code'"
          >
            <Check v-if="copied" class="w-3.5 h-3.5 text-nanna-success" />
            <Copy v-else class="w-3.5 h-3.5" />
          </button>
          <button 
            @click="deleteNode"
            class="p-1 rounded text-nanna-text-dim hover:text-nanna-error hover:bg-nanna-error/10 transition-colors"
            title="Remove code block"
          >
            <X class="w-3.5 h-3.5" />
          </button>
        </div>
      </div>
      
      <!-- Monaco Editor -->
      <div class="monaco-embed" :style="{ height: computedHeight }">
        <VueMonacoEditor
          ref="monacoRef"
          v-model:value="localCode"
          :language="monacoLanguage"
          theme="nanna-dark"
          :options="editorOptions"
          @mount="handleMount"
          @change="handleCodeChange"
        />
      </div>
    </div>
  </NodeViewWrapper>
</template>

<script setup lang="ts">
import { ref, computed, watch, onMounted } from 'vue'
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { VueMonacoEditor } from '@guolao/vue-monaco-editor'
import { X, Copy, Check } from 'lucide-vue-next'
import type * as Monaco from 'monaco-editor'

const props = defineProps(nodeViewProps)

const monacoRef = ref()
const langInput = ref()
const copied = ref(false)
let editorInstance: Monaco.editor.IStandaloneCodeEditor | null = null

// Local state
const localCode = ref('')
const localLanguage = ref('')

// Initialize from node
onMounted(() => {
  localLanguage.value = props.node.attrs.language || ''
  // Get text content from node
  localCode.value = props.node.textContent || ''
})

// Watch for external changes to the node
watch(() => props.node.attrs.language, (newLang) => {
  if (newLang !== localLanguage.value) {
    localLanguage.value = newLang || ''
  }
})

watch(() => props.node.textContent, (newContent) => {
  if (newContent !== localCode.value) {
    localCode.value = newContent || ''
  }
})

// Language mapping
const languageMap: Record<string, string> = {
  'js': 'javascript', 'ts': 'typescript', 'jsx': 'javascript', 'tsx': 'typescript',
  'rs': 'rust', 'py': 'python', 'rb': 'ruby', 'go': 'go',
  'c': 'c', 'cpp': 'cpp', 'cc': 'cpp', 'cxx': 'cpp', 'c++': 'cpp', 'h': 'c', 'hpp': 'cpp',
  'sh': 'shell', 'bash': 'shell', 'zsh': 'shell',
  'yml': 'yaml', 'md': 'markdown', 'json': 'json', 'toml': 'ini',
}

const monacoLanguage = computed(() => {
  const lang = localLanguage.value?.toLowerCase() || ''
  return languageMap[lang] || lang || 'plaintext'
})

// Auto-height based on lines
const computedHeight = computed(() => {
  const lineCount = Math.max(1, (localCode.value.match(/\n/g) || []).length + 1)
  const height = Math.min(350, Math.max(80, lineCount * 19 + 20))
  return `${height}px`
})

// Monaco options
const editorOptions = computed(() => ({
  minimap: { enabled: false },
  lineNumbers: 'on' as const,
  lineNumbersMinChars: 3,
  glyphMargin: false,
  folding: false,
  lineDecorationsWidth: 0,
  scrollBeyondLastLine: false,
  renderLineHighlight: 'line' as const,
  scrollbar: {
    vertical: 'auto' as const,
    horizontal: 'auto' as const,
    verticalScrollbarSize: 6,
    horizontalScrollbarSize: 6,
  },
  wordWrap: 'off' as const,
  fontSize: 13,
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  fontLigatures: true,
  padding: { top: 8, bottom: 8 },
  automaticLayout: true,
}))

function handleMount(editor: Monaco.editor.IStandaloneCodeEditor, monaco: typeof Monaco) {
  editorInstance = editor
  
  // Escape to blur and return to TipTap
  editor.addCommand(monaco.KeyCode.Escape, () => {
    editor.blur()
    // Focus back on TipTap editor
    props.editor.commands.focus()
  })
}

function handleCodeChange(value: string) {
  // Update the node content in TipTap
  const { state, view } = props.editor
  const { from, to } = props.getPos 
    ? { from: props.getPos(), to: props.getPos() + props.node.nodeSize }
    : { from: 0, to: 0 }
  
  if (from === 0 && to === 0) return
  
  // Create a transaction to update the text content
  const transaction = state.tr
  
  // Delete old content and insert new
  const nodeStart = from + 1 // +1 to get inside the node
  const nodeEnd = to - 1 // -1 to stay inside the node
  
  if (nodeEnd > nodeStart) {
    transaction.delete(nodeStart, nodeEnd)
  }
  
  if (value) {
    transaction.insertText(value, nodeStart)
  }
  
  view.dispatch(transaction)
}

function updateLanguage() {
  props.updateAttributes({ language: localLanguage.value })
}

function focusEditor() {
  editorInstance?.focus()
}

async function copyCode() {
  try {
    await navigator.clipboard.writeText(localCode.value)
    copied.value = true
    setTimeout(() => { copied.value = false }, 2000)
  } catch (err) {
    console.error('Failed to copy:', err)
  }
}
</script>

<style>
@reference "../assets/css/main.css";

.tiptap-monaco-wrapper {
  @apply relative;
}

.monaco-code-block {
  /* Don't capture all clicks - let Monaco handle them */
}

.monaco-embed {
  @apply w-full;
}

.monaco-embed :deep(.monaco-editor),
.monaco-embed :deep(.monaco-editor-background),
.monaco-embed :deep(.monaco-editor .margin) {
  background-color: #1e293b !important;
}

/* Ensure Monaco captures keyboard events */
.monaco-embed :deep(.monaco-editor textarea) {
  @apply outline-none;
}
</style>
