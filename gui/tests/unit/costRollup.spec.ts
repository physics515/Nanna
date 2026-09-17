import { formatUsd, parseCostRollup, spendByPeriod } from '~/lib/costRollup'

/**
 * Spend per day on the Model Stats page. The rule that matters: a day that
 * used an unpriced model is marked as a floor, never shown as a complete total.
 */
const DAEMON = {
  buckets: [
    { period: '2026-09-16', model: 'claude-sonnet-5', requests: 4, input_tokens: 1, output_tokens: 1, cache_read_tokens: 0, cache_write_tokens: 0, cache_write_1h_tokens: 0, cost_usd: 0.42 },
    { period: '2026-09-17', model: 'claude-sonnet-5', requests: 2, input_tokens: 1, output_tokens: 1, cache_read_tokens: 0, cache_write_tokens: 0, cache_write_1h_tokens: 0, cost_usd: 0.1 },
    { period: '2026-09-17', model: 'ollama/qwen3.5:9b', requests: 9, input_tokens: 1, output_tokens: 1, cache_read_tokens: 0, cache_write_tokens: 0, cache_write_1h_tokens: 0, cost_usd: null },
  ],
  priced_total_usd: 0.52,
  unpriced_models: ['ollama/qwen3.5:9b'],
}

describe('cost rollup', () => {
  it('reads the daemon shape and keeps unpriced as null', () => {
    const rollup = parseCostRollup(DAEMON)
    expect(rollup.buckets).toHaveLength(3)
    expect(rollup.buckets[2]!.costUsd).toBeNull()
    expect(rollup.pricedTotalUsd).toBe(0.52)
    expect(rollup.unpricedModels).toEqual(['ollama/qwen3.5:9b'])
  })

  it('drops malformed buckets and never throws', () => {
    expect(parseCostRollup(null).buckets).toEqual([])
    const rollup = parseCostRollup({
      buckets: [{ period: 1, model: 'x', requests: 1 }, { period: 'p', model: 'x', requests: -2 }, { period: 'p', model: 'x', requests: 1, cost_usd: -3 }],
    })
    expect(rollup.buckets).toEqual([{ period: 'p', model: 'x', requests: 1, costUsd: null }])
  })

  it('sums per day, newest first, and marks days with unpriced use', () => {
    const days = spendByPeriod(parseCostRollup(DAEMON))
    expect(days.map(d => d.period)).toEqual(['2026-09-17', '2026-09-16'])
    expect(days[0]).toEqual({ period: '2026-09-17', costUsd: 0.1, requests: 11, partlyUnpriced: true })
    expect(days[1]!.partlyUnpriced).toBe(false)
    expect(formatUsd(0.1)).toBe('$0.10')
    expect(formatUsd(null)).toBe('—')
  })
})
