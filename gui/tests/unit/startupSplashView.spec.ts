import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { SLOW_START_S, type BackendStatusLike, type DaemonFailure } from '~/lib/backendLabels'
import { describeExit, describeSplash } from '~/lib/startupSplash'

const status = (overrides: BackendStatusLike = {}): BackendStatusLike => ({
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

const failure = (overrides: Partial<DaemonFailure> = {}): DaemonFailure => ({
  kind: 'exited_during_boot',
  message: 'Error: IPC port 127.0.0.1:5149 unavailable: address in use',
  exit_code: 1,
  signal: null,
  at_ms: 1_758_000_000_000,
  ...overrides,
})

describe('describeSplash', () => {
  it('says the app itself is starting until the first status arrives', () => {
    for (const none of [null, undefined]) {
      const v = describeSplash(none)
      expect(v.phase).toBe('launching')
      expect(v.headline).toBe('Starting Nanna…')
      expect(v.primary).toBeNull()
      expect(v.tone).toBe('working')
    }
  })

  it('reads an ordinary boot as starting, with nothing to act on', () => {
    const v = describeSplash(status({ starting_for_s: SLOW_START_S - 1 }))
    expect(v.phase).toBe('starting')
    expect(v.headline).toBe('Starting the daemon…')
    expect(v.elapsed).toBeNull()
    expect(v.primary).toBeNull()
    expect(v.offerRestart).toBe(false)
  })

  it('turns into "Still starting" with the elapsed time at SLOW_START_S', () => {
    const v = describeSplash(status({ starting_for_s: 65 }))
    expect(v.phase).toBe('slow')
    expect(v.headline).toBe('Still starting')
    expect(v.elapsed).toBe('1m 05s')
    expect(v.detail).toMatch(/alive and still booting/)
    expect(v.detail).toMatch(/opens by itself as soon as it answers/)
    // A slow boot may be a hung one, and only a restart kills it, but it is
    // not the default: most slow boots finish.
    expect(v.offerRestart).toBe(true)
    expect(v.primary).toBeNull()

    expect(describeSplash(status({ starting_for_s: SLOW_START_S })).phase).toBe('slow')
  })

  it('treats a missing starting_for_s as an ordinary boot', () => {
    expect(describeSplash(status({ starting_for_s: null })).phase).toBe('starting')
  })

  it('reads a running daemon the client has not attached as connecting', () => {
    const v = describeSplash(status({ daemon_state: 'running', starting_for_s: null, init_in_progress: false }))
    expect(v.phase).toBe('connecting')
    expect(v.headline).toBe('Connecting…')
    expect(v.detail).toBe('')

    const retrying = describeSplash(status({ daemon_state: 'running', retrying: true }))
    expect(retrying.detail).toMatch(/keeps trying/)
  })

  it('offers Restart, quietly, for a running daemon that has not answered', () => {
    // The client is retrying, so the daemon is up but not answering: a hung
    // daemon looks exactly like this, and only a restart gets past it.
    const retrying = describeSplash(status({ daemon_state: 'running', retrying: true }))
    expect(retrying.offerRestart).toBe(true)
    expect(retrying.primary).toBeNull()
    // The moment between "ready" and the attach is not worth a restart.
    const attaching = describeSplash(status({ daemon_state: 'running', retrying: false }))
    expect(attaching.offerRestart).toBe(false)
  })

  it('shows a crash with the reason, the exit code and a primary Restart', () => {
    const v = describeSplash(status({
      daemon_state: 'crashed',
      starting_for_s: null,
      init_in_progress: false,
      retrying: true,
      last_error: failure(),
    }))
    expect(v.phase).toBe('failed')
    expect(v.headline).toBe('The daemon exited while starting')
    expect(v.reason).toBe('Error: IPC port 127.0.0.1:5149 unavailable: address in use')
    expect(v.exit).toBe('exit code 1')
    expect(v.primary).toBe('restart')
    expect(v.tone).toBe('failed')
    // Nothing respawns a daemon that died before the first connection; the
    // retry loop only reconnects. Say both, and no more.
    expect(v.detail).toMatch(/won't start it again by itself/)
    expect(v.detail).toMatch(/keeps trying/)
  })

  it('names the exit once when the reason already says it', () => {
    // The daemon printed no error of its own, so the backend's message is how
    // it ended; the line below it used to repeat "exit code 1".
    const view = describeSplash(
      status({
        daemon_state: 'crashed',
        last_error: failure({ kind: 'exited_during_boot', message: 'The daemon exited during startup (exit code 1)', exit_code: 1 }),
      }),
    )
    expect(view.reason).toBe('The daemon exited during startup (exit code 1)')
    expect(view.exit).toBeNull()

    const signalled = describeSplash(
      status({
        daemon_state: 'crashed',
        last_error: failure({ kind: 'exited_after_ready', message: 'The daemon exited (signal 9)', exit_code: null, signal: 9 }),
      }),
    )
    expect(signalled.exit).toBeNull()
  })

  it('names each failure kind for what the person sees', () => {
    const headline = (kind: string) =>
      describeSplash(status({ daemon_state: 'crashed', last_error: failure({ kind }) })).headline
    expect(headline('sidecar_unresolved')).toBe("Couldn't find the daemon")
    expect(headline('spawn_failed')).toBe("Couldn't start the daemon")
    expect(headline('exited_during_boot')).toBe('The daemon exited while starting')
    expect(headline('exited_after_ready')).toBe('The daemon exited')
    expect(headline('health_check_failed')).toBe('The daemon stopped answering')
    expect(headline('restarts_exhausted')).toBe('The daemon keeps crashing')
    expect(headline('something_new')).toBe('The daemon crashed')
  })

  it('still offers Restart for a crash that carries no reason', () => {
    const v = describeSplash(status({ daemon_state: 'crashed', last_error: null, retrying: false }))
    expect(v.headline).toBe('The daemon crashed')
    expect(v.reason).toBeNull()
    expect(v.exit).toBeNull()
    expect(v.primary).toBe('restart')
    expect(v.detail).not.toMatch(/keeps trying/)
  })

  it('tells "about to start" from "stopped" by init_in_progress', () => {
    const starting = describeSplash(status({ daemon_state: 'stopped', starting_for_s: null, init_in_progress: true }))
    expect(starting.phase).toBe('starting')
    expect(starting.primary).toBeNull()

    const stopped = describeSplash(status({ daemon_state: 'stopped', starting_for_s: null, init_in_progress: false }))
    expect(stopped.phase).toBe('stopped')
    expect(stopped.headline).toBe("The daemon isn't running")
    expect(stopped.primary).toBe('start')
    // Nothing is under way and nothing failed: a still dot, not the pulse.
    expect(stopped.tone).toBe('idle')
  })

  it('reads the frontend fallback and an empty state as stopped', () => {
    for (const daemon_state of ['not_started', '']) {
      expect(describeSplash(status({ daemon_state, init_in_progress: false })).phase).toBe('stopped')
    }
  })

  it('keeps the last failure on a stopped daemon', () => {
    const v = describeSplash(status({
      daemon_state: 'stopped',
      init_in_progress: false,
      last_error: failure({ kind: 'spawn_failed', message: 'Failed to spawn daemon: No such file', exit_code: null }),
    }))
    expect(v.reason).toBe('Failed to spawn daemon: No such file')
    expect(v.exit).toBeNull()
  })

  it('says "keeps trying" for a stopped daemon only when the client is retrying', () => {
    const idle = describeSplash(status({ daemon_state: 'stopped', init_in_progress: false, retrying: false }))
    // Opening Nanna anyway runs the layout's init, which starts a stopped
    // daemon: "without it" would be untrue.
    expect(idle.detail).toBe('Start it here, or open Nanna anyway, which starts it too. Chats work once it answers.')
    const retrying = describeSplash(status({ daemon_state: 'stopped', init_in_progress: false, retrying: true }))
    expect(retrying.detail).toMatch(/keeps trying/)
  })

  it('reads a stop in progress as stopping', () => {
    const v = describeSplash(status({ daemon_state: 'stopping' }))
    expect(v.phase).toBe('stopping')
    expect(v.headline).toBe('Stopping the daemon…')
    expect(v.primary).toBeNull()
  })

  it('names an unknown state instead of guessing', () => {
    const v = describeSplash(status({ daemon_state: 'hibernating' }))
    expect(v.phase).toBe('unknown')
    expect(v.detail).toBe('Daemon state: hibernating')
  })

  it('says Opening once connected, whatever the state field says', () => {
    // The transient crashed+connected of a sidecar that deferred to a daemon
    // already serving: attached is attached.
    for (const daemon_state of ['running', 'crashed']) {
      const v = describeSplash(status({ connected: true, mode: 'daemon', daemon_state }))
      expect(v.phase).toBe('ready')
      expect(v.headline).toBe('Opening Nanna…')
      expect(v.primary).toBeNull()
    }
  })

  it('holds "Restarting" while a restart has not been read back', () => {
    for (const daemon_state of ['stopping', 'stopped', 'crashed']) {
      const v = describeSplash(status({ daemon_state, init_in_progress: false }), { restarting: true })
      expect(v.phase).toBe('restarting')
      expect(v.headline).toBe('Restarting the daemon…')
      expect(v.primary).toBeNull()
    }
    // A daemon that attaches mid-restart wins.
    expect(describeSplash(status({ connected: true }), { restarting: true }).phase).toBe('ready')
  })

  it('never offers to start the daemon the updater stopped on purpose', () => {
    const v = describeSplash(status({ daemon_state: 'stopped', init_in_progress: false }), { updating: true })
    expect(v.phase).toBe('updating')
    expect(v.headline).toBe('Updating Nanna…')
    expect(v.primary).toBeNull()
    expect(describeSplash(null, { updating: true }).phase).toBe('updating')
  })

  it('keeps to the copy voice: no exclamation marks, "…" not "..."', () => {
    const views = [
      describeSplash(null),
      describeSplash(status()),
      describeSplash(status({ starting_for_s: 600 })),
      describeSplash(status({ daemon_state: 'running', retrying: true })),
      describeSplash(status({ daemon_state: 'crashed', retrying: true, last_error: failure() })),
      describeSplash(status({ daemon_state: 'stopped', init_in_progress: false })),
      describeSplash(status({ daemon_state: 'stopping' })),
      describeSplash(status(), { restarting: true }),
      describeSplash(status(), { updating: true }),
    ]
    for (const v of views) {
      for (const text of [v.headline, v.detail]) {
        expect(text).not.toMatch(/!/)
        expect(text).not.toMatch(/\.\.\./)
      }
    }
  })
})

describe('describeExit', () => {
  it('names the exit code, the signal, or both', () => {
    expect(describeExit(failure({ exit_code: 1, signal: null }))).toBe('exit code 1')
    expect(describeExit(failure({ exit_code: null, signal: 9 }))).toBe('signal 9')
    expect(describeExit(failure({ exit_code: 0, signal: 15 }))).toBe('exit code 0 · signal 15')
    expect(describeExit(failure({ exit_code: null, signal: null }))).toBeNull()
    expect(describeExit(null)).toBeNull()
  })
})

describe('spa-loading-template.html', () => {
  const template = readFileSync(join(process.cwd(), 'app/spa-loading-template.html'), 'utf8')

  it('carries no script: the CSP allows none inline', () => {
    expect(template).not.toMatch(/<script/i)
    expect(template).not.toMatch(/\son[a-z]+\s*=/i)
  })

  it("says the splash's first words, so the hand-over changes nothing", () => {
    expect(template).toContain(describeSplash(null).headline)
    expect(template).toContain('src="/logo.svg"')
  })
})
