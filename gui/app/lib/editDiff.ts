/**
 * The before/after view `edit_file` attaches to a successful edit as
 * `data.diff` (P18 "Diff presentation"). It reaches the GUI two ways — on the
 * live ToolEnd event's structured data, and on the daemon's persisted run
 * journal (`TimelineItem::Tool.diff`) — so one parser validates both before
 * anything renders it. Observability for a user returning to unattended work,
 * never an approval step.
 */
export interface EditDiff {
  /** 1-based line, in the file before the edit, where the change starts. */
  start_line: number
  removed: string[]
  added: string[]
  /** The view was cut to its bound. Rendered, never a silent cut. */
  truncated: boolean
}

/** Per-side line bound, mirrored from the daemon's `EDIT_DIFF_LINES_MAX`. */
export const EDIT_DIFF_LINES_MAX = 40

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every(line => typeof line === 'string')
}

/**
 * Validate an untrusted `diff` value. Anything malformed is `null`, so a bad
 * payload renders as no diff rather than as a broken card. A side over the
 * bound is cut to it and marked truncated — the live event carries the skill's
 * raw data, which the daemon's journal clamp never saw.
 */
export function parseEditDiff(value: unknown): EditDiff | null {
  if (value === null || typeof value !== 'object') return null
  const candidate = value as Record<string, unknown>
  const startLine = candidate.start_line
  if (typeof startLine !== 'number' || !Number.isInteger(startLine) || startLine < 1) return null
  if (!isStringArray(candidate.removed) || !isStringArray(candidate.added)) return null
  if (candidate.removed.length === 0 && candidate.added.length === 0) return null
  const removed = candidate.removed.slice(0, EDIT_DIFF_LINES_MAX)
  const added = candidate.added.slice(0, EDIT_DIFF_LINES_MAX)
  const cut = removed.length < candidate.removed.length || added.length < candidate.added.length
  return {
    start_line: startLine,
    removed,
    added,
    truncated: candidate.truncated === true || cut,
  }
}
