/**
 * The gate's state is module-level on purpose (one per process), so each test
 * loads a fresh copy of the module.
 */
async function freshGate() {
  vi.resetModules()
  const { useStartupGate } = await import('~/composables/useStartupGate')
  return useStartupGate()
}

describe('useStartupGate', () => {
  it('holds the shell back until something releases it', async () => {
    const gate = await freshGate()
    expect(gate.released.value).toBe(false)
    expect(gate.releasedOffline.value).toBe(false)
    expect(gate.pageEpoch.value).toBe(0)
  })

  it('releases on the first attach, without remounting pages that mount after it', async () => {
    const gate = await freshGate()
    gate.noteConnected()
    expect(gate.released.value).toBe(true)
    expect(gate.releasedOffline.value).toBe(false)
    expect(gate.pageEpoch.value).toBe(0)
  })

  it('releases offline when the person opens Nanna anyway', async () => {
    const gate = await freshGate()
    gate.continueOffline()
    expect(gate.released.value).toBe(true)
    expect(gate.releasedOffline.value).toBe(true)
  })

  it('remounts the page once, on the first attach after an offline release', async () => {
    const gate = await freshGate()
    gate.continueOffline()
    expect(gate.pageEpoch.value).toBe(0)
    gate.noteConnected()
    expect(gate.pageEpoch.value).toBe(1)
    // A later reconnect finds pages that already hold real data.
    gate.noteConnected()
    gate.noteConnected()
    expect(gate.pageEpoch.value).toBe(1)
  })

  it('keeps a page that works offline across the first attach', async () => {
    const gate = await freshGate()
    gate.continueOffline()
    // Settings or logs: it may hold unsaved input, and a remount would drop it.
    gate.noteConnected({ worksOffline: true })
    expect(gate.released.value).toBe(true)
    expect(gate.pageEpoch.value).toBe(0)
    // The one chance to remount has passed: a later attach is not the first.
    gate.noteConnected()
    expect(gate.pageEpoch.value).toBe(0)
  })

  it('never re-arms: nothing takes the release back', async () => {
    const gate = await freshGate()
    gate.noteConnected()
    // There is no API to close the gate; a later disconnect is simply not
    // the gate's business. Continuing after an attach is not "offline".
    gate.continueOffline()
    expect(gate.released.value).toBe(true)
    expect(gate.releasedOffline.value).toBe(false)
    expect(gate.pageEpoch.value).toBe(0)
  })

  it('shares one latch between every caller in the process', async () => {
    const first = await freshGate()
    const { useStartupGate } = await import('~/composables/useStartupGate')
    const second = useStartupGate()
    first.continueOffline()
    expect(second.released.value).toBe(true)
    expect(second.releasedOffline.value).toBe(true)
  })
})
