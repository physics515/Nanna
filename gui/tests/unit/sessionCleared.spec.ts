import { parseSessionCleared, shouldReloadForClear } from '~/lib/sessionCleared'

describe('session_cleared', () => {
  it('reads the session id and rejects malformed payloads', () => {
    expect(parseSessionCleared({ session_id: 'telegram:1:2' })).toBe('telegram:1:2')
    expect(parseSessionCleared({ session_id: '' })).toBeNull()
    expect(parseSessionCleared({ id: 'telegram:1:2' })).toBeNull()
    expect(parseSessionCleared(null)).toBeNull()
    expect(parseSessionCleared('telegram:1:2')).toBeNull()
  })

  it('reloads only the chat that shows the cleared session', () => {
    const payload = { session_id: 's-1' }
    expect(shouldReloadForClear(payload, 's-1')).toBe(true)
    expect(shouldReloadForClear(payload, 's-2')).toBe(false)
    expect(shouldReloadForClear(payload, undefined)).toBe(false)
    expect(shouldReloadForClear({}, undefined)).toBe(false)
  })
})
