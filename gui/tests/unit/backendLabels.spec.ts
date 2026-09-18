import { describe, expect, it } from 'vitest'
import { describeBackend, formatElapsed, SLOW_START_S, statusBarLabel } from '../../app/lib/backendLabels'

describe('describeBackend', () => {
  it('reports attached daemon specifically', () => {
    const d = describeBackend({
      mode: 'daemon',
      connected: true,
      daemon_url: 'ws://127.0.0.1:5149',
      daemon_state: 'running',
    })
    expect(d.online).toBe(true)
    expect(d.short).toBe('Daemon')
    expect(d.detail).toContain('5149')
    expect(d.tooltip.toLowerCase()).not.toContain('disconnected')
  })

  it('never says bare Disconnected when offline', () => {
    const d = describeBackend({
      mode: 'disconnected',
      connected: false,
      daemon_url: 'ws://127.0.0.1:5149',
      daemon_state: 'stopped',
    })
    expect(d.online).toBe(false)
    expect(d.short.toLowerCase()).not.toBe('disconnected')
    expect(d.tooltip).toMatch(/5149|not reachable|offline/i)
  })

  it('distinguishes reconnecting from hard offline', () => {
    const d = describeBackend({
      mode: 'disconnected',
      connected: false,
      daemon_url: 'ws://127.0.0.1:5149',
      daemon_state: 'reconnecting',
    })
    expect(d.short).toBe('Reconnecting')
    expect(d.tone).toBe('warn')
  })

  it('marks crashed daemon as error', () => {
    const d = describeBackend({
      mode: 'disconnected',
      connected: false,
      daemon_url: 'ws://127.0.0.1:5149',
      daemon_state: 'crashed',
    })
    expect(d.short).toMatch(/crash/i)
    expect(d.tone).toBe('error')
  })

  // useBackend falls back to this state when init_backend itself fails. It
  // contains "start", and it used to read as "Starting" for good (2026-09-18).
  it('does not mistake not_started for starting', () => {
    const d = describeBackend({ mode: 'disconnected', connected: false, daemon_state: 'not_started' })
    expect(d.short).toBe('Daemon offline')
    expect(d.tone).toBe('error')
  })

  it('says starting plainly while a boot is still normal', () => {
    const d = describeBackend({
      mode: 'disconnected',
      connected: false,
      daemon_state: 'starting',
      starting_for_s: SLOW_START_S - 1,
    })
    expect(d.short).toBe('Starting')
    expect(d.tone).toBe('info')
  })

  // A slow boot is alive, not failed. Show how long it has been going.
  it('says still starting, with the elapsed time, once a boot is slow', () => {
    const d = describeBackend({
      mode: 'disconnected',
      connected: false,
      daemon_url: null,
      daemon_state: 'starting',
      starting_for_s: 125,
    })
    expect(d.short).toBe('Still starting · 2m 05s')
    expect(d.tone).toBe('info')
    expect(d.online).toBe(false)
    expect(d.tooltip).toMatch(/alive and still booting/)
  })

  it('tells the footer when the app is still retrying', () => {
    const crashed = statusBarLabel(
      { mode: 'disconnected', connected: false, daemon_state: 'crashed', retrying: true },
      true,
    )
    expect(crashed.text).toBe('Daemon crashed · retrying')
    expect(crashed.tone).toBe('error')

    const offline = describeBackend({ mode: 'disconnected', connected: false, daemon_state: 'stopped', retrying: true })
    expect(offline.short).toBe('Daemon offline · retrying')
    expect(offline.tooltip).toMatch(/keeps trying/)

    // Not retrying (after a deliberate shutdown): no promise of a retry.
    const stopped = describeBackend({ mode: 'disconnected', connected: false, daemon_state: 'stopped', retrying: false })
    expect(stopped.short).toBe('Daemon offline')
    expect(stopped.tooltip).not.toMatch(/keeps trying/)
  })

  it('reads a running daemon with a retrying client as reconnecting', () => {
    const d = describeBackend({ mode: 'disconnected', connected: false, daemon_state: 'running', retrying: true })
    expect(d.short).toBe('Reconnecting')
    expect(d.tone).toBe('warn')
  })

  it('labels retired embedded mode without claiming online', () => {
    const d = describeBackend({ mode: 'embedded', connected: true })
    expect(d.online).toBe(false)
    expect(d.short).toMatch(/legacy/i)
  })

  it('statusBar prefers Connected when online', () => {
    const s = statusBarLabel(
      { mode: 'daemon', connected: true, daemon_url: 'ws://127.0.0.1:5149' },
      true,
    )
    expect(s.text).toBe('Connected')
    expect(s.tone).toBe('ok')
  })
})

describe('formatElapsed', () => {
  it('stays compact at every scale', () => {
    expect(formatElapsed(0)).toBe('0s')
    expect(formatElapsed(42.9)).toBe('42s')
    expect(formatElapsed(60)).toBe('1m 00s')
    expect(formatElapsed(125)).toBe('2m 05s')
    expect(formatElapsed(3780)).toBe('1h 03m')
    expect(formatElapsed(-5)).toBe('0s')
  })
})
