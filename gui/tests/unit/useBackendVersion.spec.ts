import { beforeEach, describe, expect, it, vi } from 'vitest'

/**
 * A status read must never wait on the daemon's version. A daemon too old to
 * know `system.version` answers get_daemon_version only after the client's
 * 300 s request timeout, and while refresh() awaited it the footer, the
 * startup splash and the layout's first load all froze for that long.
 */

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}))

const connected = { mode: 'daemon', connected: true, daemon_state: 'running', daemon_url: 'ws://127.0.0.1:5149', version: '0.0.0-gui' }
const offline = { mode: 'disconnected', connected: false, daemon_state: 'crashed', daemon_url: null, version: '0.0.0-gui' }

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

async function freshBackend() {
  // useBackend keeps module-level state (one per process).
  vi.resetModules()
  const { useBackend } = await import('~/composables/useBackend')
  return useBackend()
}

describe('useBackend — the daemon version', () => {
  beforeEach(() => {
    invoke.mockReset()
  })

  it('reads the status without waiting for the version', async () => {
    const version = deferred<string | null>()
    invoke.mockImplementation((command: string) =>
      command === 'get_backend_status' ? Promise.resolve(connected) : version.promise,
    )
    const backend = await freshBackend()

    const status = await backend.refresh()
    expect(status?.connected).toBe(true)
    expect(backend.daemonVersion.value).toBeNull()

    version.resolve('0.3.22')
    await vi.waitFor(() => expect(backend.daemonVersion.value).toBe('0.3.22'))
  })

  it('asks once while an answer is outstanding', async () => {
    const version = deferred<string | null>()
    invoke.mockImplementation((command: string) =>
      command === 'get_backend_status' ? Promise.resolve(connected) : version.promise,
    )
    const backend = await freshBackend()

    await backend.refresh()
    await backend.refresh()
    await backend.refresh()
    expect(invoke.mock.calls.filter((c) => c[0] === 'get_daemon_version')).toHaveLength(1)
  })

  it('drops an answer that arrives after the daemon went away', async () => {
    const version = deferred<string | null>()
    let status: Record<string, unknown> = connected
    invoke.mockImplementation((command: string) =>
      command === 'get_backend_status' ? Promise.resolve(status) : version.promise,
    )
    const backend = await freshBackend()

    await backend.refresh()
    status = offline
    await backend.refresh()
    version.resolve('0.3.21')
    await new Promise((r) => setTimeout(r, 0))
    expect(backend.daemonVersion.value).toBeNull()
  })
})
