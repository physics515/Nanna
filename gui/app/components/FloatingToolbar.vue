<template>
  <BubbleMenu
    :editor="editor"
    :tippy-options="{ duration: 150, theme: 'nanna-bubble', maxWidth: 'none' }"
    :should-show="shouldShow"
    v-if="editor"
  >
    <div class="floating-toolbar">
      <button
        @click="editor.chain().focus().toggleBold().run()"
        :class="{ active: editor.isActive('bold') }"
        class="ft-btn"
        title="Bold (Ctrl+B)"
      >
        <Bold class="w-3.5 h-3.5" />
      </button>
      <button
        @click="editor.chain().focus().toggleItalic().run()"
        :class="{ active: editor.isActive('italic') }"
        class="ft-btn"
        title="Italic (Ctrl+I)"
      >
        <Italic class="w-3.5 h-3.5" />
      </button>
      <button
        @click="editor.chain().focus().toggleStrike().run()"
        :class="{ active: editor.isActive('strike') }"
        class="ft-btn"
        title="Strikethrough"
      >
        <Strikethrough class="w-3.5 h-3.5" />
      </button>
      <button
        @click="editor.chain().focus().toggleCode().run()"
        :class="{ active: editor.isActive('code') }"
        class="ft-btn"
        title="Inline Code (Ctrl+E)"
      >
        <Code class="w-3.5 h-3.5" />
      </button>
      <span class="ft-divider" />
      <button
        @click="setLink"
        :class="{ active: editor.isActive('link') }"
        class="ft-btn"
        title="Link"
      >
        <LinkIcon class="w-3.5 h-3.5" />
      </button>
      <button
        v-if="editor.isActive('link')"
        @click="editor.chain().focus().unsetLink().run()"
        class="ft-btn ft-btn--danger"
        title="Remove Link"
      >
        <Unlink class="w-3.5 h-3.5" />
      </button>
    </div>
  </BubbleMenu>
</template>

<script setup lang="ts">
import { BubbleMenu } from '@tiptap/vue-3'
import { Bold, Italic, Strikethrough, Code, Link as LinkIcon, Unlink } from 'lucide-vue-next'
import type { Editor } from '@tiptap/core'

const props = defineProps<{
  editor: Editor
}>()

function shouldShow({ editor, state }: { editor: Editor; state: any }) {
  const { from, to } = state.selection
  // Don't show on empty selections or inside code blocks
  if (from === to) return false
  if (editor.isActive('monacoCodeBlock')) return false
  if (editor.isActive('codeBlock')) return false
  return true
}

function setLink() {
  const previousUrl = props.editor.getAttributes('link').href
  const url = window.prompt('URL:', previousUrl)
  if (url === null) return
  if (url === '') {
    props.editor.chain().focus().extendMarkRange('link').unsetLink().run()
    return
  }
  props.editor.chain().focus().extendMarkRange('link').setLink({ href: url }).run()
}
</script>

<style>
@reference "../assets/css/main.css";

.floating-toolbar {
  @apply flex items-center gap-0.5 p-1 rounded-lg;
  background: rgba(15, 23, 42, 0.95);
  border: 1px solid rgba(71, 85, 105, 0.3);
  backdrop-filter: blur(12px);
  box-shadow: 0 4px 20px rgba(0, 0, 0, 0.4);
}

.ft-btn {
  @apply flex items-center justify-center w-7 h-7 rounded transition-colors;
  color: rgba(203, 213, 225, 0.8);
}

.ft-btn:hover {
  background: rgba(99, 102, 241, 0.15);
  color: #e2e8f0;
}

.ft-btn.active {
  background: rgba(99, 102, 241, 0.25);
  color: rgba(165, 180, 252, 1);
}

.ft-btn--danger:hover {
  background: rgba(239, 68, 68, 0.15);
  color: rgba(252, 165, 165, 1);
}

.ft-divider {
  @apply w-px h-4 mx-0.5;
  background: rgba(71, 85, 105, 0.4);
}

/* Tippy theme */
.tippy-box[data-theme~='nanna-bubble'] {
  background: transparent;
  border: none;
  box-shadow: none;
}

.tippy-box[data-theme~='nanna-bubble'] > .tippy-content {
  padding: 0;
}

.tippy-box[data-theme~='nanna-bubble'] > .tippy-arrow {
  display: none;
}
</style>
