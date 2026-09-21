import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import type { Editor } from '@tiptap/vue-3'
import { defineComponent, nextTick } from 'vue'
import ChatInput from '~/components/ChatInput.vue'
import RichTextEditor from '~/components/RichTextEditor.vue'

vi.mock('~/composables/useSplatter', () => ({ useSplatter: () => ({ splatterBg: '', onEnter: vi.fn(), onLeave: vi.fn() }) }))
vi.mock('~/composables/useGroundGlass', () => ({ useGroundGlass: () => ({ glassStyle: {} }) }))

const RichTextEditorStub = defineComponent({
  props: ['modelValue', 'disabled', 'editable'], emits: ['update:modelValue', 'submit', 'keydown'],
  setup(props, { emit, expose }) {
    expose({ isEmpty: !String(props.modelValue || '').trim(), clear: () => {} })
    return { emit }
  },
  template: `<textarea data-test="editor" :value="modelValue" :disabled="disabled" @input="emit('update:modelValue', $event.target.value)" />`,
})
const ButtonStub = defineComponent({ props: ['disabled'], emits: ['click'], template: `<button :disabled="disabled" @click="$emit('click')"><slot /></button>` })
const mountInput = (props: Record<string, unknown> = {}) => mount(ChatInput, {
  props: { modelValue: '', ...props },
  global: { stubs: { RichTextEditor: RichTextEditorStub, UiGlassButton: ButtonStub, UiIconButton: ButtonStub, MarkdownContent: true, UiShortcutBadge: true, UiTooltip: { template: '<span><slot /></span>' } } },
})

describe('ChatInput', () => {
  it('submits non-empty content from Send', async () => {
    const wrapper = mountInput({ modelValue: 'Ask the moon' })
    const send = wrapper.findAll('button').find(button => button.text().includes('Send'))!
    await send.trigger('click')
    expect(wrapper.emitted('submit')).toHaveLength(1)
  })

  it('does not submit blank content', async () => {
    const wrapper = mountInput({ modelValue: '   ' })
    const send = wrapper.findAll('button').find(button => button.text().includes('Send'))!
    expect(send.attributes('disabled')).toBeDefined(); await send.trigger('click')
    expect(wrapper.emitted('submit')).toBeUndefined()
  })

  it('shows Stop while active and emits stop', async () => {
    const wrapper = mountInput({ modelValue: 'running', isActive: true })
    const stop = wrapper.findAll('button').find(button => button.text().includes('Stop'))!
    await stop.trigger('click'); expect(wrapper.emitted('stop')).toHaveLength(1)
  })

  it('forwards editor updates to v-model', async () => {
    const wrapper = mountInput(); await wrapper.get('[data-test="editor"]').setValue('new value')
    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['new value'])
  })
})

/**
 * The Send gate against the REAL Tiptap editor. @tiptap/vue-3 keeps editor
 * state in a debounced ref: the new state is stored synchronously, but Vue is
 * only told about it two animation frames later. Anything cached over it (a
 * `computed(() => editor.isEmpty)`) keeps answering "empty" until those frames
 * run — so Ctrl+Enter pressed right after typing was silently dropped (the
 * intermittent critical-path e2e failure), and where frames never run (the
 * tauri-webdriver window on Linux) Send never enabled at all. Frames are
 * stubbed to never fire here so the tests can only pass if the gate reads the
 * editor's live document.
 */
describe('ChatInput with the real editor', () => {
  let realRequestAnimationFrame: typeof requestAnimationFrame
  beforeEach(() => {
    realRequestAnimationFrame = globalThis.requestAnimationFrame
    globalThis.requestAnimationFrame = () => 0
  })
  afterEach(() => { globalThis.requestAnimationFrame = realRequestAnimationFrame })

  const mountWithEditor = async () => {
    const wrapper = mount(ChatInput, {
      props: { modelValue: '' },
      attachTo: document.body,
      global: {
        components: { RichTextEditor },
        stubs: { FloatingToolbar: true, NuiIcon: true, NuiKbd: true, MarkdownContent: true },
      },
    })
    await flushPromises()
    const editor = wrapper.findComponent(RichTextEditor).vm.editor as Editor
    expect(editor).toBeTruthy()
    return { wrapper, editor }
  }
  // What ProseMirror dispatches for typed characters.
  const type = (editor: Editor, text: string) => editor.view.dispatch(editor.view.state.tr.insertText(text))
  const pressCtrlEnter = (editor: Editor) => editor.view.dom.dispatchEvent(
    new KeyboardEvent('keydown', { key: 'Enter', ctrlKey: true, bubbles: true, cancelable: true }),
  )
  const sendButton = (wrapper: VueWrapper) => wrapper.findAll('button').find(button => button.attributes('title') === 'Send')!

  it('submits on Ctrl+Enter pressed immediately after typing', async () => {
    const { wrapper, editor } = await mountWithEditor()
    type(editor, 'Hello from e2e')
    pressCtrlEnter(editor)
    expect(wrapper.emitted('submit')).toHaveLength(1)
    wrapper.unmount()
  })

  it('enables Send as soon as text is typed and disables it once cleared', async () => {
    const { wrapper, editor } = await mountWithEditor()
    expect(sendButton(wrapper).attributes('disabled')).toBeDefined()

    type(editor, 'Hello')
    await nextTick()
    expect(sendButton(wrapper).attributes('disabled')).toBeUndefined()
    await sendButton(wrapper).trigger('click')
    expect(wrapper.emitted('submit')).toHaveLength(1)

    // submit() clears the editor, which must re-disable Send.
    await nextTick()
    expect(editor.isEmpty).toBe(true)
    expect(sendButton(wrapper).attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })

  // Ctrl+Enter is also StarterKit's HardBreak chord (Mod-Enter). ChatInput
  // handles it (preventDefault, submit, clear the editor); if the editor then
  // ran its own keymaps too, a hard break landed in the freshly cleared
  // composer: blank-looking, not empty, Send enabled.
  const hasHardBreak = (editor: Editor) => JSON.stringify(editor.getJSON()).includes('"hardBreak"')

  it('leaves the composer empty with Send disabled after Ctrl+Enter sends', async () => {
    const { wrapper, editor } = await mountWithEditor()
    type(editor, 'Hello from e2e')
    pressCtrlEnter(editor)
    expect(wrapper.emitted('submit')).toHaveLength(1)

    await nextTick()
    expect(hasHardBreak(editor)).toBe(false)
    expect(editor.isEmpty).toBe(true)
    expect(sendButton(wrapper).attributes('disabled')).toBeDefined()
    wrapper.unmount()
  })

  it('still lets the editor handle keys ChatInput leaves alone', async () => {
    const { wrapper, editor } = await mountWithEditor()
    type(editor, 'line one')
    editor.view.dom.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', shiftKey: true, bubbles: true, cancelable: true }))
    expect(hasHardBreak(editor)).toBe(true)
    expect(wrapper.emitted('submit')).toBeUndefined()
    wrapper.unmount()
  })

  it('does not submit an empty editor on Ctrl+Enter', async () => {
    const { wrapper, editor } = await mountWithEditor()
    pressCtrlEnter(editor)
    expect(wrapper.emitted('submit')).toBeUndefined()
    wrapper.unmount()
  })
})
