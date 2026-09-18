import { flushPromises, mount } from '@vue/test-utils'

/**
 * The splash's ways out, through the real close handler: Close, the window's
 * own close (Alt+F4, the taskbar) and Quit must end the app in every phase,
 * including the one before Rust has managed the app's state, when every
 * command that reads it is refused. That phase lasts as long as Config::load's
 * keyring read, which a locked Secret Service holds open while its unlock
 * prompt is up.
 */
const { invoke, exit, appWindow } = vi.hoisted(() => ({
  invoke: vi.fn(),
  exit: vi.fn(),
  appWindow: {
    isMaximized: vi.fn(),
    onResized: vi.fn(),
    onCloseRequested: vi.fn(),
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    hide: vi.fn(),
    destroy: vi.fn(),
  },
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => appWindow }))
vi.mock('@tauri-apps/plugin-process', () => ({ exit: (...args: unknown[]) => exit(...args) }))
vi.mock('~/composables/useAppUpdater', async () => {
  const { ref: vueRef } = await import('vue')
  return {
    useAppUpdater: () => ({
      updateVersion: vueRef<string | null>(null),
      checking: vueRef(false),
      updating: vueRef(false),
      applyUpdate: vi.fn(),
    }),
  }
})

const stubs = {
  NuiLogo: { template: '<img src="/logo.svg" alt="Nanna" />' },
  NuiIcon: { template: '<span />' },
  NuiKbd: { props: ['keys'], template: '<kbd>{{ keys }}</kbd>' },
  NuiWindowControls: {
    emits: ['close'],
    template: '<div><button aria-label="Close" @click="$emit(\'close\')" /></div>',
  },
  CloseDialog: { template: '<div />' },
}

const NOT_MANAGED = 'state not managed'

const calls = (cmd: string) => invoke.mock.calls.filter(([c]) => c === cmd).length

type CloseRequested = (event: { preventDefault: () => void }) => Promise<void>

async function mountSplash() {
  vi.resetModules()
  const { default: StartupSplash } = await import('~/components/StartupSplash.vue')
  const wrapper = mount(StartupSplash, { global: { stubs }, attachTo: document.body })
  await flushPromises()
  return wrapper
}

describe('StartupSplash close and quit', () => {
  let wrapper: Awaited<ReturnType<typeof mountSplash>> | null = null
  let onCloseRequested: CloseRequested | null = null

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] })
    invoke.mockReset()
    exit.mockReset()
    exit.mockResolvedValue(undefined)
    appWindow.destroy.mockReset()
    appWindow.isMaximized.mockResolvedValue(false)
    appWindow.onResized.mockResolvedValue(() => {})
    onCloseRequested = null
    appWindow.onCloseRequested.mockImplementation(async (handler: CloseRequested) => {
      onCloseRequested = handler
      return () => {}
    })
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    wrapper?.unmount()
    wrapper = null
    vi.useRealTimers()
  })

  describe('before the app state is managed', () => {
    beforeEach(() => {
      invoke.mockRejectedValue(NOT_MANAGED)
    })

    it('ends the app from the close button', async () => {
      wrapper = await mountSplash()
      await wrapper.get('button[aria-label="Close"]').trigger('click')
      await flushPromises()
      expect(exit).toHaveBeenCalledWith(0)
      expect(appWindow.destroy).not.toHaveBeenCalled()
    })

    it("ends the app on the window's own close (Alt+F4)", async () => {
      wrapper = await mountSplash()
      expect(onCloseRequested).not.toBeNull()
      const event = { preventDefault: vi.fn() }
      await onCloseRequested!(event)
      await flushPromises()
      expect(event.preventDefault).toHaveBeenCalled()
      expect(exit).toHaveBeenCalledWith(0)
    })

    it('ends the app from Quit', async () => {
      wrapper = await mountSplash()
      const quit = wrapper.findAll('button').find((b) => b.text() === 'Quit')
      await quit!.trigger('click')
      await flushPromises()
      expect(exit).toHaveBeenCalledWith(0)
    })
  })

  it('quits once when the saved preference is "quit"', async () => {
    invoke.mockImplementation(async (cmd: string) => (cmd === 'handle_window_close' ? 'quit' : null))
    wrapper = await mountSplash()
    await wrapper.get('button[aria-label="Close"]').trigger('click')
    await flushPromises()
    expect(calls('perform_quit')).toBe(1)
    expect(exit).not.toHaveBeenCalled()
  })
})
