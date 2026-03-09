<template>
  <NodeViewWrapper class="tiptap-monaco-wrapper my-2" data-type="monacoCodeBlock">
    <div class="monaco-code-block rounded-lg overflow-hidden"
      @mouseenter="splatterEnter(); glassEnter()"
      @mouseleave="splatterLeave(); glassLeave()"
    >
      <!-- Header with language selector (ground glass) -->
      <div class="monaco-header" :style="glassStyle">
        <span class="monaco-header__mesh" :style="{ background: meshBg }" />
        <span class="monaco-header__noise" />
        <input
          ref="langInput"
          v-model="localLanguage"
          type="text"
          placeholder="language"
          spellcheck="false"
          class="bg-transparent text-[10px] uppercase tracking-wider text-nanna-text-dim font-medium w-24 outline-none placeholder:text-nanna-text-dim/50 relative z-1"
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

      <!-- Monaco Editor (splatter) -->
      <div class="monaco-embed" :style="{ height: computedHeight }">
        <span class="monaco-splatter" :style="{ background: splatterBg }" />
        <VueMonacoEditor
          ref="monacoRef"
          v-model:value="localCode"
          :language="monacoLanguage"
          theme="nanna-dark-transparent"
          :height="computedHeight"
          :options="editorOptions"
          @mount="handleMount"
          @change="handleCodeChange"
        />
      </div>
    </div>
  </NodeViewWrapper>
</template>

<script setup lang="ts">
import { ref, computed, watch } from 'vue'
import { NodeViewWrapper, nodeViewProps } from '@tiptap/vue-3'
import { VueMonacoEditor } from '@guolao/vue-monaco-editor'
import { X, Copy, Check } from 'lucide-vue-next'
import { useSplatter } from '~/composables/useSplatter'
import { useGroundGlass } from '~/composables/useGroundGlass'
import type * as Monaco from 'monaco-editor'

const props = defineProps(nodeViewProps)

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

const monacoRef = ref()
const langInput = ref()
const copied = ref(false)
let editorInstance: Monaco.editor.IStandaloneCodeEditor | null = null

// Local state — initialised from node attrs (atom node, no textContent)
const localCode = ref(props.node.attrs.content || '')
const localLanguage = ref(props.node.attrs.language || '')

// Watch for external attribute changes (e.g. undo/redo, collaboration)
watch(() => props.node.attrs.language, (v) => {
  if (v !== localLanguage.value) localLanguage.value = v || ''
})

watch(() => props.node.attrs.content, (v) => {
  if (v !== localCode.value) localCode.value = v ?? ''
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
  const height = Math.min(350, Math.max(5 * 19 + 8, lineCount * 19 + 8))
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
  lineHeight: 19,
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  fontLigatures: true,
  automaticLayout: true,
  padding: { top: 4, bottom: 4 },
}))

function handleMount(editor: Monaco.editor.IStandaloneCodeEditor, monaco: typeof Monaco) {
  editorInstance = editor

  // Fix line number alignment — Tailwind preflight shifts content area down.
  const container = editor.getDomNode()
  if (container) {
    const s = document.createElement('style')
    s.textContent = `.margin-view-overlays { transform: translateY(1rem) !important; }`
    container.appendChild(s)
    // Force margin background transparent (Monaco dynamically sets it)
    const fixBg = () => {
      const m = container.querySelector('.margin') as HTMLElement
      if (m) m.style.backgroundColor = 'transparent'
    }
    fixBg()
    editor.onDidChangeConfiguration(fixBg)
    requestAnimationFrame(fixBg)
    setTimeout(fixBg, 100)
  }

  // Auto-focus when created empty (e.g. via ``` input rule)
  if (!localCode.value) {
    setTimeout(() => editor.focus(), 50)
  }

  // Escape to blur and return to TipTap
  editor.addCommand(monaco.KeyCode.Escape, () => {
    editor.blur()
    props.editor.commands.focus()
  })
}

// Sync Monaco content → TipTap node attribute (no ProseMirror transactions)
function handleCodeChange(value: string) {
  props.updateAttributes({ content: value })
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

.monaco-header {
  @apply flex items-center justify-between px-3 py-1.5;
  position: relative;
  isolation: isolate;
  overflow: hidden;
  background: transparent;
}

.monaco-header__mesh {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
}

.monaco-header__noise {
  position: absolute;
  inset: 0;
  z-index: 1;
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

.monaco-header > *:not(.monaco-header__mesh):not(.monaco-header__noise) {
  position: relative;
  z-index: 2;
}

.monaco-embed {
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



/* Ensure Monaco captures keyboard events */
.monaco-embed :deep(.monaco-editor textarea) {
  @apply outline-none;
}
</style>
