import { describe, expect, it } from 'vitest'
import { isProviderAvailable } from '../../app/lib/providerAvailability'

describe('isProviderAvailable', () => {
  it('daemon-reported list is authoritative when present', () => {
    expect(isProviderAvailable(['anthropic', 'ollama'], 'anthropic', false)).toBe(true)
    expect(isProviderAvailable(['openrouter', 'ollama'], 'openrouter', false)).toBe(true)
  })

  it('hides a provider the daemon cannot route even when the GUI is logged in', () => {
    // The live split-brain: GUI-local OAuth login true, but the daemon booted
    // without Anthropic — offering claude-* here made every chat turn fail
    // with "No provider available".
    expect(isProviderAvailable(['openrouter', 'ollama'], 'anthropic', true)).toBe(false)
  })

  it('an empty daemon list still wins over local credentials', () => {
    expect(isProviderAvailable([], 'anthropic', true)).toBe(false)
    expect(isProviderAvailable([], 'openai', true)).toBe(false)
  })

  it('falls back to local credential state only when the daemon was unreachable', () => {
    expect(isProviderAvailable(null, 'anthropic', true)).toBe(true)
    expect(isProviderAvailable(null, 'anthropic', false)).toBe(false)
    expect(isProviderAvailable(null, 'github', true)).toBe(true)
  })
})
