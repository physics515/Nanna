/**
 * The daemon removed every message of a session (`session_cleared`) — a chat
 * app's `/new`, or `session.clear` from another client.
 *
 * Without handling it, a chat open on that session kept showing a conversation
 * the next turn no longer sees. The page reloads the session from the daemon
 * rather than emptying local state by hand, so whatever else it holds for the
 * session (run state, pin) comes from the same authoritative read as on open.
 */

/** The cleared session id from an event payload. Malformed → null, never a throw. */
export function parseSessionCleared(payload: unknown): string | null {
  if (payload === null || typeof payload !== 'object') return null
  const { session_id } = payload as Record<string, unknown>
  return typeof session_id === 'string' && session_id !== '' ? session_id : null
}

/** Whether the chat showing `openSessionId` must reload for this event. */
export function shouldReloadForClear(payload: unknown, openSessionId: string | undefined): boolean {
  const cleared = parseSessionCleared(payload)
  return cleared !== null && cleared === openSessionId
}
