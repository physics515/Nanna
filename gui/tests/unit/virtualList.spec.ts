import { describe, expect, it } from 'vitest'
import { visibleRange } from '../../app/lib/virtualList'

describe('visibleRange', () => {
  it('returns empty window for zero items', () => {
    expect(visibleRange(0, 200, 0, 40, 6)).toEqual({
      start: 0,
      end: 0,
      offsetY: 0,
      totalHeight: 0,
    })
  })

  it('returns empty window for non-positive item height', () => {
    expect(visibleRange(0, 200, 50, 0, 6)).toEqual({
      start: 0,
      end: 0,
      offsetY: 0,
      totalHeight: 0,
    })
  })

  it('computes start/end with overscan at top', () => {
    // viewport fits 5 rows (200/40); overscan 2 → end = 5 + 2 = 7
    const r = visibleRange(0, 200, 100, 40, 2)
    expect(r.start).toBe(0)
    expect(r.end).toBe(7)
    expect(r.offsetY).toBe(0)
    expect(r.totalHeight).toBe(4000)
  })

  it('shifts window when scrolled mid-list', () => {
    // scrollTop 400 → index 10; overscan 3 → start 7, end ceil((400+200)/40)+3 = 18
    const r = visibleRange(400, 200, 100, 40, 3)
    expect(r.start).toBe(7)
    expect(r.end).toBe(18)
    expect(r.offsetY).toBe(280)
    expect(r.totalHeight).toBe(4000)
  })

  it('clamps end to itemCount near bottom', () => {
    const r = visibleRange(3600, 200, 100, 40, 6)
    expect(r.start).toBe(Math.max(0, Math.floor(3600 / 40) - 6))
    expect(r.end).toBe(100)
    expect(r.offsetY).toBe(r.start * 40)
  })

  it('defaults overscan to 6', () => {
    const withDefault = visibleRange(0, 100, 50, 20)
    const explicit = visibleRange(0, 100, 50, 20, 6)
    expect(withDefault).toEqual(explicit)
    expect(withDefault.end).toBe(Math.ceil(100 / 20) + 6)
  })
})
