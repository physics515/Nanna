# Phase 7: Rich Input & Editor Experience

**Status:** ❌ Not Started

## Overview

Replace the plain `<textarea>` chat input with a Tiptap-based rich editor that supports markdown formatting, Monaco-powered code blocks, slash commands, and the Nanna Palenight aesthetic. This editor will also be reused for system prompt editing, memory editing, and workspace file editing.

## Architecture

```
gui/app/
├── components/
│   ├── editor/
│   │   ├── NannaEditor.vue        # Main Tiptap editor component
│   │   ├── EditorToolbar.vue      # Formatting toolbar
│   │   ├── FloatingToolbar.vue    # Selection-based floating toolbar
│   │   ├── SlashMenu.vue          # Slash command menu
│   │   ├── MonacoCodeBlock.vue    # Monaco-powered code block node
│   │   └── EditorTheme.ts         # Palenight theme tokens
│   ├── ChatInput.vue              # Updated to use NannaEditor
│   └── SystemPromptEditor.vue     # Updated to use NannaEditor
├── extensions/
│   ├── MonacoCodeBlock.ts         # (existing, 140 lines) — Tiptap node extension
│   └── SlashCommands.ts           # Slash command extension
```

## Current State

### What Exists

**`ChatInput.vue` (340 lines):**
- Standard `<textarea>` with auto-resize
- Shift+Enter for newlines, Enter to send
- File attachment button (images, PDFs)
- Character count display
- No formatting support

**`MonacoBlock.vue` (236 lines):**
- Vue component wrapping Monaco editor
- Language detection and syntax highlighting
- Copy button
- Used in message rendering (output), not input

**`TiptapMonacoBlock.vue` (216 lines):**
- Tiptap node view wrapper for Monaco
- Bridges Tiptap's node system with Monaco editor
- Language selector dropdown
- Already partially implemented but not wired into chat input

**`MonacoCodeBlock.ts` (140 lines):**
- Tiptap extension defining the Monaco code block node
- `addNodeView()` returns the TiptapMonacoBlock component
- Handles content updates between Tiptap and Monaco
- Already exists as an extension

**`monaco.client.ts` (107 lines):**
- Nuxt client plugin that initializes Monaco
- Registers Palenight-inspired theme
- Configures default editor options

### What's Missing

- No Tiptap editor component for input
- No formatting toolbar
- No slash commands
- No floating toolbar on selection
- No markdown shortcuts (type `**bold**` → auto-format)
- No image paste/drag-drop in editor
- No integration with chat input flow

## Implementation Plan

### Step 1: Basic Tiptap Editor Component

```vue
<!-- components/editor/NannaEditor.vue -->
<template>
  <div class="nanna-editor" :class="{ focused: isFocused }">
    <EditorToolbar v-if="showToolbar" :editor="editor" />
    <EditorContent :editor="editor" class="editor-content" />
    <div class="editor-footer">
      <span class="char-count">{{ charCount }}</span>
      <slot name="actions" />
    </div>
  </div>
</template>
```

**Dependencies:**
```json
{
  "@tiptap/vue-3": "^2.x",
  "@tiptap/starter-kit": "^2.x",
  "@tiptap/extension-placeholder": "^2.x",
  "@tiptap/extension-typography": "^2.x",
  "@tiptap/extension-link": "^2.x",
  "@tiptap/extension-image": "^2.x",
  "@tiptap/extension-table": "^2.x",
  "@tiptap/extension-code-block-lowlight": "^2.x",
  "@tiptap/extension-task-list": "^2.x",
  "@tiptap/extension-task-item": "^2.x"
}
```

**Key behaviors:**
- Enter sends message (like current textarea)
- Shift+Enter creates newline
- Markdown shortcuts: `**bold**`, `*italic*`, `` `code` ``, `# heading`, `> quote`, `- list`
- Output: Markdown string (for LLM) — use `@tiptap/extension-markdown` or custom serializer
- Placeholder: "Ask Nanna anything..."

### Step 2: Theme Integration

The editor needs to match the Palenight aesthetic:

```css
.nanna-editor {
  background: theme('colors.slate.800');
  border: 1px solid theme('colors.slate.700');
  border-radius: 0.5rem;
  font-family: 'JetBrains Mono', 'Fira Code', monospace;
}

.nanna-editor.focused {
  border-color: theme('colors.violet.500');
  box-shadow: 0 0 0 2px theme('colors.violet.500/20');
}

/* Prose styling */
.editor-content :deep(.ProseMirror) {
  color: theme('colors.slate.200');
  min-height: 2.5rem;
  max-height: 20rem;
  overflow-y: auto;
  padding: 0.75rem;
}

.editor-content :deep(.ProseMirror p.is-editor-empty:first-child::before) {
  color: theme('colors.slate.500');
  content: attr(data-placeholder);
  float: left;
  pointer-events: none;
}

/* Code blocks */
.editor-content :deep(pre) {
  background: theme('colors.slate.900');
  border: 1px solid theme('colors.slate.700');
  border-radius: 0.375rem;
  padding: 0.75rem;
}

/* Inline code */
.editor-content :deep(code) {
  background: theme('colors.slate.700');
  color: theme('colors.cyan.400');
  padding: 0.125rem 0.25rem;
  border-radius: 0.25rem;
  font-size: 0.875em;
}

/* Selection */
.editor-content :deep(.ProseMirror ::selection) {
  background: theme('colors.violet.500/30');
}
```

### Step 3: Code Block Extension

The existing `MonacoCodeBlock.ts` extension is a good foundation. Enhancements needed:

- **Language auto-detection** — Use first line heuristics or `highlight.js` auto-detect
- **Inline code vs block** — Single backtick for inline, triple backtick for block
- **Tab handling** — Tab inserts spaces in code blocks, not focus change
- **Copy button** — Already exists in MonacoBlock, wire into Tiptap node view

For the input editor, consider using `lowlight` (lightweight) instead of full Monaco to keep the editor responsive. Reserve Monaco for the rendered output and dedicated code editing contexts.

### Step 4: Slash Commands

```typescript
// extensions/SlashCommands.ts
const commands = [
  { name: 'heading1', label: 'Heading 1', icon: 'H1', action: (editor) => editor.chain().focus().toggleHeading({ level: 1 }).run() },
  { name: 'heading2', label: 'Heading 2', icon: 'H2', action: (editor) => editor.chain().focus().toggleHeading({ level: 2 }).run() },
  { name: 'bullet', label: 'Bullet List', icon: '•', action: (editor) => editor.chain().focus().toggleBulletList().run() },
  { name: 'numbered', label: 'Numbered List', icon: '1.', action: (editor) => editor.chain().focus().toggleOrderedList().run() },
  { name: 'code', label: 'Code Block', icon: '<>', action: (editor) => editor.chain().focus().toggleCodeBlock().run() },
  { name: 'quote', label: 'Blockquote', icon: '"', action: (editor) => editor.chain().focus().toggleBlockquote().run() },
  { name: 'divider', label: 'Divider', icon: '—', action: (editor) => editor.chain().focus().setHorizontalRule().run() },
  { name: 'task', label: 'Task List', icon: '☑', action: (editor) => editor.chain().focus().toggleTaskList().run() },
  { name: 'image', label: 'Image', icon: '🖼', action: () => { /* open file picker */ } },
  { name: 'table', label: 'Table', icon: '⊞', action: (editor) => editor.chain().focus().insertTable({ rows: 3, cols: 3 }).run() },
];
```

### Step 5: Floating Toolbar

Appears when text is selected:
- Bold, Italic, Strikethrough, Code, Link
- Positioned above selection using Tiptap's `BubbleMenu`

```vue
<BubbleMenu :editor="editor" :tippy-options="{ duration: 100 }">
  <div class="floating-toolbar">
    <button @click="editor.chain().focus().toggleBold().run()" :class="{ active: editor.isActive('bold') }">B</button>
    <button @click="editor.chain().focus().toggleItalic().run()" :class="{ active: editor.isActive('italic') }">I</button>
    <!-- ... -->
  </div>
</BubbleMenu>
```

### Step 6: Chat Integration

Replace the textarea in `ChatInput.vue`:

```vue
<NannaEditor
  v-model="message"
  placeholder="Ask Nanna anything..."
  :show-toolbar="showToolbar"
  @submit="sendMessage"
  @attach="handleAttachment"
>
  <template #actions>
    <button @click="sendMessage" class="send-button">
      <SendIcon />
    </button>
  </template>
</NannaEditor>
```

**Output format:** The editor stores rich content internally but serializes to Markdown when sending to the LLM. The `getMarkdown()` method converts the Tiptap document to a Markdown string.

### Step 7: Reuse in Other Contexts

- **System Prompt Editor** — Replace the current textarea in settings
- **Memory Editor** — Rich editing when updating memory content
- **Workspace Files** — Edit SOUL.md, USER.md, AGENTS.md with live preview

## Issues & Considerations

### Performance

Tiptap + Monaco together can be heavy. The chat input needs to be snappy.

**Mitigation:**
- Use `lowlight` for code highlighting in the input editor (not full Monaco)
- Lazy-load Tiptap extensions
- Only mount Monaco for dedicated code editing (tools page, workspace files)
- Debounce markdown serialization

### Markdown Fidelity

The editor's internal representation (ProseMirror) doesn't map 1:1 to Markdown. Some edge cases:
- Nested lists with mixed types
- Complex table formatting
- Raw HTML in markdown

**Mitigation:**
- Use `@tiptap/extension-markdown` for serialization
- Test with common markdown patterns
- Fall back to raw text mode if needed

### Mobile Experience

Tiptap works on mobile but the toolbar needs adaptation:
- Bottom-anchored toolbar instead of top
- Larger touch targets
- Simplified formatting options

### Draft Persistence

Save editor content to localStorage so users don't lose drafts on page navigation or accidental close.

```typescript
// Auto-save draft every 2 seconds
watch(editorContent, useDebounceFn((content) => {
  localStorage.setItem(`draft-${sessionId}`, content)
}, 2000))
```

## Dependencies to Install

```bash
cd gui
pnpm add @tiptap/vue-3 @tiptap/starter-kit @tiptap/extension-placeholder @tiptap/extension-typography @tiptap/extension-link @tiptap/extension-image @tiptap/extension-table @tiptap/extension-code-block-lowlight @tiptap/extension-task-list @tiptap/extension-task-item lowlight
```

## Potential Enhancements

1. **Vim mode** — Optional vim keybindings via Tiptap extension
2. **Collaborative editing** — Multiple GUI instances editing same workspace file
3. **Emoji picker** — Quick emoji insertion
4. **Mention system** — @agent to invoke specific agents
5. **File tree sidebar** — Browse workspace files from editor
6. **Split preview** — Side-by-side editor + rendered markdown
7. **Template snippets** — Reusable prompt templates
8. **Voice input** — Whisper transcription directly into editor
