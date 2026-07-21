import { mount } from '@vue/test-utils'
import ConnectionStatus from '~/components/ConnectionStatus.vue'

describe('ConnectionStatus', () => {
  it('shows the connected state when visible', () => {
    const wrapper = mount(ConnectionStatus, { props: { status: 'connected', visible: true } })
    expect(wrapper.text()).toContain('Connected')
    expect(wrapper.classes()).toContain('status-connected')
  })

  it('shows reconnecting progress with its message', () => {
    const wrapper = mount(ConnectionStatus, { props: { status: 'reconnecting', visible: true, message: 'Attempt 3 of 10' } })
    expect(wrapper.text()).toContain('Reconnecting...')
    expect(wrapper.text()).toContain('Attempt 3 of 10')
  })

  it('emits retry and disables retry while one is running', async () => {
    const wrapper = mount(ConnectionStatus, { props: { status: 'error', visible: true, canRetry: true } })
    await wrapper.get('.retry-btn').trigger('click')
    expect(wrapper.emitted('retry')).toHaveLength(1)
    await wrapper.setProps({ isRetrying: true })
    expect(wrapper.get('.retry-btn').attributes('disabled')).toBeDefined()
  })
})
