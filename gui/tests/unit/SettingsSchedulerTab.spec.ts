import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import SettingsSchedulerTab from '~/components/settings/SettingsSchedulerTab.vue'
import { useSettingsPage } from '~/composables/useSettingsPage'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

vi.mock('~/composables/useSettingsPage', () => ({
  useSettingsPage: vi.fn(),
}))

const settings = ref({
  scheduler_enabled: true,
  heartbeat_enabled: true,
  heartbeat_interval_seconds: 300,
})
const loadSettings = vi.fn()
const showToast = vi.fn()

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
  return mount(SettingsSchedulerTab, {
    global: {
      stubs: {
        UiCard: { template: '<section><slot /></section>' },
        UiSwitch: UiSwitchStub,
        Clock: true,
      },
    },
  })
}

describe('SettingsSchedulerTab', () => {
  beforeEach(() => {
    invoke.mockReset()
    invoke.mockResolvedValue(undefined)
    loadSettings.mockReset()
    showToast.mockReset()
    settings.value = {
      scheduler_enabled: true,
      heartbeat_enabled: true,
      heartbeat_interval_seconds: 300,
    }
    mockedUseSettingsPage.mockReturnValue({
      settings,
      loadSettings,
      showToast,
    } as ReturnType<typeof useSettingsPage>)
  })

  it('updates scheduler enabled state and refreshes settings', async () => {
    const wrapper = mountTab()
    await wrapper.findAll('.switch')[0].trigger('click')
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('set_scheduler_enabled', { enabled: false }),
    )
    expect(loadSettings).toHaveBeenCalled()
  })

  it('updates heartbeat enabled state', async () => {
    const wrapper = mountTab()
    await wrapper.findAll('.switch')[1].trigger('click')
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('set_heartbeat_enabled', { enabled: false }),
    )
  })

  it('persists the selected heartbeat interval', async () => {
    const wrapper = mountTab()
    await wrapper.get('input[type="range"]').setValue('600')
    await wrapper.get('input[type="range"]').trigger('change')
    await vi.waitFor(() =>
      expect(invoke).toHaveBeenCalledWith('set_heartbeat_interval', { seconds: 600 }),
    )
  })

  it('surfaces daemon failures through the settings toast', async () => {
    invoke.mockRejectedValue(new Error('not connected'))
    const wrapper = mountTab()
    await wrapper.findAll('.switch')[0].trigger('click')
    await vi.waitFor(() =>
      expect(showToast).toHaveBeenCalledWith('Failed: not connected', 'error'),
    )
  })
})
