import { Node, mergeAttributes, InputRule } from '@tiptap/core'
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

  // Atom node — no ProseMirror contentDOM. Monaco is the sole editor.
  // Code content lives in the `content` attribute, not in PM's text layer.
  atom: true,

  defining: true,

  isolating: true,

  addOptions() {
    return {
      HTMLAttributes: {},
    }
  },

  addAttributes() {
    return {
      language: {
        default: '',
        parseHTML: element => element.getAttribute('data-language') || '',
        renderHTML: attributes => ({
          'data-language': attributes.language,
        }),
      },
      content: {
        default: '',
        parseHTML: element => {
          const code = element.querySelector('code')
          return (code || element).textContent || ''
        },
        renderHTML: () => ({}), // rendered in renderHTML body
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
        'data-language': node.attrs.language,
      }),
      ['code', {}, node.attrs.content || ''],
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
        ({ chain }) => {
          return chain()
            .insertContent([
              {
                type: 'monacoCodeBlock',
                attrs: { language: '', content: '', ...attributes },
              },
              { type: 'paragraph' },
            ])
            .focus()
            .run()
        },
    }
  },

  addKeyboardShortcuts() {
    return {
      'Mod-Alt-c': () => this.editor.commands.insertMonacoCodeBlock(),
    }
  },

  addInputRules() {
    return [
      // Replace ```language + whitespace with an empty code block
      new InputRule({
        find: /^```([a-zA-Z0-9_+-]*)[\s]$/,
        handler: ({ state, range, match }) => {
          const language = match[1] || ''
          const node = this.type.create({ language, content: '' })
          const tr = state.tr.delete(range.from, range.to)
          tr.insert(range.from, node)
          // Add a paragraph after for continued typing
          const paragraphPos = range.from + node.nodeSize
          tr.insert(paragraphPos, state.schema.nodes.paragraph.create())
        },
      }),
    ]
  },
})

export default MonacoCodeBlock
