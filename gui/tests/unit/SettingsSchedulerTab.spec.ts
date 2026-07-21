import { mount } from '@vue/test-utils'
import { describe, it, expect, vi } from 'vitest'
import { ref, computed } from 'vue'
import { useSettings } from '~/composables/useSettings'

vi.mock('~/composables/useSettings')
const settings = ref({ scheduler_enabled: true, heartbeat_enabled: true, heartbeat_interval_seconds: 300 })
const { useSettingsPage } = vi.mocked(useSettings, true)

useSettingsPage.mockReturnValue({ settings, loadSettings: vi.fn(), showToast: vi.fn() })

const SwitchStub = { props: ['modelValue'], emits: ['update:modelValue'], template: '<button class="switch" @click="$emit('update:modelValue', !modelValue)">{{ modelValue }}</button>' }
const mountTab = () => mount(SettingsSchedulerTab, { global: { stubs: { UiCard: { template: '<section><slot /></section>' }, UiSwitch: SwitchStub } } })

describe('SettingsSchedulerTab', () => {
  beforeEach(() => {
    invoke.mockReset()
    invoke.mockResolvedValue(undefined)
    const loadSettingsFn = useSettingsPage().loadSettings
    loadSettingsFn.mockReset()
    useSettingsPage().showToast.mockReset()
  })

  it('updates scheduler enabled state and refreshes settings', async () => {
    const wrapper = mountTab()
    await wrapper.findAll('.switch')[0].trigger('click')
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('set_scheduler_enabled', { enabled: false }))
    expect(useSettingsPage().loadSettings).toHaveBeenCalled()
  })

  it('updates heartbeat enabled state', async () => {
    const wrapper = mountTab()
    await wrapper.findAll('.switch')[1].trigger('click')
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('set_heartbeat_enabled', { enabled: false }))
  })

  it('persists the selected heartbeat interval', async () => {
    const wrapper = mountTab()
    await wrapper.get('input[type="range"]').setValue('600')
    await wrapper.get('input[type="range"]').trigger('change')
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('set_heartbeat_interval', { seconds: 600 }))
  })

  it('surfaces daemon failures through the settings toast', async () => {
    invoke.mockRejectedValue(new Error('not connected'))
    const wrapper = mountTab()
    await wrapper.findAll('.switch')[0].trigger('click')
    await vi.waitFor(() => expect(useSettingsPage().showToast).toHaveBeenCalledWith('Failed: not connected', 'error'))
  })
})
