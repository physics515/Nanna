import { EDIT_DIFF_LINES_MAX, parseEditDiff } from '~/lib/editDiff'

/**
 * P18 "Diff presentation": edit_file's before/after view reaches the GUI on
 * the live event AND on the daemon's persisted journal. One parser validates
 * both, so a malformed payload renders as no diff, never as a broken card.
 */
describe('parseEditDiff', () => {
  const view = { start_line: 2, removed: ['line two'], added: ['line 2'], truncated: false }

  it('reads a well-formed view back exactly', () => {
    expect(parseEditDiff(view)).toEqual(view)
  })

  it('drops malformed payloads', () => {
    const bad: unknown[] = [
      null,
      undefined,
      'diff',
      42,
      { ...view, start_line: 0 },
      { ...view, start_line: 1.5 },
      { ...view, start_line: '2' },
      { ...view, removed: 'line two' },
      { ...view, added: [1] },
      { ...view, removed: [], added: [] },
    ]
    for (const payload of bad) {
      expect(parseEditDiff(payload)).toBeNull()
    }
  })

  it('cuts an oversized side to the bound and says so', () => {
    const many = Array.from({ length: 100 }, (_, i) => `old ${i}`)
    const parsed = parseEditDiff({ ...view, removed: many })
    expect(parsed?.removed).toHaveLength(EDIT_DIFF_LINES_MAX)
    expect(parsed?.removed[0]).toBe('old 0')
    expect(parsed?.truncated).toBe(true)
  })

  it('reads a missing truncated flag as not cut', () => {
    const noFlag: Record<string, unknown> = { ...view }
    delete noFlag.truncated
    expect(parseEditDiff(noFlag)?.truncated).toBe(false)
  })
})
