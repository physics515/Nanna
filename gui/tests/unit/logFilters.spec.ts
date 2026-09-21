import { filterLogs } from '~/lib/logFilters'

const logs = [
  { level: 'info', source: 'daemon', message: 'Daemon ready' },
  { level: 'error', source: 'daemon', message: 'Model request failed' },
  { level: 'info', source: 'embedded', message: 'Legacy entry' },
]

const scoped = [
  { level: 'info', source: 'daemon', message: 'Executing tool', scope: 'chat_turn{session_id=aaaa-1111}:tool_call{tool=exec}' },
  { level: 'info', source: 'daemon', message: 'Executing tool', scope: 'chat_turn{session_id=bbbb-2222}:tool_call{tool=echo}' },
  { level: 'info', source: 'daemon', message: 'Daemon ready' },
]

describe('filterLogs', () => {
  it('combines level and source filters', () => expect(filterLogs(logs, 'info', 'daemon', '')).toEqual([logs[0]]))
  it('searches case-insensitively and trims the query', () => expect(filterLogs(logs, 'all', 'all', '  MODEL REQUEST ')).toEqual([logs[1]]))
  it('returns all entries with inactive filters', () => expect(filterLogs(logs, 'all', 'all', '')).toEqual(logs))
  it('finds one conversation by its session id in the scope', () =>
    expect(filterLogs(scoped, 'all', 'all', 'BBBB-2222')).toEqual([scoped[1]]))
  it('treats an entry without a scope as not matching a scope-only query', () =>
    expect(filterLogs(scoped, 'all', 'all', 'tool_call')).toEqual([scoped[0], scoped[1]]))
})
