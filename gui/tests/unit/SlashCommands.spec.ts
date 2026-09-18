import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import type { Editor } from '@tiptap/vue-3'
import { nextTick } from 'vue'
import ChatInput from '~/components/ChatInput.vue'
import RichTextEditor from '~/components/RichTextEditor.vue'

// Unmount even when an assertion fails: a leftover editor would leave its menu
// in document.body for the next test to find.
enableAutoUnmount(afterEach)

/**
 * The chat composer's slash menu, against the REAL Tiptap editor. The extension
 * used to be loaded with `require` inside a silent catch; the ESM build has no
 * `require`, so it never registered and "/" did nothing, with no error logged.
 */
const stubs = { FloatingToolbar: true, NuiIcon: true, NuiKbd: true, MarkdownContent: true }

const mountComposer = async (props: Record<string, unknown> = {}) => {
  const wrapper = mount(ChatInput, {
    props: { modelValue: '', ...props },
    attachTo: document.body,
    global: { components: { RichTextEditor }, stubs },
  })
  await flushPromises()
  const editor = wrapper.findComponent(RichTextEditor).vm.editor as Editor
  expect(editor).toBeTruthy()
  return { wrapper, editor }
}

const extensionNames = (editor: Editor) => editor.extensionManager.extensions.map(extension => extension.name)
// What ProseMirror dispatches for typed characters. The suggestion plugin
// fetches the menu's items asynchronously, so let that settle.
const type = async (editor: Editor, text: string) => {
  editor.view.dispatch(editor.view.state.tr.insertText(text))
  await flushPromises()
}
const press = (editor: Editor, key: string, modifiers: KeyboardEventInit = {}) => editor.view.dom.dispatchEvent(
  new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...modifiers }),
)
const menu = () => document.querySelector<HTMLElement>('.slash-menu')
const menuShown = () => Boolean(menu()) && menu()!.style.display !== 'none'
const menuLabels = () => [...document.querySelectorAll('.slash-menu-item__label')].map(label => label.textContent)
const blockTypes = (editor: Editor) => editor.getJSON().content?.map(node => node.type)

describe('slash commands in the chat composer', () => {
  it('registers the extension when the prop is set', async () => {
    const { editor } = await mountComposer()
    expect(extensionNames(editor)).toContain('slashCommands')
  })

  it('leaves it out when the prop is not set', async () => {
    const wrapper = mount(RichTextEditor, { attachTo: document.body, global: { stubs } })
    await flushPromises()
    expect(extensionNames(wrapper.vm.editor as Editor)).not.toContain('slashCommands')
  })

  it('opens the menu on "/" and Enter applies the highlighted command', async () => {
    const { wrapper, editor } = await mountComposer()
    await type(editor, '/')
    // In the document and visible: the popup used to mount before the items
    // arrived and stayed an empty box.
    expect(menuShown()).toBe(true)
    expect(menuLabels()).toContain('Heading 1')

    await type(editor, 'head')
    expect(menuLabels()).toEqual(['Heading 1', 'Heading 2', 'Heading 3'])
    press(editor, 'ArrowDown')
    await nextTick()
    expect(document.querySelector('.slash-menu-item--active')?.textContent).toContain('Heading 2')

    press(editor, 'Enter')
    await flushPromises()
    expect(editor.getJSON().content?.[0]).toMatchObject({ type: 'heading', attrs: { level: 2 } })
    expect(editor.getText()).not.toContain('/head')
    expect(menu()).toBeNull()
    expect(wrapper.emitted('submit')).toBeUndefined()
  })

  it('closes the menu on Escape without stopping the running turn', async () => {
    const { wrapper, editor } = await mountComposer({ isActive: true })
    await type(editor, '/')
    expect(menuShown()).toBe(true)

    press(editor, 'Escape')
    await flushPromises()
    expect(menu()).toBeNull()
    expect(wrapper.emitted('stop')).toBeUndefined()

    // With the menu closed, Escape is the stop shortcut again.
    press(editor, 'Escape')
    expect(wrapper.emitted('stop')).toHaveLength(1)
  })

  it('lets Enter through when the text after "/" matches no command', async () => {
    const { editor } = await mountComposer()
    await type(editor, 'see /usr')
    expect(menuShown()).toBe(false)

    press(editor, 'Enter')
    expect(blockTypes(editor)).toEqual(['paragraph', 'paragraph'])
  })

  it('still sends on Ctrl+Enter with the menu open', async () => {
    const { wrapper, editor } = await mountComposer()
    await type(editor, 'see /head')
    expect(menuShown()).toBe(true)

    press(editor, 'Enter', { ctrlKey: true })
    expect(wrapper.emitted('submit')).toHaveLength(1)
    expect(blockTypes(editor)).not.toContain('heading')
  })
})
