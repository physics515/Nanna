<template>
  <div class="markdown-content prose prose-invert prose-sm max-w-none">
    <template v-for="(block, idx) in blocks" :key="idx">
      <!-- Code block -->
      <MonacoBlock
        v-if="block.type === 'code'"
        :code="block.content"
        :language="block.language"
        :read-only="true"
        :max-height="350"
        class="my-3"
      />
      <!-- Regular markdown content -->
      <div 
        v-else 
        v-html="block.content"
      />
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { renderMarkdown } from '~/lib/markdown'

const props = defineProps<{
  content: string
}>()

interface ContentBlock {
  type: 'text' | 'code'
  content: string
  language?: string
}

// Parse content into blocks (text and code)
const blocks = computed<ContentBlock[]>(() => {
  const content = props.content || ''
  const result: ContentBlock[] = []
  
  // Regex to match fenced code blocks
  const codeBlockRegex = /```(\w*)\n([\s\S]*?)```/g
  
  let lastIndex = 0
  let match
  
  while ((match = codeBlockRegex.exec(content)) !== null) {
    // Add text before this code block
    if (match.index > lastIndex) {
      const textContent = content.slice(lastIndex, match.index)
      const html = renderMarkdown(textContent)
      if (html.trim()) {
        result.push({ type: 'text', content: html })
      }
    }
    
    // Add code block
    result.push({
      type: 'code',
      content: match[2].trim(),
      language: match[1] || undefined,
    })
    
    lastIndex = match.index + match[0].length
  }
  
  // Add remaining text after last code block
  if (lastIndex < content.length) {
    const textContent = content.slice(lastIndex)
    const html = renderMarkdown(textContent)
    if (html.trim()) {
      result.push({ type: 'text', content: html })
    }
  }
  
  // If no code blocks found, return entire content as text
  if (result.length === 0 && content.trim()) {
    result.push({ type: 'text', content: renderMarkdown(content) })
  }
  
  return result
})

// Rendering lives in `~/lib/markdown`: everything below is untrusted (assistant
// output, user input, and whatever a tool returned that the assistant quoted),
// and it goes straight into `v-html`. That module escapes author HTML and
// scheme-checks link/image URLs so this component cannot hand parsed markup to
// the webview. Fenced code blocks never reach it — they are split out above and
// rendered by Monaco.
</script>

<style>
@reference "../assets/css/main.css";

.markdown-content {
  @apply break-words;
}

/* Ensure inline code still looks good */
.markdown-content :deep(code) {
  @apply px-1.5 py-0.5 rounded glass-tag text-nanna-secondary font-mono text-xs;
}

/* Don't apply inline styles to Monaco code */
.markdown-content :deep(.monaco-editor code) {
  @apply p-0 bg-transparent;
}
</style>
