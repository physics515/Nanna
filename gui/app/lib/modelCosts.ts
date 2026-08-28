/**
 * Merge daemon ModelStats + cost_report into the GUI model-stats cards.
 *
 * Summaries do not carry USD. Cost lives on a sibling `costs` array
 * (`estimated_cost_usd`, `priced`). Local/unknown models stay unpriced —
 * never a silent $0 folded into the lifetime total.
 */

export interface ModelCostRow {
  model: string
  estimated_cost_usd: number
  priced: boolean
}

export interface ModelStatLike {
  model: string
  cost_usd?: number
  cost_priced?: boolean
  [key: string]: unknown
}

export interface ModelStatsPayload {
  models?: ModelStatLike[]
  costs?: ModelCostRow[]
  total_cost_usd?: number
}

export interface MergedModelStat extends ModelStatLike {
  cost_usd: number
  cost_priced: boolean
}

/** Format a USD amount for the stats header. Unpriced totals stay unlabeled. */
export function formatUsd(amount: number): string {
  if (!Number.isFinite(amount) || amount < 0) return '—'
  if (amount === 0) return '$0.00'
  if (amount < 0.01) return `$${amount.toFixed(4)}`
  return `$${amount.toFixed(2)}`
}

/**
 * Known cloud spend only. Prefer the daemon's `total_cost_usd` (already
 * filtered to priced models). Fall back to summing priced cost rows.
 */
export function lifetimePricedUsd(payload: ModelStatsPayload): number | null {
  if (typeof payload.total_cost_usd === 'number' && Number.isFinite(payload.total_cost_usd)) {
    return payload.total_cost_usd
  }
  const costs = payload.costs
  if (!costs || costs.length === 0) return null
  let sum = 0
  let any = false
  for (const row of costs) {
    if (row.priced && Number.isFinite(row.estimated_cost_usd)) {
      sum += row.estimated_cost_usd
      any = true
    }
  }
  return any ? sum : null
}

/** Attach per-model cost onto each summary. Missing cost → unpriced, not $0. */
export function mergeModelStatsWithCosts(
  models: ModelStatLike[] | undefined,
  costs: ModelCostRow[] | undefined,
): MergedModelStat[] {
  const byModel = new Map<string, ModelCostRow>()
  for (const row of costs ?? []) {
    if (row.model) byModel.set(row.model, row)
  }
  return (models ?? []).map((model) => {
    const row = byModel.get(model.model)
    if (!row) {
      return { ...model, cost_usd: 0, cost_priced: false }
    }
    return {
      ...model,
      cost_usd: row.priced ? row.estimated_cost_usd : 0,
      cost_priced: row.priced,
    }
  })
}
