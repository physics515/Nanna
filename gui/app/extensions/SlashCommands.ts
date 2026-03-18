import { Extension } from '@tiptap/core'
import Suggestion from '@tiptap/suggestion'
import type { SuggestionOptions } from '@tiptap/suggestion'
import tippy, { type Instance as TippyInstance } from 'tippy.js'
import { VueRenderer } from '@tiptap/vue-3'
import SlashMenu from '~/components/SlashMenu.vue'

export interface SlashCommandItem {
  name: string
  label: string
  icon: string
  description: string
  action: (editor: any) => void
}

export const slashCommands: SlashCommandItem[] = [
  {
    name: 'heading1',
    label: 'Heading 1',
    icon: 'H1',
    description: 'Large heading',
    action: (editor) => editor.chain().focus().toggleHeading({ level: 1 }).run(),
  },
  {
    name: 'heading2',
    label: 'Heading 2',
    icon: 'H2',
    description: 'Medium heading',
    action: (editor) => editor.chain().focus().toggleHeading({ level: 2 }).run(),
  },
  {
    name: 'heading3',
    label: 'Heading 3',
    icon: 'H3',
    description: 'Small heading',
    action: (editor) => editor.chain().focus().toggleHeading({ level: 3 }).run(),
  },
  {
    name: 'bullet',
    label: 'Bullet List',
    icon: '•',
    description: 'Unordered list',
    action: (editor) => editor.chain().focus().toggleBulletList().run(),
  },
  {
    name: 'numbered',
    label: 'Numbered List',
    icon: '1.',
    description: 'Ordered list',
    action: (editor) => editor.chain().focus().toggleOrderedList().run(),
  },
  {
    name: 'task',
    label: 'Task List',
    icon: '☑',
    description: 'Checklist with toggles',
    action: (editor) => editor.chain().focus().toggleTaskList().run(),
  },
  {
    name: 'code',
    label: 'Code Block',
    icon: '<>',
    description: 'Monaco-powered code editor',
    action: (editor) => editor.chain().focus().insertMonacoCodeBlock().run(),
  },
  {
    name: 'quote',
    label: 'Blockquote',
    icon: '"',
    description: 'Quote block',
    action: (editor) => editor.chain().focus().toggleBlockquote().run(),
  },
  {
    name: 'divider',
    label: 'Divider',
    icon: '—',
    description: 'Horizontal rule',
    action: (editor) => editor.chain().focus().setHorizontalRule().run(),
  },
  {
    name: 'image',
    label: 'Image',
    icon: '🖼',
    description: 'Insert image from URL',
    action: (editor) => {
      const url = window.prompt('Image URL:')
      if (url) editor.chain().focus().setImage({ src: url }).run()
    },
  },
]

export const SlashCommands = Extension.create({
  name: 'slashCommands',

  addOptions() {
    return {
      suggestion: {
        char: '/',
        startOfLine: false,
        command: ({ editor, range, props }: { editor: any; range: any; props: SlashCommandItem }) => {
          // Delete the slash trigger text
          editor.chain().focus().deleteRange(range).run()
          // Execute the command
          props.action(editor)
        },
        items: ({ query }: { query: string }) => {
          return slashCommands.filter((item) =>
            item.label.toLowerCase().includes(query.toLowerCase()) ||
            item.name.toLowerCase().includes(query.toLowerCase())
          ).slice(0, 10)
        },
        render: () => {
          let component: VueRenderer | null = null
          let popup: TippyInstance[] | null = null

          return {
            onStart: (props: any) => {
              component = new VueRenderer(SlashMenu, {
                props,
                editor: props.editor,
              })

              if (!props.clientRect) return

              popup = tippy('body', {
                getReferenceClientRect: props.clientRect,
                appendTo: () => document.body,
                content: component.element,
                showOnCreate: true,
                interactive: true,
                trigger: 'manual',
                placement: 'bottom-start',
                theme: 'nanna-slash',
              })
            },
            onUpdate: (props: any) => {
              component?.updateProps(props)
              if (popup?.[0] && props.clientRect) {
                popup[0].setProps({
                  getReferenceClientRect: props.clientRect,
                })
              }
            },
            onKeyDown: (props: any) => {
              if (props.event.key === 'Escape') {
                popup?.[0]?.hide()
                return true
              }
              return (component?.ref as any)?.onKeyDown?.(props.event) ?? false
            },
            onExit: () => {
              popup?.[0]?.destroy()
              component?.destroy()
            },
          }
        },
      } as Partial<SuggestionOptions>,
    }
  },

  addProseMirrorPlugins() {
    return [
      Suggestion({
        editor: this.editor,
        ...this.options.suggestion,
      }),
    ]
  },
})

export default SlashCommands
