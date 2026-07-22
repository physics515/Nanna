/** Pure windowed-list helpers (fixed row height). */

export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  itemCount: number,
  itemHeight: number,
  overscan = 6,
): { start: number; end: number; offsetY: number; totalHeight: number } {
  const safeCount = Math.max(0, itemCount | 0)
  const safeHeight = itemHeight > 0 ? itemHeight : 0
  const totalHeight = safeCount * safeHeight

  if (safeCount === 0 || safeHeight === 0) {
    return { start: 0, end: 0, offsetY: 0, totalHeight }
  }

  const top = Math.max(0, scrollTop)
  const start = Math.max(0, Math.floor(top / safeHeight) - overscan)
  const end = Math.min(
    safeCount,
    Math.ceil((top + Math.max(0, viewportHeight)) / safeHeight) + overscan,
  )
  const offsetY = start * safeHeight

  return { start, end, offsetY, totalHeight }
}
