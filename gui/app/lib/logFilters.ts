export interface FilterableLog {
  level: string
  source?: string
  message: string
}

export function filterLogs<T extends FilterableLog>(logs: T[], level: string, source: string, query: string): T[] {
  const needle = query.trim().toLocaleLowerCase()
  return logs.filter(log => (level === 'all' || log.level === level)
    && (source === 'all' || log.source === source)
    && (!needle || log.message.toLocaleLowerCase().includes(needle)))
}
