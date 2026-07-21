import { mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import ChatInput from '~/components/ChatInput.vue'

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
    const send = wrapper.findAllComponents(ButtonStub).find(button => button.text().includes('Send'))!
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
