import { filterLogs } from '~/lib/logFilters'

const logs = [
  { level: 'info', source: 'daemon', message: 'Daemon ready' },
  { level: 'error', source: 'daemon', message: 'Model request failed' },
  { level: 'info', source: 'embedded', message: 'Legacy entry' },
]

describe('filterLogs', () => {
  it('combines level and source filters', () => expect(filterLogs(logs, 'info', 'daemon', '')).toEqual([logs[0]]))
  it('searches case-insensitively and trims the query', () => expect(filterLogs(logs, 'all', 'all', '  MODEL REQUEST ')).toEqual([logs[1]]))
  it('returns all entries with inactive filters', () => expect(filterLogs(logs, 'all', 'all', '')).toEqual(logs))
})
