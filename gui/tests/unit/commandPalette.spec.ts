import { describe, expect, it } from 'vitest'
import {
  filterActions,
  subsequenceScore,
  NAV_ACTIONS,
  QUICK_ACTIONS,
  type PaletteAction,
} from '../../app/lib/commandPalette'

const ALL: PaletteAction[] = [...NAV_ACTIONS, ...QUICK_ACTIONS]

describe('filterActions', () => {
  it('returns all actions (capped) when query is empty', () => {
    const result = filterActions(ALL, '')
    expect(result.length).toBe(ALL.length)
    expect(result.map((a) => a.id)).toEqual(ALL.map((a) => a.id))
  })

  it('matches label case-insensitively', () => {
    const chat = filterActions(ALL, 'CHAT')
    expect(chat.some((a) => a.id === 'nav-chat')).toBe(true)
    expect(chat[0]?.id).toBe('nav-chat')
  })

  it('matches keywords and id', () => {
    expect(filterActions(ALL, 'cron').some((a) => a.id === 'nav-scheduler')).toBe(true)
    expect(filterActions(ALL, 'nav-tools').some((a) => a.id === 'nav-tools')).toBe(true)
    expect(filterActions(ALL, 'new-chat').some((a) => a.action === 'new-chat')).toBe(true)
  })

  it('matches group names', () => {
    const admin = filterActions(ALL, 'admin')
    expect(admin.length).toBeGreaterThan(0)
    expect(admin.every((a) => a.group.toLowerCase().includes('admin'))).toBe(true)
  })

  it('returns empty for no match', () => {
    expect(filterActions(ALL, 'zzzz-no-such-command-qqq')).toEqual([])
  })

  it('trims whitespace query', () => {
    const bare = filterActions(ALL, 'memory')
    const padded = filterActions(ALL, '  memory  ')
    expect(padded.map((a) => a.id)).toEqual(bare.map((a) => a.id))
  })

  it('limits results to 50', () => {
    const many: PaletteAction[] = Array.from({ length: 80 }, (_, i) => ({
      id: `item-${i}`,
      label: `Item ${i}`,
      group: 'Bulk',
      keywords: ['bulk'],
    }))
    expect(filterActions(many, '').length).toBe(50)
    expect(filterActions(many, 'item').length).toBe(50)
  })

  it('prefers exact / prefix label matches', () => {
    const actions: PaletteAction[] = [
      { id: 'a', label: 'Settings advanced', group: 'X' },
      { id: 'b', label: 'Settings', group: 'X' },
      { id: 'c', label: 'Open settings models', group: 'X' },
    ]
    const result = filterActions(actions, 'settings')
    expect(result[0]?.id).toBe('b')
  })
})

describe('subsequenceScore', () => {
  it('rejects a query that is not a subsequence', () => {
    expect(subsequenceScore('model stats', 'zzz')).toBe(-1)
  })

  it('rejects a single-character query as too weak to be meaningful', () => {
    expect(subsequenceScore('model stats', 'm')).toBe(-1)
  })

  it('rejects a subsequence that never lands on a word boundary', () => {
    // 'oe' really is a subsequence of "model stats", and that kind of accidental hit is
    // exactly the noise that makes naive fuzzy palettes feel broken.
    expect(subsequenceScore('model stats', 'oe')).toBe(-1)
  })

  it('scores initials of each word highly', () => {
    expect(subsequenceScore('model stats', 'ms')).toBeGreaterThan(0)
  })

  it('rewards consecutive runs over scattered matches', () => {
    const consecutive = subsequenceScore('toggle live logs', 'togg')
    const scattered = subsequenceScore('toggle live logs', 'tlls')
    expect(consecutive).toBeGreaterThan(scattered)
  })

  it('never exceeds the fuzzy ceiling, so a fuzzy hit cannot outrank a literal one', () => {
    // 25 is FUZZY_SCORE_MAX; the weakest literal tier (group) is 30.
    expect(subsequenceScore('model stats', 'model stats')).toBeLessThanOrEqual(25)
  })
})

describe('filterActions — fuzzy tier', () => {
  it('finds actions that a substring search misses entirely', () => {
    // None of these appear as a contiguous substring of any label or keyword.
    expect(filterActions(ALL, 'mstats').some((a) => a.id === 'nav-model-stats')).toBe(true)
    expect(filterActions(ALL, 'tglogs').some((a) => a.id === 'toggle-live-logs')).toBe(true)
    expect(filterActions(ALL, 'nchat').some((a) => a.id === 'new-chat')).toBe(true)
  })

  it('ranks every literal match above every fuzzy one', () => {
    const results = filterActions(ALL, 'logs')
    // 'Logs' is an exact label match; anything reached only by subsequence must come after it.
    expect(results[0]?.id).toBe('nav-logs')
  })

  it('still returns nothing for a query that matches no action at all', () => {
    expect(filterActions(ALL, 'zzzz-no-such-command-qqq')).toEqual([])
  })

  it('keeps a keyword hit ahead of a group-name hit', () => {
    const actions: PaletteAction[] = [
      { id: 'by-group', label: 'Unrelated', group: 'Diagnostics' },
      { id: 'by-keyword', label: 'Unrelated too', group: 'Other', keywords: ['diagnostics'] },
    ]
    expect(filterActions(actions, 'diagnostics')[0]?.id).toBe('by-keyword')
  })
})
