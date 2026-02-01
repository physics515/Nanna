<template>
  <div class="code-block-container" :class="{ 'code-block-editable': !readOnly }">
    <!-- Language label + copy button -->
    <div v-if="language || showCopy" class="code-block-header">
      <span v-if="language" class="code-lang">{{ language }}</span>
      <button 
        v-if="showCopy" 
        class="copy-btn"
        @click="copyCode"
        :title="copied ? 'Copied!' : 'Copy code'"
      >
        <Check v-if="copied" class="w-3.5 h-3.5" />
        <Copy v-else class="w-3.5 h-3.5" />
      </button>
    </div>
    
    <!-- Monaco Editor -->
    <div class="monaco-wrapper" :style="{ height: computedHeight }">
      <VueMonacoEditor
        v-model:value="internalValue"
        :language="monacoLanguage"
        theme="nanna-dark"
        :options="editorOptions"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import { VueMonacoEditor } from '@guolao/vue-monaco-editor'
import { Copy, Check } from 'lucide-vue-next'

const props = withDefaults(defineProps<{
  modelValue?: string
  code?: string
  language?: string
  readOnly?: boolean
  height?: string | number
  minHeight?: number
  maxHeight?: number
  showCopy?: boolean
  lineNumbers?: boolean
  wordWrap?: boolean
}>(), {
  modelValue: '',
  code: '',
  readOnly: true,
  minHeight: 60,
  maxHeight: 400,
  showCopy: true,
  lineNumbers: true,
  wordWrap: false,
})

const emit = defineEmits<{
  (e: 'update:modelValue', value: string): void
  (e: 'change', value: string): void
}>()

// Internal value management
const internalValue = computed({
  get: () => props.modelValue || props.code,
  set: (val) => {
    emit('update:modelValue', val)
    emit('change', val)
  }
})

// Monaco language mapping (common aliases to Monaco language IDs)
const languageMap: Record<string, string> = {
  // JavaScript/TypeScript
  'js': 'javascript',
  'ts': 'typescript',
  'jsx': 'javascript',
  'tsx': 'typescript',
  'mjs': 'javascript',
  'cjs': 'javascript',
  
  // Systems languages
  'rs': 'rust',
  'c': 'c',
  'h': 'c',
  'cc': 'cpp',
  'cpp': 'cpp',
  'cxx': 'cpp',
  'c++': 'cpp',
  'hpp': 'cpp',
  'hxx': 'cpp',
  
  // Python
  'py': 'python',
  'pyw': 'python',
  'pyi': 'python',
  
  // Shell
  'sh': 'shell',
  'bash': 'shell',
  'zsh': 'shell',
  'ps1': 'powershell',
  
  // Web
  'htm': 'html',
  'vue': 'html',
  'scss': 'scss',
  'sass': 'scss',
  'less': 'less',
  
  // Data/Config
  'yml': 'yaml',
  'toml': 'ini',
  'ini': 'ini',
  'jsonc': 'json',
  
  // Other languages
  'rb': 'ruby',
  'go': 'go',
  'java': 'java',
  'kt': 'kotlin',
  'swift': 'swift',
  'cs': 'csharp',
  'php': 'php',
  'lua': 'lua',
  'sql': 'sql',
  'graphql': 'graphql',
  'gql': 'graphql',
  'md': 'markdown',
  'dockerfile': 'dockerfile',
}

const monacoLanguage = computed(() => {
  const lang = props.language?.toLowerCase() || ''
  return languageMap[lang] || lang || 'plaintext'
})

// Calculate height based on content
const computedHeight = computed(() => {
  if (props.height) {
    return typeof props.height === 'number' ? `${props.height}px` : props.height
  }
  
  const content = internalValue.value
  const lineCount = (content.match(/\n/g) || []).length + 1
  const lineHeight = 19
  const padding = 16
  const calculatedHeight = lineCount * lineHeight + padding
  
  return `${Math.min(Math.max(calculatedHeight, props.minHeight), props.maxHeight)}px`
})

// Editor options
const editorOptions = computed(() => ({
  readOnly: props.readOnly,
  minimap: { enabled: false },
  lineNumbers: props.lineNumbers ? 'on' : 'off',
  lineNumbersMinChars: 3,
  glyphMargin: false,
  folding: false,
  lineDecorationsWidth: 0,
  scrollBeyondLastLine: false,
  renderLineHighlight: props.readOnly ? 'none' : 'line',
  hideCursorInOverviewRuler: true,
  overviewRulerBorder: false,
  overviewRulerLanes: 0,
  scrollbar: {
    vertical: 'auto',
    horizontal: 'auto',
    verticalScrollbarSize: 8,
    horizontalScrollbarSize: 8,
  },
  wordWrap: props.wordWrap ? 'on' : 'off',
  fontSize: 13,
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  fontLigatures: true,
  padding: { top: 8, bottom: 8 },
  contextmenu: !props.readOnly,
  domReadOnly: props.readOnly,
  // Disable features in read-only mode
  ...(props.readOnly && {
    renderValidationDecorations: 'off',
    quickSuggestions: false,
    parameterHints: { enabled: false },
    suggestOnTriggerCharacters: false,
    acceptSuggestionOnEnter: 'off',
    tabCompletion: 'off',
    wordBasedSuggestions: 'off',
  }),
}))

// Copy functionality
const copied = ref(false)
async function copyCode() {
  try {
    await navigator.clipboard.writeText(internalValue.value)
    copied.value = true
    setTimeout(() => { copied.value = false }, 2000)
  } catch (err) {
    console.error('Failed to copy:', err)
  }
}
</script>

<style>
@reference "../assets/css/main.css";

.code-block-container {
  @apply relative rounded-lg overflow-hidden border border-nanna-primary/30 bg-nanna-bg-surface;
}

.code-block-editable {
  @apply border-nanna-primary/50;
}

.code-block-header {
  @apply flex items-center justify-between px-3 py-1.5 bg-nanna-bg-elevated/50 border-b border-nanna-primary/20;
}

.code-lang {
  @apply text-[10px] uppercase tracking-wider text-nanna-text-dim font-medium;
}

.copy-btn {
  @apply p-1 rounded text-nanna-text-dim hover:text-nanna-text hover:bg-nanna-primary/20 transition-colors;
}

.monaco-wrapper {
  @apply w-full;
}

/* Override Monaco's default background */
.monaco-wrapper :deep(.monaco-editor),
.monaco-wrapper :deep(.monaco-editor-background),
.monaco-wrapper :deep(.monaco-editor .margin) {
  background-color: #1e293b !important;
}
</style>
