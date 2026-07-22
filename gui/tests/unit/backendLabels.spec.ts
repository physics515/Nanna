import { describe, expect, it } from 'vitest'
import { describeBackend, statusBarLabel } from '../../app/lib/backendLabels'

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
