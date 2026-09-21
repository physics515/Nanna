export interface FilterableLog {
  level: string
  source?: string
  message: string
  /** The span chain the line was logged in (`chat_turn{session_id=…}:tool_call{…}`); absent from older daemons. */
  scope?: string
}

/** Search matches the message OR the scope, so pasting a session id narrows the log to that conversation. */
export function filterLogs<T extends FilterableLog>(logs: T[], level: string, source: string, query: string): T[] {
  const needle = query.trim().toLocaleLowerCase()
  return logs.filter(log => (level === 'all' || log.level === level)
    && (source === 'all' || log.source === source)
    && (!needle
      || log.message.toLocaleLowerCase().includes(needle)
      || (log.scope ?? '').toLocaleLowerCase().includes(needle)))
}
