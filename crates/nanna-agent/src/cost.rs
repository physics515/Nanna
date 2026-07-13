//! Model pricing + USD cost estimation from accumulated token usage.
//!
//! The daemon already tracks per-model token counts (see [`crate::model_stats`]);
//! this module turns those counts into a dollar estimate so the agent can
//! surface spend per model/session. Pricing is a **reference list-price table**
//! (public per-1M-token rates, not negotiated contract rates) matched by model
//! *family prefix* so dated ids like `claude-opus-4-8` resolve. Local models
//! (Ollama / on-device Burn) are free and intentionally return `None`.

/// USD price per 1,000,000 tokens for one model, split by token class.
///
/// Cache reads are billed far below fresh input; cache writes slightly above.
/// All rates are non-negative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPricing {
    /// Fresh (uncached) input tokens, USD per 1M.
    pub input_usd_per_mtok: f64,
    /// Output/completion tokens, USD per 1M.
    pub output_usd_per_mtok: f64,
    /// Cache-read (prompt-cache hit) tokens, USD per 1M.
    pub cache_read_usd_per_mtok: f64,
    /// Cache-write (prompt-cache creation) tokens, USD per 1M.
    pub cache_write_usd_per_mtok: f64,
}

impl ModelPricing {
    /// Construct a pricing row. All rates must be finite and non-negative.
    #[must_use]
    pub const fn new(
        input_usd_per_mtok: f64,
        output_usd_per_mtok: f64,
        cache_read_usd_per_mtok: f64,
        cache_write_usd_per_mtok: f64,
    ) -> Self {
        Self {
            input_usd_per_mtok,
            output_usd_per_mtok,
            cache_read_usd_per_mtok,
            cache_write_usd_per_mtok,
        }
    }
}

/// Tokens in one pricing unit (rates are quoted per 1M tokens).
const TOKENS_PER_MILLION: f64 = 1_000_000.0;

/// Reference list prices (USD per 1M tokens), captured Jan 2026. These are
/// **public list prices**, not contract rates — update as vendors change them.
/// Matched by family prefix in [`default_pricing`], most-specific first.
///
/// Format: `(prefix, input, output, cache_read, cache_write)`.
const PRICING_TABLE: &[(&str, f64, f64, f64, f64)] = &[
    // Anthropic Claude — 2026 rates (cache read = 0.1x input, cache write =
    // 1.25x input for the 5-min TTL). Opus 4.x is $5/$25 (NOT the old Opus-3
    // $15/$75); Haiku 4.5 is $1/$5. Source: platform.claude.com/docs pricing.
    ("claude-opus-4", 5.00, 25.00, 0.50, 6.25),
    ("claude-sonnet", 3.00, 15.00, 0.30, 3.75),
    ("claude-haiku", 1.00, 5.00, 0.10, 1.25),
    ("claude-3-5-sonnet", 3.00, 15.00, 0.30, 3.75),
    ("claude-3-5-haiku", 0.80, 4.00, 0.08, 1.00),
    ("claude-3-opus", 15.00, 75.00, 1.50, 18.75), // legacy Opus 3
    ("claude-opus", 5.00, 25.00, 0.50, 6.25),     // generic Opus → 4.x rate
    ("claude", 3.00, 15.00, 0.30, 3.75),          // generic Claude → Sonnet rate
    // OpenAI GPT (cache read ~0.5x input; no separate cache-write charge).
    ("gpt-5", 1.25, 10.00, 0.625, 1.25),
    ("gpt-4o-mini", 0.15, 0.60, 0.075, 0.15),
    ("gpt-4o", 2.50, 10.00, 1.25, 2.50),
    ("gpt-4-turbo", 10.00, 30.00, 10.00, 10.00),
    ("gpt-4", 30.00, 60.00, 30.00, 30.00),
    ("o1-mini", 1.10, 4.40, 0.55, 1.10),
    ("o1", 15.00, 60.00, 7.50, 15.00),
];

/// Look up reference pricing for a model by family prefix.
///
/// Returns `None` for unknown or explicitly-free (local/Ollama) models — a
/// caller should treat `None` as "cost unknown / not billed", never as `$0`
/// silently rolled into a total.
#[must_use]
pub fn default_pricing(model_id: &str) -> Option<ModelPricing> {
    let id = model_id.to_ascii_lowercase();
    // Local backends are free; don't fall through to a cloud family match on a
    // model string that merely contains e.g. "gpt" in a local repo name.
    if id.starts_with("ollama") || id.starts_with("local") || id.contains(":latest") {
        return None;
    }
    for (prefix, input, output, cache_read, cache_write) in PRICING_TABLE {
        if id.starts_with(prefix) {
            return Some(ModelPricing::new(*input, *output, *cache_read, *cache_write));
        }
    }
    None
}

/// Estimate the USD cost of a set of token counts under a pricing row.
///
/// Pure arithmetic — no clock, no IO. `debug_assert`s guard the pricing
/// invariants (finite, non-negative) on hot paths; the return is always
/// non-negative.
// Token counts are u64 but never approach f64's 2^52 exact-integer ceiling
// (that's ~4.5 quadrillion tokens); the f64 cast is exact for any real usage.
#[allow(clippy::cast_precision_loss)]
#[must_use]
pub fn estimate_cost_usd(
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    pricing: &ModelPricing,
) -> f64 {
    debug_assert!(
        pricing.input_usd_per_mtok >= 0.0 && pricing.output_usd_per_mtok >= 0.0,
        "input/output rates must be non-negative"
    );
    debug_assert!(
        pricing.cache_read_usd_per_mtok >= 0.0 && pricing.cache_write_usd_per_mtok >= 0.0,
        "cache rates must be non-negative"
    );

    // Per-class cost in "USD * 1M tokens", summed then divided once. Kept as
    // explicit per-term locals so the sum is plain addition (no fused
    // multiply-add rewrite that would obscure the money math).
    let input = input_tokens as f64 * pricing.input_usd_per_mtok;
    let output = output_tokens as f64 * pricing.output_usd_per_mtok;
    let cache_read = cache_read_tokens as f64 * pricing.cache_read_usd_per_mtok;
    let cache_write = cache_write_tokens as f64 * pricing.cache_write_usd_per_mtok;
    let cost = (input + output + cache_read + cache_write) / TOKENS_PER_MILLION;

    debug_assert!(cost >= 0.0, "estimated cost must be non-negative");
    cost
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exact arithmetic: 1M input @ $3 + 1M output @ $15 = $18.00, with cache
    // classes priced independently.
    #[test]
    fn estimate_cost_is_exact_per_million() {
        let p = ModelPricing::new(3.0, 15.0, 0.3, 3.75);
        // 1,000,000 input + 1,000,000 output only:
        let c = estimate_cost_usd(1_000_000, 1_000_000, 0, 0, &p);
        assert!((c - 18.0).abs() < 1e-9, "got {c}");
        // Add 2M cache-read @ $0.30 (= $0.60) and 1M cache-write @ $3.75:
        let c2 = estimate_cost_usd(1_000_000, 1_000_000, 2_000_000, 1_000_000, &p);
        assert!((c2 - (18.0 + 0.60 + 3.75)).abs() < 1e-9, "got {c2}");
    }

    #[test]
    fn zero_tokens_cost_nothing() {
        let p = ModelPricing::new(3.0, 15.0, 0.3, 3.75);
        assert!(estimate_cost_usd(0, 0, 0, 0, &p).abs() < f64::EPSILON);
    }

    // Dated/versioned model ids must resolve via family prefix.
    #[test]
    fn dated_model_ids_resolve_by_prefix() {
        assert!(default_pricing("claude-opus-4-8").is_some());
        assert!(default_pricing("claude-sonnet-5").is_some());
        assert!(default_pricing("gpt-5-2026-01-01").is_some());
        // Most-specific wins: opus is pricier than the generic claude fallback.
        let opus = default_pricing("claude-opus-4-8").unwrap();
        let generic = default_pricing("claude-instant-xyz").unwrap();
        assert!(opus.output_usd_per_mtok > generic.output_usd_per_mtok);
    }

    // Local / unknown models are unpriced (None), never silently $0.
    #[test]
    fn local_and_unknown_models_are_unpriced() {
        assert!(default_pricing("ollama/llama3.2").is_none());
        assert!(default_pricing("local-qwen-3.5-9b").is_none());
        assert!(default_pricing("qwen2.5:latest").is_none());
        assert!(default_pricing("some-model-nobody-knows").is_none());
    }
}
