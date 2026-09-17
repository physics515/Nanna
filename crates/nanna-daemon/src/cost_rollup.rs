//! Spend over time: the request log priced per day or month.
//!
//! The daemon already priced its lifetime token totals (`system.model_stats`
//! → `costs`). An always-on daemon that works autonomously needs the other
//! axis — what did yesterday cost, what is this month on track for — and
//! lifetime totals cannot be split back into days. The per-request log can,
//! once something writes it: nothing did until this change (see the sink set
//! in `server.rs`).
//!
//! Prices are the reference list-price table in `nanna_agent::cost`. A model
//! with no row (local, unknown) is reported **unpriced**, never as $0 — a
//! local model costing nothing and an unknown model costing an unknown amount
//! must not read the same.

use nanna_storage::ModelUsageBucket;
use serde::Serialize;

/// One period's usage and estimated spend for one model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PricedBucket {
    #[serde(flatten)]
    pub usage: ModelUsageBucket,
    /// `None` when the model has no price row.
    pub cost_usd: Option<f64>,
}

/// The whole rollup.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CostRollup {
    pub buckets: Vec<PricedBucket>,
    /// Sum over priced buckets only.
    pub priced_total_usd: f64,
    /// Models that have usage but no price, so the total is a floor.
    pub unpriced_models: Vec<String>,
}

/// Price usage buckets. Pure.
#[must_use]
pub fn price_buckets(usage: Vec<ModelUsageBucket>) -> CostRollup {
    let mut unpriced_models: Vec<String> = Vec::new();
    let mut priced_total_usd = 0.0_f64;
    let buckets: Vec<PricedBucket> = usage
        .into_iter()
        .map(|usage| {
            let cost_usd = nanna_agent::cost::default_pricing(&usage.model).map(|pricing| {
                nanna_agent::cost::estimate_cost_usd_with_hour_writes(
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read_tokens,
                    usage.cache_write_tokens,
                    usage.cache_write_1h_tokens.min(usage.cache_write_tokens),
                    &pricing,
                )
            });
            match cost_usd {
                Some(cost) => priced_total_usd += cost,
                None if !unpriced_models.contains(&usage.model) => {
                    unpriced_models.push(usage.model.clone());
                }
                None => {}
            }
            PricedBucket { usage, cost_usd }
        })
        .collect();
    debug_assert!(priced_total_usd >= 0.0, "list prices are non-negative");
    debug_assert!(unpriced_models.len() <= buckets.len());
    CostRollup {
        buckets,
        priced_total_usd,
        unpriced_models,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(model: &str, input: u64, output: u64) -> ModelUsageBucket {
        ModelUsageBucket {
            period: "2026-09-17".into(),
            model: model.into(),
            requests: 1,
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cache_write_1h_tokens: 0,
        }
    }

    #[test]
    fn priced_models_sum_and_unpriced_ones_are_named_not_zeroed() {
        let rollup = price_buckets(vec![
            bucket("claude-sonnet-5", 1_000_000, 0),
            bucket("ollama/qwen3.5:9b", 5_000_000, 5_000_000),
            bucket("ollama/qwen3.5:9b", 1, 1),
        ]);
        let sonnet = rollup.buckets[0].cost_usd.expect("sonnet is priced");
        assert!(sonnet > 0.0);
        assert_eq!(
            rollup.buckets[1].cost_usd, None,
            "local is unpriced, not $0"
        );
        assert!((rollup.priced_total_usd - sonnet).abs() < 1e-9);
        assert_eq!(
            rollup.unpriced_models,
            vec!["ollama/qwen3.5:9b".to_string()],
            "named once"
        );
        assert_eq!(price_buckets(Vec::new()).buckets, Vec::new());
    }
}
