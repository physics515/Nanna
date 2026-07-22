import { config } from '@vue/test-utils'
import { defineComponent, h } from 'vue'

class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}

vi.stubGlobal('ResizeObserver', ResizeObserverMock)
vi.stubGlobal('requestAnimationFrame', (callback: FrameRequestCallback) => {
  callback(0)
  return 0
})
vi.stubGlobal('cancelAnimationFrame', () => {})

/**
 * Nuxt auto-import aliases (`UiSpinner`, `UiGlassButton`, …) are not resolved by
 * plain Vitest. Stub them globally so component unit tests stay hermetic.
 */
const UiSpinnerStub = defineComponent({
  name: 'UiSpinner',
  props: { class: { type: String, default: '' }, size: { type: String, default: 'default' } },
  setup(_props, { attrs }) {
    return () => h('div', { ...attrs, 'data-testid': 'ui-spinner', role: 'status', 'aria-busy': 'true' })
  },
})

const UiGlassButtonStub = defineComponent({
  name: 'UiGlassButton',
  props: {
    disabled: { type: Boolean, default: false },
    size: { type: String, default: 'sm' },
    color: { type: String, default: 'default' },
    pill: { type: Boolean, default: false },
    type: { type: String, default: 'button' },
  },
  emits: ['click'],
  setup(props, { slots, emit, attrs }) {
    return () =>
      h(
        'button',
        {
          ...attrs,
          type: props.type,
          disabled: props.disabled || undefined,
          'data-testid': 'ui-glass-button',
          onClick: (e: Event) => emit('click', e),
        },
        slots.default?.(),
      )
  },
})

config.global.stubs = {
  Teleport: true,
  UiSpinner: UiSpinnerStub,
  UiGlassButton: UiGlassButtonStub,
}
