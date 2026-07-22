import { mount } from '@vue/test-utils'
import BackendStatus from '~/components/BackendStatus.vue'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))

describe('BackendStatus', () => {
  beforeEach(() => {
    invoke.mockReset()
    vi.useFakeTimers()
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  const mockStatus = (overrides = {}) => ({
    mode: 'daemon' as const,
    connected: true,
    daemon_url: 'ws://127.0.0.1:5149',
    daemon_state: 'running',
    version: '0.1.0',
    ...overrides,
  })

  it('shows loading state initially', () => {
    invoke.mockReturnValue(new Promise(() => {}))
    const wrapper = mount(BackendStatus)
    // calm copy via describeBackend — not all-caps legacy
    expect(wrapper.text()).toMatch(/Checking|Loading/i)
    expect(wrapper.get('.status-indicator').text()).toBe('⏳')
    expect(wrapper.classes()).toContain('status-loading')
  })

  it('displays daemon mode when connected', async () => {
    invoke.mockResolvedValue(mockStatus())
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('get_backend_status'))
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Daemon')
    expect(wrapper.get('.status-indicator').text()).toBe('🔗')
    expect(wrapper.classes()).toContain('status-daemon')
  })

  it('labels retired embedded path without claiming live data', async () => {
    invoke.mockResolvedValue(mockStatus({ mode: 'embedded', connected: false }))
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toMatch(/Legacy|embedded/i)
    // should not claim connected-daemon
    expect(wrapper.classes()).not.toContain('status-daemon')
  })

  it('never shows bare DISCONNECTED next to offline state', async () => {
    invoke.mockResolvedValue(mockStatus({ connected: false, daemon_state: 'stopped' }))
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text().toUpperCase()).not.toContain('DISCONNECTED')
    expect(wrapper.text()).toMatch(/offline|not reachable|stopped/i)
    expect(wrapper.get('.status-indicator').text()).toBe('🔌')
    expect(wrapper.classes()).toContain('status-disconnected')
  })

  it('shows detail with daemon host when showDetail is true', async () => {
    invoke.mockResolvedValue(mockStatus())
    const wrapper = mount(BackendStatus, { props: { showDetail: true } })
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('127.0.0.1:5149')
  })

  it('shows crashed detail when offline + showDetail', async () => {
    invoke.mockResolvedValue(mockStatus({ connected: false, daemon_state: 'crashed' }))
    const wrapper = mount(BackendStatus, { props: { showDetail: true } })
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toMatch(/crash/i)
  })

  it('tooltip names the IPC endpoint when connected', async () => {
    invoke.mockResolvedValue(mockStatus())
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    const title = wrapper.get('.backend-status').attributes('title') || ''
    expect(title).toMatch(/5149/)
    expect(title.toLowerCase()).toMatch(/attach|daemon|connected/)
  })

  it('polls for status every 5 seconds', async () => {
    invoke.mockResolvedValue(mockStatus())
    mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1))

    vi.advanceTimersByTime(5000)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(2))

    vi.advanceTimersByTime(5000)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(3))
  })

  it('clears poll interval on unmount', async () => {
    invoke.mockResolvedValue(mockStatus())
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())

    wrapper.unmount()
    vi.advanceTimersByTime(5000)
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('handles fetch error gracefully', async () => {
    invoke.mockRejectedValue(new Error('Connection failed'))
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(consoleError).toHaveBeenCalled()
    // unknown / offline, not a false "Connected"
    expect(wrapper.text().toLowerCase()).not.toContain('connected')
    consoleError.mockRestore()
  })
})
