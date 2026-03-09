<template>
  <div class="code-block-container" :class="{ 'code-block-editable': !readOnly }"
    @mouseenter="splatterEnter(); glassEnter()"
    @mouseleave="splatterLeave(); glassLeave()"
  >
    <!-- Language label + copy button (ground glass) -->
    <div v-if="language || showCopy" class="code-block-header" :style="glassStyle">
      <span class="code-block-header__mesh" :style="{ background: meshBg }" />
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
    
    <!-- Monaco Editor (splatter) -->
    <div class="monaco-wrapper" :style="{ height: computedHeight }">
      <span class="monaco-splatter" :style="{ background: splatterBg }" />
      <VueMonacoEditor
        v-model:value="internalValue"
        :language="monacoLanguage"
        theme="nanna-dark"
        :options="editorOptions"
        @mount="handleMount"
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

// Splatter effect for the editor content area
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

// Ground glass effect for the toolbar
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
  const padding = 8
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
  lineHeight: 19,
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  fontLigatures: true,
  padding: { top: 4, bottom: 4 },
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

// Fix line number alignment — Tailwind preflight shifts content area down.
function handleMount(editor: any) {
  const container = editor.getDomNode()
  if (container) {
    const s = document.createElement('style')
    s.textContent = `.margin-view-overlays { transform: translateY(1rem) !important; }`
    const fixBg = () => {
      const m = container.querySelector('.margin') as HTMLElement
      if (m) m.style.backgroundColor = 'transparent'
    }
    fixBg()
    requestAnimationFrame(fixBg)
    setTimeout(fixBg, 100)
    container.appendChild(s)
  }
}

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
  @apply relative rounded-lg overflow-hidden border border-white/[0.08] bg-nanna-bg-surface;
}

.code-block-editable {
  @apply border-nanna-primary/50;
}

.code-block-header {
  @apply flex items-center justify-between px-3 py-1.5 border-b border-white/[0.06];
  position: relative;
  isolation: isolate;
  overflow: hidden;
  background: rgba(30, 41, 59, 0.25);
  border-top: 1px solid rgba(255, 255, 255, 0.04);
}

.code-block-header__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
}

.code-block-header > *:not(.code-block-header__mesh) {
  position: relative;
  z-index: 1;
}

.code-lang {
  @apply text-[10px] uppercase tracking-wider text-nanna-text-dim font-medium;
}

.copy-btn {
  @apply p-1 rounded text-nanna-text-dim hover:text-nanna-text hover:bg-nanna-primary/20 transition-colors;
}

.monaco-wrapper {
  @apply w-full;
  position: relative;
  isolation: isolate;
}

.monaco-splatter {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  transition: opacity 0.4s ease;
  opacity: 0.5;
}



/* Override Monaco's default background */
.monaco-wrapper :deep(.monaco-editor),
.monaco-wrapper :deep(.monaco-editor-background) {
  background-color: #1e293b !important;
}

.monaco-wrapper :deep(.monaco-editor .margin) {
  background-color: transparent !important;
}
</style>
