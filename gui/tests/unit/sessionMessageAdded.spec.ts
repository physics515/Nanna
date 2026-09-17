import { parseSessionMessageAdded, shouldAppendSessionMessage } from '~/lib/sessionMessageAdded'

/**
 * A reminder that comes due is appended to its session by the daemon with no
 * streamed turn around it. These pin the two decisions the chat page makes
 * about that event: is the payload usable, and does the open chat show it.
 */
describe('parseSessionMessageAdded', () => {
  const event = {
    session_id: 's-1',
    message_id: 'm-1',
    role: 'assistant',
    content: '⏰ Reminder: stretch',
  }

  it('reads the daemon wire shape back exactly', () => {
    expect(parseSessionMessageAdded(event)).toEqual(event)
  })

  it('drops malformed payloads and roles the chat does not render', () => {
    const bad: unknown[] = [
      null,
      undefined,
      'reminder',
      { ...event, session_id: '' },
      { ...event, message_id: 7 },
      { ...event, role: 'system' },
      { ...event, role: 'tool' },
      { ...event, content: '   ' },
      { ...event, content: undefined },
    ]
    for (const payload of bad) {
      expect(parseSessionMessageAdded(payload)).toBeNull()
    }
  })
})

describe('shouldAppendSessionMessage', () => {
  const added = { session_id: 's-1', message_id: 'm-9', role: 'assistant' as const, content: 'x' }

  it('appends into the open session', () => {
    expect(shouldAppendSessionMessage(added, 's-1', ['m-1', 'm-2'])).toBe(true)
  })

  it('leaves another session, or no open session, alone', () => {
    expect(shouldAppendSessionMessage(added, 's-2', [])).toBe(false)
    expect(shouldAppendSessionMessage(added, undefined, [])).toBe(false)
  })

  it('never shows a message twice', () => {
    expect(shouldAppendSessionMessage(added, 's-1', ['m-9'])).toBe(false)
  })
})
