import { Node, mergeAttributes, textblockTypeInputRule } from '@tiptap/core'
import { VueNodeViewRenderer } from '@tiptap/vue-3'
import TiptapMonacoBlock from '../components/TiptapMonacoBlock.vue'

export interface MonacoCodeBlockOptions {
  HTMLAttributes: Record<string, any>
}

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    monacoCodeBlock: {
      setMonacoCodeBlock: (attributes?: { language?: string }) => ReturnType
      toggleMonacoCodeBlock: (attributes?: { language?: string }) => ReturnType
      insertMonacoCodeBlock: (attributes?: { language?: string }) => ReturnType
    }
  }
}

export const MonacoCodeBlock = Node.create<MonacoCodeBlockOptions>({
  name: 'monacoCodeBlock',

  group: 'block',

  content: 'text*',

  marks: '',

  defining: true,

  isolating: true,

  code: true,

  addOptions() {
    return {
      HTMLAttributes: {},
    }
  },

  addAttributes() {
    return {
      language: {
        default: '',
        parseHTML: element => element.getAttribute('data-language'),
        renderHTML: attributes => ({
          'data-language': attributes.language,
        }),
      },
    }
  },

  parseHTML() {
    return [
      {
        tag: 'pre',
        preserveWhitespace: 'full',
      },
    ]
  },

  renderHTML({ node, HTMLAttributes }) {
    return [
      'pre',
      mergeAttributes(this.options.HTMLAttributes, HTMLAttributes, { 
        'data-language': node.attrs.language 
      }),
      ['code', 0],
    ]
  },

  addNodeView() {
    return VueNodeViewRenderer(TiptapMonacoBlock)
  },

  addCommands() {
    return {
      setMonacoCodeBlock:
        (attributes) =>
        ({ commands }) => {
          return commands.setNode(this.name, attributes)
        },
      toggleMonacoCodeBlock:
        (attributes) =>
        ({ commands }) => {
          return commands.toggleNode(this.name, 'paragraph', attributes)
        },
      insertMonacoCodeBlock:
        (attributes) =>
        ({ chain, state }) => {
          const { selection } = state
          const { $from } = selection
          
          // Check if we're already in a code block
          if ($from.parent.type.name === 'monacoCodeBlock') {
            return false
          }
          
          // If cursor is at the end of text, just insert after
          // Otherwise, split the paragraph and insert
          return chain()
            .insertContent([
              { type: 'paragraph' }, // Ensure we're on a new line
              { type: 'monacoCodeBlock', attrs: attributes || { language: '' } },
              { type: 'paragraph' }, // Add paragraph after for continued typing
            ])
            .focus()
            .run()
        },
    }
  },

  addKeyboardShortcuts() {
    return {
      // Ctrl/Cmd + Alt + C to insert code block at cursor
      'Mod-Alt-c': () => this.editor.commands.insertMonacoCodeBlock(),
      // Tab in code block inserts tab
      'Tab': () => {
        if (this.editor.isActive(this.name)) {
          return this.editor.commands.insertContent('\t')
        }
        return false
      },
    }
  },

  addInputRules() {
    return [
      // Match ```language at start of paragraph, followed by space
      textblockTypeInputRule({
        find: /^```([a-zA-Z0-9_+-]*)[\s]$/,
        type: this.type,
        getAttributes: match => ({
          language: match[1] || '',
        }),
      }),
    ]
  },
})

export default MonacoCodeBlock
