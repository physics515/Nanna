/**
 * The daemon's spend rollup (`system.cost_rollup`), validated. A bucket with
 * no price stays unpriced (`costUsd: null`) all the way to the screen — a local
 * model and an unknown one must not both read as "$0.00".
 */
export interface CostBucket {
  period: string
  model: string
  requests: number
  costUsd: number | null
}

export interface CostRollup {
  buckets: CostBucket[]
  pricedTotalUsd: number
  unpricedModels: string[]
}

const isCount = (v: unknown): v is number => typeof v === 'number' && Number.isInteger(v) && v >= 0

/** Parse the daemon's reply. Never throws; malformed buckets are dropped. */
export function parseCostRollup(payload: unknown): CostRollup {
  const empty: CostRollup = { buckets: [], pricedTotalUsd: 0, unpricedModels: [] }
  if (payload === null || typeof payload !== 'object') return empty
  const p = payload as Record<string, unknown>
  const buckets: CostBucket[] = []
  for (const entry of Array.isArray(p.buckets) ? p.buckets : []) {
    if (entry === null || typeof entry !== 'object') continue
    const e = entry as Record<string, unknown>
    if (typeof e.period !== 'string' || typeof e.model !== 'string' || !isCount(e.requests)) continue
    const cost = typeof e.cost_usd === 'number' && Number.isFinite(e.cost_usd) && e.cost_usd >= 0 ? e.cost_usd : null
    buckets.push({ period: e.period, model: e.model, requests: e.requests, costUsd: cost })
  }
  const total = typeof p.priced_total_usd === 'number' && Number.isFinite(p.priced_total_usd) ? p.priced_total_usd : 0
  const unpriced = Array.isArray(p.unpriced_models)
    ? p.unpriced_models.filter((m): m is string => typeof m === 'string')
    : []
  return { buckets, pricedTotalUsd: total, unpricedModels: unpriced }
}

/** Spend per period (priced buckets summed), newest first. Pure. */
export function spendByPeriod(rollup: CostRollup): { period: string; costUsd: number; requests: number; partlyUnpriced: boolean }[] {
  const byPeriod = new Map<string, { costUsd: number; requests: number; partlyUnpriced: boolean }>()
  for (const b of rollup.buckets) {
    const row = byPeriod.get(b.period) ?? { costUsd: 0, requests: 0, partlyUnpriced: false }
    row.requests += b.requests
    if (b.costUsd === null) row.partlyUnpriced = true
    else row.costUsd += b.costUsd
    byPeriod.set(b.period, row)
  }
  return [...byPeriod.entries()]
    .map(([period, row]) => ({ period, ...row }))
    .sort((a, b) => (a.period < b.period ? 1 : a.period > b.period ? -1 : 0))
}

/** Dollars to cents, or "—" for unknown. Pure. */
export function formatUsd(value: number | null): string {
  return value === null ? '—' : `$${value.toFixed(2)}`
}
