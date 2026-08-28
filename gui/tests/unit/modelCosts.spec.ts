import { describe, expect, it } from 'vitest'
import {
  formatUsd,
  lifetimePricedUsd,
  mergeModelStatsWithCosts,
} from '../../app/lib/modelCosts'

describe('mergeModelStatsWithCosts', () => {
  it('attaches priced USD onto matching models', () => {
    const merged = mergeModelStatsWithCosts(
      [{ model: 'claude-sonnet-5', total_requests: 3 }],
      [{ model: 'claude-sonnet-5', estimated_cost_usd: 18, priced: true }],
    )
    expect(merged).toHaveLength(1)
    expect(merged[0].cost_priced).toBe(true)
    expect(merged[0].cost_usd).toBe(18)
  })

  it('flags local/unknown models unpriced instead of silent $0', () => {
    const merged = mergeModelStatsWithCosts(
      [{ model: 'ollama/qwen3.5:9b' }],
      [{ model: 'ollama/qwen3.5:9b', estimated_cost_usd: 0, priced: false }],
    )
    expect(merged[0].cost_priced).toBe(false)
    expect(merged[0].cost_usd).toBe(0)
  })

  it('treats a missing cost row as unpriced', () => {
    const merged = mergeModelStatsWithCosts([{ model: 'mystery' }], [])
    expect(merged[0].cost_priced).toBe(false)
  })
})

describe('lifetimePricedUsd', () => {
  it('prefers the daemon total_cost_usd', () => {
    expect(
      lifetimePricedUsd({
        total_cost_usd: 29.25,
        costs: [{ model: 'claude-sonnet-5', estimated_cost_usd: 18, priced: true }],
      }),
    ).toBe(29.25)
  })

  it('sums only priced rows when the total is absent', () => {
    expect(
      lifetimePricedUsd({
        costs: [
          { model: 'claude-sonnet-5', estimated_cost_usd: 18, priced: true },
          { model: 'ollama/llama', estimated_cost_usd: 0, priced: false },
        ],
      }),
    ).toBe(18)
  })

  it('returns null when nothing is priced', () => {
    expect(lifetimePricedUsd({ costs: [{ model: 'local', estimated_cost_usd: 0, priced: false }] })).toBeNull()
    expect(lifetimePricedUsd({})).toBeNull()
  })
})

describe('formatUsd', () => {
  it('formats dollars and tiny amounts', () => {
    expect(formatUsd(18)).toBe('$18.00')
    expect(formatUsd(0)).toBe('$0.00')
    expect(formatUsd(0.0012)).toBe('$0.0012')
    expect(formatUsd(Number.NaN)).toBe('—')
  })
})
