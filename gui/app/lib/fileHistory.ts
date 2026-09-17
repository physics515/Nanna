/**
 * A conversation's file checkpoints (`session.file_history`): each file as it
 * was just before a tool write replaced it. Parsed defensively — a malformed
 * entry is dropped rather than rendered as a row whose Restore does something
 * nobody can predict.
 */
export interface FileCheckpoint {
  checkpoint: number
  path: string
  existed: boolean
  bytes: number
  takenAt: string
  baseline: boolean
}

export interface FileHistory {
  checkpoints: FileCheckpoint[]
  total: number
}

const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0

/** Parse the daemon's reply. Never throws. */
export function parseFileHistory(payload: unknown): FileHistory {
  const empty = { checkpoints: [], total: 0 }
  if (payload === null || typeof payload !== 'object') return empty
  const p = payload as Record<string, unknown>
  const raw = Array.isArray(p.checkpoints) ? p.checkpoints : []
  const checkpoints: FileCheckpoint[] = []
  for (const entry of raw) {
    if (entry === null || typeof entry !== 'object') continue
    const e = entry as Record<string, unknown>
    if (!isCount(e.checkpoint) || typeof e.path !== 'string' || e.path === '') continue
    if (typeof e.existed !== 'boolean' || !isCount(e.bytes) || typeof e.taken_at !== 'string') continue
    checkpoints.push({
      checkpoint: e.checkpoint,
      path: e.path,
      existed: e.existed,
      bytes: e.bytes,
      takenAt: e.taken_at,
      baseline: e.baseline === true,
    })
  }
  const total = isCount(p.total) ? Math.max(p.total, checkpoints.length) : checkpoints.length
  return { checkpoints, total }
}

/** The last path component, for a compact row; the full path is the title. */
export function fileName(path: string): string {
  const parts = path.split(/[\\/]/)
  return parts[parts.length - 1] || path
}

/** What restoring this checkpoint will do, in one clause. Pure. */
export function restoreEffect(c: FileCheckpoint): string {
  return c.existed
    ? `put ${fileName(c.path)} back to its ${c.bytes.toLocaleString()}-byte version`
    : `remove ${fileName(c.path)} (a tool created it)`
}
