<script setup lang="ts">
/**
 * MemoryEditor — wraps RichTextEditor for memory card viewing/editing.
 * Read-only by default; toggle editable for inline editing.
 * Shows a compact formatting toolbar when editable.
 */
import {
  Bold, Italic, Strikethrough, Code,
  Heading2, List, ListOrdered, Quote, ListChecks, Minus,
} from 'lucide-vue-next'
import { useSplatter } from '~/composables/useSplatter'

const props = withDefaults(defineProps<{
  modelValue: string
  editable?: boolean
  placeholder?: string
}>(), {
  editable: false,
  placeholder: '',
})

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const editorRef = ref<any>(null)
const tiptapEditor = computed(() => editorRef.value?.editor)

const { splatterBg: toolbarSplatterBg, onEnter: toolbarEnter, onLeave: toolbarLeave } = useSplatter({
  colors: ['139,92,246', '99,102,241', '71,85,105'] as [string, string, string],
  opacityRanges: [[0.08, 0.12], [0.06, 0.10], [0.04, 0.08]] as [[number, number], [number, number], [number, number]],
  sizes: ['65%', '60%', '50%'] as [string, string, string],
})

function focus() {
  editorRef.value?.focus()
}

defineExpose({ focus })
</script>

<template>
  <div class="memory-editor" :class="{ 'memory-editor--editable': editable }">
    <!-- Formatting toolbar (edit mode only) -->
    <div
      v-if="editable && tiptapEditor"
      class="memory-toolbar"
      @mouseenter="toolbarEnter"
      @mouseleave="toolbarLeave"
    >
      <span class="memory-toolbar__splatter" :style="{ background: toolbarSplatterBg }" />
      <button
        @click="tiptapEditor.chain().focus().toggleBold().run()"
        :class="{ active: tiptapEditor.isActive('bold') }"
        class="memory-toolbar__btn" title="Bold"
      ><Bold class="w-3.5 h-3.5" /></button>
      <button
        @click="tiptapEditor.chain().focus().toggleItalic().run()"
        :class="{ active: tiptapEditor.isActive('italic') }"
        class="memory-toolbar__btn" title="Italic"
      ><Italic class="w-3.5 h-3.5" /></button>
      <button
        @click="tiptapEditor.chain().focus().toggleStrike().run()"
        :class="{ active: tiptapEditor.isActive('strike') }"
        class="memory-toolbar__btn" title="Strikethrough"
      ><Strikethrough class="w-3.5 h-3.5" /></button>
      <button
        @click="tiptapEditor.chain().focus().toggleCode().run()"
        :class="{ active: tiptapEditor.isActive('code') }"
        class="memory-toolbar__btn" title="Inline code"
      ><Code class="w-3.5 h-3.5" /></button>
      <span class="memory-toolbar__divider" />
      <button
        @click="tiptapEditor.chain().focus().toggleHeading({ level: 2 }).run()"
        :class="{ active: tiptapEditor.isActive('heading') }"
        class="memory-toolbar__btn" title="Heading"
      ><Heading2 class="w-3.5 h-3.5" /></button>
      <button
        @click="tiptapEditor.chain().focus().toggleBulletList().run()"
        :class="{ active: tiptapEditor.isActive('bulletList') }"
        class="memory-toolbar__btn" title="Bullet list"
      ><List class="w-3.5 h-3.5" /></button>
      <button
        @click="tiptapEditor.chain().focus().toggleOrderedList().run()"
        :class="{ active: tiptapEditor.isActive('orderedList') }"
        class="memory-toolbar__btn" title="Numbered list"
      ><ListOrdered class="w-3.5 h-3.5" /></button>
      <button
        @click="tiptapEditor.chain().focus().toggleTaskList().run()"
        :class="{ active: tiptapEditor.isActive('taskList') }"
        class="memory-toolbar__btn" title="Task list"
      ><ListChecks class="w-3.5 h-3.5" /></button>
      <button
        @click="tiptapEditor.chain().focus().toggleBlockquote().run()"
        :class="{ active: tiptapEditor.isActive('blockquote') }"
        class="memory-toolbar__btn" title="Blockquote"
      ><Quote class="w-3.5 h-3.5" /></button>
      <button
        @click="tiptapEditor.chain().focus().setHorizontalRule().run()"
        class="memory-toolbar__btn" title="Horizontal rule"
      ><Minus class="w-3.5 h-3.5" /></button>
    </div>

    <RichTextEditor
      ref="editorRef"
      :model-value="modelValue"
      :editable="editable"
      :placeholder="placeholder"
      @update:model-value="emit('update:modelValue', $event)"
    />
  </div>
</template>

<style scoped>
.memory-editor {
  position: relative;
  max-height: 200px;
  overflow-y: auto;
  overflow-x: hidden;
}

.memory-editor--editable {
  max-height: 400px;
}

/* ═══ Toolbar ═══ */
.memory-toolbar {
  position: relative;
  isolation: isolate;
  display: flex;
  align-items: center;
  gap: 2px;
  padding: 4px 8px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.04);
  background: rgba(15, 23, 42, 0.3);
  flex-shrink: 0;
}

.memory-toolbar__splatter {
  position: absolute;
  inset: 0;
  z-index: 0;
  pointer-events: none;
  -webkit-mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  mask-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='200' height='200'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.65' numOctaves='3' stitchTiles='stitch' result='noise'/%3E%3CfeColorMatrix type='saturate' values='0' in='noise' result='gray'/%3E%3CfeColorMatrix type='matrix' in='gray' values='0 0 0 0 1 0 0 0 0 1 0 0 0 0 1 1 0 0 0 0' result='a'/%3E%3CfeComponentTransfer in='a'%3E%3CfeFuncA type='linear' slope='0.9' intercept='0.05'/%3E%3C/feComponentTransfer%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)'/%3E%3C/svg%3E");
  -webkit-mask-size: 200px 200px;
  mask-size: 200px 200px;
}

.memory-toolbar__btn {
  position: relative;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 4px;
  border: none;
  background: transparent;
  color: rgba(196, 205, 214, 0.5);
  cursor: pointer;
  transition: all 0.15s ease;
}

.memory-toolbar__btn:hover {
  color: rgba(226, 232, 240, 0.9);
  background: rgba(255, 255, 255, 0.06);
}

.memory-toolbar__btn.active {
  color: rgba(167, 139, 250, 1);
  background: rgba(139, 92, 246, 0.15);
}

.memory-toolbar__divider {
  position: relative;
  z-index: 1;
  width: 1px;
  height: 16px;
  margin: 0 4px;
  background: rgba(71, 85, 105, 0.3);
  flex-shrink: 0;
}

/* Scrollbar */
.memory-editor::-webkit-scrollbar { width: 6px; }
.memory-editor::-webkit-scrollbar-track { background: transparent; }
.memory-editor::-webkit-scrollbar-thumb { background: rgba(139, 92, 246, 0.2); border-radius: 3px; }
.memory-editor::-webkit-scrollbar-thumb:hover { background: rgba(139, 92, 246, 0.3); }
</style>
