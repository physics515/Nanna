import { flushPromises, mount } from '@vue/test-utils'
import type { BackendStatusLike } from '~/lib/backendLabels'

const { invoke, updater, closeHandler, appWindow } = vi.hoisted(() => ({
  invoke: vi.fn(),
  updater: {
    updateVersion: { value: null as string | null },
    updating: { value: false },
    applyUpdate: vi.fn(),
  },
  closeHandler: {
    handleClose: vi.fn(),
    performQuit: vi.fn(),
  },
  appWindow: {
    isMaximized: vi.fn(),
    onResized: vi.fn(),
    onCloseRequested: vi.fn(),
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
  },
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => appWindow }))
vi.mock('~/composables/useAppUpdater', async () => {
  const { ref: vueRef } = await import('vue')
  return {
    // Read per mount, so a test sets the update it wants before mounting.
    useAppUpdater: () => ({
      currentVersion: vueRef('0.3.22'),
      updateVersion: vueRef<string | null>(updater.updateVersion.value),
      checking: vueRef(false),
      updating: vueRef(updater.updating.value),
      updateError: vueRef(null),
      checkForUpdates: vi.fn(),
      applyUpdate: updater.applyUpdate,
    }),
  }
})
vi.mock('~/composables/useCloseHandler', async () => {
  const { ref: vueRef } = await import('vue')
  const showCloseDialog = vueRef(false)
  return {
    useCloseHandler: () => ({
      showCloseDialog,
      handleClose: closeHandler.handleClose,
      performQuit: closeHandler.performQuit,
    }),
  }
})

const stubs = {
  NuiLogo: { props: ['height'], template: '<img src="/logo.svg" alt="Nanna" />' },
  NuiIcon: { props: ['name', 'size'], template: '<span />' },
  NuiKbd: { props: ['keys'], template: '<kbd>{{ keys }}</kbd>' },
  NuiWindowControls: {
    emits: ['minimize', 'maximize', 'close'],
    template:
      '<div><button aria-label="Minimize" @click="$emit(\'minimize\')" />' +
      '<button aria-label="Maximize" @click="$emit(\'maximize\')" />' +
      '<button aria-label="Close" @click="$emit(\'close\')" /></div>',
  },
  CloseDialog: { template: '<div data-testid="close-dialog" />' },
}

const baseStatus = (overrides: BackendStatusLike = {}): BackendStatusLike => ({
  mode: 'disconnected',
  connected: false,
  daemon_url: null,
  daemon_state: 'starting',
  version: '0.3.22',
  starting_for_s: 2,
  retrying: false,
  init_in_progress: true,
  last_error: null,
  ...overrides,
})

/** What the fake backend answers; a test changes it to move the boot along. */
let backend: BackendStatusLike | Error | null
let bootLog: Array<{ stream: 'stdout' | 'stderr', line: string }>

function answer(cmd: string): unknown {
  switch (cmd) {
    case 'get_backend_status':
      if (backend instanceof Error) throw backend
      return backend
    case 'get_daemon_version':
      return '0.3.22'
    case 'get_boot_log':
      return bootLog
    case 'restart_daemon':
      return null
    default:
      throw new Error('unexpected invoke: ' + cmd)
  }
}

/**
 * useBackend and useStartupGate keep module-level state (one per process),
 * so every mount loads fresh copies of them.
 */
async function mountSplash(status: BackendStatusLike | Error | null) {
  backend = status
  vi.resetModules()
  const { default: StartupSplash } = await import('~/components/StartupSplash.vue')
  const { useStartupGate } = await import('~/composables/useStartupGate')
  const wrapper = mount(StartupSplash, { global: { stubs }, attachTo: document.body })
  await flushPromises()
  return { wrapper, gate: useStartupGate() }
}

const statusLine = (wrapper: Awaited<ReturnType<typeof mountSplash>>['wrapper']) =>
  wrapper.get('[role="status"]').text()

function button(wrapper: Awaited<ReturnType<typeof mountSplash>>['wrapper'], name: string | RegExp) {
  const found = wrapper.findAll('button').find((b) =>
    typeof name === 'string' ? b.text().trim() === name : name.test(b.text()),
  )
  if (!found) throw new Error('no button ' + String(name) + ' in: ' + wrapper.text())
  return found
}

describe('StartupSplash', () => {
  let wrappers: Array<{ unmount: () => void }> = []

  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] })
    invoke.mockReset()
    invoke.mockImplementation(async (cmd: string) => answer(cmd))
    closeHandler.handleClose.mockReset()
    closeHandler.performQuit.mockReset()
    updater.applyUpdate.mockReset()
    updater.updateVersion.value = null
    updater.updating.value = false
    appWindow.isMaximized.mockResolvedValue(false)
    appWindow.onResized.mockResolvedValue(() => {})
    appWindow.onCloseRequested.mockResolvedValue(() => {})
    bootLog = []
    vi.spyOn(console, 'error').mockImplementation(() => {})
  })

  afterEach(() => {
    for (const w of wrappers) w.unmount()
    wrappers = []
    vi.useRealTimers()
  })

  async function show(status: BackendStatusLike | Error | null) {
    const mounted = await mountSplash(status)
    wrappers.push(mounted.wrapper)
    return mounted
  }

  it('is a main landmark named by its logo, with a polite status line', async () => {
    const { wrapper } = await show(baseStatus())
    const main = wrapper.get('main')
    expect(main.get('h1 img').attributes('alt')).toBe('Nanna')
    const status = wrapper.get('[role="status"]')
    expect(status.attributes('aria-live')).toBe('polite')
    for (const name of ['Open Nanna anyway', 'Show log', 'Quit']) {
      expect(() => button(wrapper, new RegExp('^' + name))).not.toThrow()
    }
  })

  it('says Nanna itself is starting while the app state is not managed yet', async () => {
    const { wrapper } = await show(new Error('state not managed'))
    expect(statusLine(wrapper)).toBe('Starting Nanna…')
    expect(wrapper.find('[role="alert"]').exists()).toBe(false)
  })

  it('shows an ordinary boot without anything to act on', async () => {
    const { wrapper } = await show(baseStatus())
    expect(statusLine(wrapper)).toBe('Starting the daemon…')
    expect(wrapper.findAll('button').some((b) => /Restart|Start the daemon/.test(b.text()))).toBe(false)
  })

  it('shows the elapsed time of a slow boot, calmly, and offers Restart', async () => {
    const { wrapper } = await show(baseStatus({ starting_for_s: 65 }))
    expect(statusLine(wrapper)).toBe('Still starting · 1m 05s')
    // The ticking clock is not announced; the phase words are.
    expect(wrapper.get('[role="status"] [aria-live="off"]').text()).toBe('· 1m 05s')
    expect(wrapper.text()).toMatch(/opens by itself as soon as it answers/)

    await button(wrapper, 'Restart the daemon').trigger('click')
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('restart_daemon')
  })

  it('shows a ready daemon the client has not attached as connecting', async () => {
    const { wrapper } = await show(baseStatus({ daemon_state: 'running', retrying: true, starting_for_s: null }))
    expect(statusLine(wrapper)).toBe('Connecting…')
    expect(wrapper.text()).toMatch(/keeps trying/)
  })

  it('announces a crash with its reason and focuses Restart', async () => {
    const { wrapper } = await show(baseStatus({
      daemon_state: 'crashed',
      starting_for_s: null,
      init_in_progress: false,
      retrying: true,
      last_error: {
        kind: 'exited_during_boot',
        message: 'Error: IPC port 127.0.0.1:5149 unavailable: address in use',
        exit_code: 1,
        signal: null,
        at_ms: 1,
      },
    }))
    expect(statusLine(wrapper)).toBe('The daemon exited while starting')
    const alert = wrapper.get('[role="alert"]')
    expect(alert.text()).toContain('Error: IPC port 127.0.0.1:5149 unavailable: address in use')
    expect(alert.text()).toContain('exit code 1')
    const restart = button(wrapper, 'Restart the daemon')
    expect(document.activeElement).toBe(restart.element)
  })

  it('says Restarting until the restart has been read back', async () => {
    const { wrapper } = await show(baseStatus({ daemon_state: 'crashed', init_in_progress: false }))
    let finish: (value: unknown) => void = () => {}
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'restart_daemon') return new Promise((resolve) => { finish = resolve })
      return answer(cmd)
    })
    await button(wrapper, 'Restart the daemon').trigger('click')
    await flushPromises()
    expect(statusLine(wrapper)).toBe('Restarting the daemon…')

    backend = baseStatus({ daemon_state: 'starting', starting_for_s: 0 })
    finish(null)
    await flushPromises()
    expect(statusLine(wrapper)).toBe('Starting the daemon…')
  })

  it('reports a restart the backend refused', async () => {
    const { wrapper } = await show(baseStatus({ daemon_state: 'crashed', init_in_progress: false }))
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'restart_daemon') throw new Error('state not managed')
      return answer(cmd)
    })
    await button(wrapper, 'Restart the daemon').trigger('click')
    await flushPromises()
    expect(wrapper.get('[role="alert"]').text()).toContain("Couldn't restart the daemon: state not managed")
  })

  it('offers Start for a daemon that is not running and not about to', async () => {
    const { wrapper } = await show(baseStatus({ daemon_state: 'stopped', init_in_progress: false, starting_for_s: null }))
    expect(statusLine(wrapper)).toBe("The daemon isn't running")
    const start = button(wrapper, 'Start the daemon')
    expect(document.activeElement).toBe(start.element)
    await start.trigger('click')
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('restart_daemon')
  })

  it('reads stopped with an init under way as starting', async () => {
    const { wrapper } = await show(baseStatus({ daemon_state: 'stopped', init_in_progress: true, starting_for_s: null }))
    expect(statusLine(wrapper)).toBe('Starting the daemon…')
  })

  it('follows the boot on its own poll and never calls init_backend', async () => {
    const { wrapper } = await show(baseStatus())
    backend = baseStatus({ starting_for_s: 40 })
    await vi.advanceTimersByTimeAsync(500)
    await flushPromises()
    expect(statusLine(wrapper)).toBe('Still starting · 40s')
    expect(invoke).not.toHaveBeenCalledWith('init_backend')

    const polls = () => invoke.mock.calls.filter(([cmd]) => cmd === 'get_backend_status').length
    wrapper.unmount()
    wrappers = []
    const before = polls()
    await vi.advanceTimersByTimeAsync(5000)
    expect(polls()).toBe(before)
  })

  it('opens Nanna anyway from the button', async () => {
    const { wrapper, gate } = await show(baseStatus())
    await button(wrapper, /^Open Nanna anyway/).trigger('click')
    expect(gate.released.value).toBe(true)
    expect(gate.releasedOffline.value).toBe(true)
  })

  it('opens Nanna anyway on Esc', async () => {
    const { gate } = await show(baseStatus())
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(gate.released.value).toBe(true)
  })

  it('shows the boot log on request, newest last, stderr tinted', async () => {
    bootLog = [
      { stream: 'stdout', line: 'Loading config' },
      { stream: 'stderr', line: 'Error: IPC port 127.0.0.1:5149 unavailable' },
    ]
    const { wrapper } = await show(baseStatus())
    expect(invoke).not.toHaveBeenCalledWith('get_boot_log')

    const toggle = button(wrapper, 'Show log')
    expect(toggle.attributes('aria-expanded')).toBe('false')
    await toggle.trigger('click')
    await flushPromises()
    expect(toggle.text()).toBe('Hide log')
    expect(toggle.attributes('aria-expanded')).toBe('true')

    const log = wrapper.get('[role="log"]')
    expect(toggle.attributes('aria-controls')).toBe(log.attributes('id'))
    const lines = log.findAll('p')
    expect(lines.map((l) => l.text())).toEqual(['Loading config', 'Error: IPC port 127.0.0.1:5149 unavailable'])
    expect(lines[1]!.classes()).toContain('text-nui-pink')
    expect(lines[0]!.classes()).not.toContain('text-nui-pink')

    // The tail keeps up while it is open, and stops being read once closed.
    bootLog = [...bootLog, { stream: 'stdout', line: 'Daemon ready' }]
    await vi.advanceTimersByTimeAsync(1000)
    await flushPromises()
    expect(wrapper.get('[role="log"]').text()).toContain('Daemon ready')

    await toggle.trigger('click')
    const reads = () => invoke.mock.calls.filter(([cmd]) => cmd === 'get_boot_log').length
    const before = reads()
    await vi.advanceTimersByTimeAsync(3000)
    expect(reads()).toBe(before)
    expect(wrapper.find('[role="log"]').exists()).toBe(false)
  })

  it('says so when there is no log to read yet', async () => {
    const { wrapper } = await show(baseStatus())
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_boot_log') throw new Error('state not managed')
      return answer(cmd)
    })
    await button(wrapper, 'Show log').trigger('click')
    await flushPromises()
    expect(wrapper.get('[role="log"]').text()).toBe("The daemon's output isn't available yet.")
  })

  it('quits', async () => {
    const { wrapper } = await show(baseStatus())
    await button(wrapper, 'Quit').trigger('click')
    expect(closeHandler.performQuit).toHaveBeenCalledTimes(1)
  })

  it('offers the update, the one way out of a version mismatch', async () => {
    updater.updateVersion.value = '0.3.23'
    const { wrapper } = await show(baseStatus({ daemon_state: 'crashed' }))
    await button(wrapper, 'Update to v0.3.23').trigger('click')
    expect(updater.applyUpdate).toHaveBeenCalledTimes(1)
  })

  it('closes through the saved close preference, which also owns quitting', async () => {
    const { wrapper } = await show(baseStatus())
    // handleClose hides, asks or quits by itself. Its old answer, true, read
    // as "quit now" here, which ran perform_quit twice in "quit" mode.
    closeHandler.handleClose.mockResolvedValue(true)
    await wrapper.get('button[aria-label="Close"]').trigger('click')
    await flushPromises()
    expect(closeHandler.handleClose).toHaveBeenCalledTimes(1)
    expect(closeHandler.performQuit).not.toHaveBeenCalled()
    expect(wrapper.find('[data-testid="close-dialog"]').exists()).toBe(true)
  })

  it("takes the window's own close (Alt+F4) the same route as the button", async () => {
    let onClose: ((event: { preventDefault: () => void }) => Promise<void>) | null = null
    appWindow.onCloseRequested.mockImplementation(async (handler) => {
      onClose = handler
      return () => {}
    })
    await show(baseStatus())
    expect(onClose).not.toBeNull()
    const event = { preventDefault: vi.fn() }
    await onClose!(event)
    // Prevented, since the JS API would otherwise destroy the window, which
    // the capabilities do not allow; handleClose then ends it.
    expect(event.preventDefault).toHaveBeenCalledTimes(1)
    expect(closeHandler.handleClose).toHaveBeenCalledTimes(1)
    expect(closeHandler.performQuit).not.toHaveBeenCalled()
  })

  it('never opens anyway while an update installs', async () => {
    // The updater stops the daemon before installing. Opening the shell then
    // runs the layout's init, which would start the old daemon mid-install.
    updater.updating.value = true
    const { wrapper, gate } = await show(baseStatus({ daemon_state: 'stopped', init_in_progress: false }))
    expect(button(wrapper, /^Open Nanna anyway/).attributes('disabled')).toBeDefined()
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    expect(gate.released.value).toBe(false)
  })

  it('says what opening anyway does: the daemon is started if it is not running', async () => {
    const { wrapper } = await show(baseStatus())
    expect(button(wrapper, /^Open Nanna anyway/).attributes('title')).toBe(
      "Settings and logs work now. Nanna starts the daemon if it isn't running, and chats work once it answers.",
    )
  })

  it('offers Restart while a running daemon does not answer', async () => {
    const { wrapper } = await show(baseStatus({ daemon_state: 'running', retrying: true, starting_for_s: null }))
    const restart = button(wrapper, 'Restart the daemon')
    expect(restart.attributes('title')).toBe('Stop the daemon and start it again')
    await restart.trigger('click')
    await flushPromises()
    expect(invoke).toHaveBeenCalledWith('restart_daemon')
  })

  it('moves focus to the status line when the focused action goes away', async () => {
    const { wrapper } = await show(baseStatus({ daemon_state: 'crashed', init_in_progress: false }))
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'restart_daemon') return new Promise(() => {})
      return answer(cmd)
    })
    const restart = button(wrapper, 'Restart the daemon')
    expect(document.activeElement).toBe(restart.element)
    await restart.trigger('click')
    await flushPromises()
    // "Restarting…" has no action to focus; <body> would lose the reader's place.
    expect(statusLine(wrapper)).toBe('Restarting the daemon…')
    expect(document.activeElement).toBe(wrapper.get('[role="status"]').element)
  })

  it('moves focus to the status line when a quiet Restart is used', async () => {
    const { wrapper } = await show(baseStatus({ starting_for_s: 65 }))
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'restart_daemon') return new Promise(() => {})
      return answer(cmd)
    })
    const restart = button(wrapper, 'Restart the daemon')
    ;(restart.element as HTMLButtonElement).focus()
    await restart.trigger('click')
    await flushPromises()
    expect(document.activeElement).toBe(wrapper.get('[role="status"]').element)
  })

  it('never stacks status reads behind one that has not answered', async () => {
    await show(baseStatus())
    const reads = () => invoke.mock.calls.filter(([cmd]) => cmd === 'get_backend_status').length
    // get_backend_status waits on the app state's and the daemon manager's
    // locks. Neither the splash's 500 ms poll nor the shared 2 s one starts
    // another read while this one is out.
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'get_backend_status') return new Promise(() => {})
      return answer(cmd)
    })
    const before = reads()
    await vi.advanceTimersByTimeAsync(5000)
    expect(reads()).toBe(before + 1)
  })

  it('drops the corner radius while maximized', async () => {
    appWindow.isMaximized.mockResolvedValue(true)
    const { wrapper } = await show(baseStatus())
    expect(wrapper.get('main').classes()).not.toContain('rounded-[32px]')
  })

  it('keeps the corner radius when there is no Tauri window (browser dev)', async () => {
    appWindow.isMaximized.mockRejectedValue(new Error('no window'))
    const { wrapper } = await show(baseStatus())
    expect(wrapper.get('main').classes()).toContain('rounded-[32px]')
  })
})
