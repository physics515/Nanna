import { describe, expect, it } from 'vitest'
import { filterActions, NAV_ACTIONS, QUICK_ACTIONS, type PaletteAction } from '../../app/lib/commandPalette'

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
