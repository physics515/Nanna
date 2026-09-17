/**
 * A complete message the daemon appended to a session outside any streamed
 * turn — today, a reminder coming due (`session_message_added`).
 *
 * A streamed turn announces itself with stream chunks and a final `done`; this
 * one has no turn, so without handling the event the chat would show nothing
 * until the session was reopened. One parser validates the payload and one
 * pure function decides whether the open chat shows it, so both are testable
 * without mounting the page.
 */
export interface SessionMessageAdded {
  session_id: string
  message_id: string
  role: 'user' | 'assistant'
  content: string
}

/** Roles the chat renders as bubbles; system/tool rows are not shown there. */
const RENDERED_ROLES = new Set(['user', 'assistant'])

/** Validate an event payload. Malformed or unrendered → null, never a throw. */
export function parseSessionMessageAdded(payload: unknown): SessionMessageAdded | null {
  if (payload === null || typeof payload !== 'object') return null
  const p = payload as Record<string, unknown>
  const { session_id, message_id, role, content } = p
  if (typeof session_id !== 'string' || session_id === '') return null
  if (typeof message_id !== 'string' || message_id === '') return null
  if (typeof role !== 'string' || !RENDERED_ROLES.has(role)) return null
  if (typeof content !== 'string' || content.trim() === '') return null
  return { session_id, message_id, role: role as SessionMessageAdded['role'], content }
}

/**
 * Whether the chat currently showing `openSessionId`, holding messages with
 * `shownIds`, should append `added`. Another session's message is left for
 * that session's own load; a message already shown (the page loaded history
 * after the daemon persisted it) is not shown twice.
 */
export function shouldAppendSessionMessage(
  added: SessionMessageAdded,
  openSessionId: string | undefined,
  shownIds: readonly string[],
): boolean {
  if (added.session_id !== openSessionId) return false
  return !shownIds.includes(added.message_id)
}
