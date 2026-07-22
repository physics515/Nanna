import { describe, expect, it } from 'vitest'
import { validateApiKey, validateRequired, validatePort } from '../../app/lib/formValidation'

describe('formValidation', () => {
  it('rejects empty API keys with inline error', () => {
    const r = validateApiKey('   ')
    expect(r.ok).toBe(false)
    if (!r.ok) expect(r.error.length).toBeGreaterThan(0)
  })

  it('rejects too-short keys', () => {
    const r = validateApiKey('sk-ab')
    expect(r.ok).toBe(false)
  })

  it('accepts plausible keys and trims', () => {
    const r = validateApiKey('  sk-test-0123456789abcdef  ')
    expect(r.ok).toBe(true)
    if (r.ok) expect(r.value.startsWith('sk-test')).toBe(true)
  })

  it('required field does not invent values', () => {
    const r = validateRequired('', 'Name')
    expect(r.ok).toBe(false)
  })

  it('port bounds', () => {
    expect(validatePort('0').ok).toBe(false)
    expect(validatePort('65536').ok).toBe(false)
    expect(validatePort('5149').ok).toBe(true)
  })
})
