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
    invoke.mockReturnValue(new Promise(() => {})) // never resolves
    const wrapper = mount(BackendStatus)
    expect(wrapper.text()).toContain('Loading...')
    expect(wrapper.get('.status-indicator').text()).toBe('⏳')
    expect(wrapper.classes()).toContain('status-loading')
  })

  it('displays daemon mode when connected', async () => {
    invoke.mockResolvedValue(mockStatus())
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledWith('get_backend_status'))
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('DAEMON')
    expect(wrapper.get('.status-indicator').text()).toBe('🔗')
    expect(wrapper.classes()).toContain('status-daemon')
  })

  it('displays embedded mode', async () => {
    invoke.mockResolvedValue(mockStatus({ mode: 'embedded', connected: false }))
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('EMBEDDED')
    expect(wrapper.get('.status-indicator').text()).toBe('📱')
    expect(wrapper.classes()).toContain('status-embedded')
  })

  it('displays disconnected state when daemon not connected', async () => {
    invoke.mockResolvedValue(mockStatus({ connected: false, daemon_state: 'stopped' }))
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('DISCONNECTED')
    expect(wrapper.get('.status-indicator').text()).toBe('🔌')
    expect(wrapper.classes()).toContain('status-disconnected')
  })

  it('shows detail with daemon URL when showDetail is true', async () => {
    invoke.mockResolvedValue(mockStatus())
    const wrapper = mount(BackendStatus, { props: { showDetail: true } })
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('127.0.0.1:5149')
  })

  it('hides detail when showDetail is false', async () => {
    invoke.mockResolvedValue(mockStatus())
    const wrapper = mount(BackendStatus, { props: { showDetail: false } })
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).not.toContain('127.0.0.1:5149')
  })

  it('shows daemon state as detail when disconnected', async () => {
    invoke.mockResolvedValue(mockStatus({ connected: false, daemon_state: 'crashed' }))
    const wrapper = mount(BackendStatus, { props: { showDetail: true } })
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('crashed')
  })

  it('shows Direct mode for embedded detail', async () => {
    invoke.mockResolvedValue(mockStatus({ mode: 'embedded', connected: false }))
    const wrapper = mount(BackendStatus, { props: { showDetail: true } })
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.text()).toContain('Direct mode')
  })

  it('sets correct tooltip for daemon connected', async () => {
    invoke.mockResolvedValue(mockStatus())
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.backend-status').attributes('title')).toContain('Connected to daemon at ws://127.0.0.1:5149')
  })

  it('sets correct tooltip for embedded mode', async () => {
    invoke.mockResolvedValue(mockStatus({ mode: 'embedded', connected: false }))
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.backend-status').attributes('title')).toContain('Running in embedded mode')
  })

  it('sets correct tooltip for disconnected', async () => {
    invoke.mockResolvedValue(mockStatus({ connected: false, daemon_state: 'stopped' }))
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(wrapper.get('.backend-status').attributes('title')).toContain('Daemon state: stopped')
  })

  it('polls for status every 5 seconds', async () => {
    invoke.mockResolvedValue(mockStatus())
    const wrapper = mount(BackendStatus)
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
    // invoke should not be called again after unmount
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('handles fetch error gracefully', async () => {
    invoke.mockRejectedValue(new Error('Connection failed'))
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {})
    const wrapper = mount(BackendStatus)
    await vi.waitFor(() => expect(invoke).toHaveBeenCalled())
    await wrapper.vm.$nextTick()

    expect(consoleError).toHaveBeenCalledWith('Failed to fetch backend status:', expect.any(Error))
    expect(wrapper.text()).toContain('Unknown')
    expect(wrapper.get('.status-indicator').text()).toBe('❓')

    consoleError.mockRestore()
  })

  it('applies status-loading class while loading', () => {
    invoke.mockReturnValue(new Promise(() => {}))
    const wrapper = mount(BackendStatus)
    expect(wrapper.classes()).toContain('status-loading')
  })
})
