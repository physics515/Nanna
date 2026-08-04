import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import SettingsAgentTab from '~/components/settings/SettingsAgentTab.vue'
import { useSettingsPage } from '~/composables/useSettingsPage'

/**
 * Owner directive 2026-08-04: "thinking should be on by default and remove the
 * option in settings to turn it off."
 *
 * The switch that used to sit in Response Preferences was worse than an unused
 * control — the settings payload never carried `thinking_enabled`, so it
 * always rendered OFF while the agent was (per config) thinking. This asserts
 * it is gone and, just as importantly, that no call site is left invoking the
 * deleted `set_thinking_enabled` command (an unregistered Tauri command fails
 * at runtime, not at build time — see invokeCommands.spec.ts).
 */

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

vi.mock('~/composables/useSettingsPage', () => ({
  useSettingsPage: vi.fn(),
}))

const settings = ref<Record<string, unknown>>({
  agent_name: 'Nanna',
  personality_mode: 'balanced',
  max_tokens: 4096,
  streaming_enabled: true,
})
const loadSettings = vi.fn()
const showToast = vi.fn()
const onSettingsLoaded = vi.fn()

const mockedUseSettingsPage = vi.mocked(useSettingsPage)

const UiSwitchStub = defineComponent({
  name: 'UiSwitch',
  props: {
    modelValue: { type: Boolean, default: false },
    label: { type: String, default: '' },
  },
  emits: ['update:modelValue'],
  setup(props, { emit }) {
    return () =>
      h(
        'button',
        {
          type: 'button',
          class: 'switch',
          'aria-label': props.label,
          onClick: () => emit('update:modelValue', !props.modelValue),
        },
        String(props.modelValue),
      )
  },
})

function mountTab() {
  return mount(SettingsAgentTab, {
    global: {
      stubs: {
        UiSwitch: UiSwitchStub,
        UiInput: { template: '<input />' },
        UiSelect: { template: '<select />' },
        UiButton: { template: '<button><slot /></button>' },
        SettingsSection: { template: '<section><slot /></section>' },
        SystemPromptEditor: { template: '<div />' },
        Bot: true,
        Cpu: true,
        MessageSquare: true,
        Trash2: true,
        Plus: true,
      },
    },
  })
}

describe('SettingsAgentTab', () => {
  beforeEach(() => {
    invoke.mockReset()
    invoke.mockResolvedValue(undefined)
    loadSettings.mockReset()
    showToast.mockReset()
    onSettingsLoaded.mockReset()
    mockedUseSettingsPage.mockReturnValue({
      settings,
      showAdvanced: ref(true),
      routingModelOptions: ref([]),
      loadSettings,
      showToast,
      onSettingsLoaded,
    } as unknown as ReturnType<typeof useSettingsPage>)
  })

  it('renders no thinking switch — thinking is always on', () => {
    const wrapper = mountTab()
    const labels = wrapper.findAll('.switch').map((s) => s.attributes('aria-label'))
    expect(labels).not.toContain('Thinking enabled')
    expect(wrapper.text()).not.toContain('Thinking Mode')
    // The section it lived in is still there, with its other controls.
    expect(wrapper.text()).toContain('Streaming')
  })

  it('never invokes the deleted set_thinking_enabled command', async () => {
    const wrapper = mountTab()
    for (const toggle of wrapper.findAll('.switch')) {
      await toggle.trigger('click')
    }
    const invoked = invoke.mock.calls.map((call) => call[0])
    expect(invoked).not.toContain('set_thinking_enabled')
    // Sanity: the surviving switches DO still reach the backend, so the
    // assertion above is about absence, not about a dead component.
    expect(invoked).toContain('set_streaming_enabled')
  })
})
