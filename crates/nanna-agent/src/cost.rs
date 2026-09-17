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

    /// Batch-API pricing: input and output are billed at **50%** of the interactive
    /// rate. The Batch API does not combine with prompt caching, so the cache rates
    /// are carried through unchanged (they simply don't apply to a batched request).
    #[must_use]
    pub fn with_batch_discount(&self) -> Self {
        let discounted = Self::new(
            self.input_usd_per_mtok * 0.5,
            self.output_usd_per_mtok * 0.5,
            self.cache_read_usd_per_mtok,
            self.cache_write_usd_per_mtok,
        );
        debug_assert!(
            discounted.input_usd_per_mtok <= self.input_usd_per_mtok
                && discounted.output_usd_per_mtok <= self.output_usd_per_mtok,
            "the batch rate can only lower input/output"
        );
        discounted
    }

    /// 1-hour prompt-cache pricing: the cache-**write** rate rises to **2x** the
    /// fresh-input rate (vs 1.25x for the default 5-minute TTL). Cache reads, input,
    /// and output are unchanged. Anchored to the input rate rather than scaling the
    /// stored 5-minute write, so it is exact regardless of how the row was seeded.
    #[must_use]
    pub fn with_hour_cache_write(&self) -> Self {
        let hourly = Self::new(
            self.input_usd_per_mtok,
            self.output_usd_per_mtok,
            self.cache_read_usd_per_mtok,
            self.input_usd_per_mtok * 2.0,
        );
        debug_assert!(
            hourly.cache_write_usd_per_mtok >= self.input_usd_per_mtok,
            "a 1-hour cache write is never cheaper than a fresh input token"
        );
        hourly
    }
}

/// Tokens in one pricing unit (rates are quoted per 1M tokens).
const TOKENS_PER_MILLION: f64 = 1_000_000.0;

/// Reference list prices (USD per 1M tokens); the Claude rows re-checked
/// against the vendor's page on 2026-09-11. These are **public list prices**,
/// not contract rates — update as vendors change them. Matched by family
/// prefix in [`default_pricing`]: the first row whose prefix the id starts
/// with wins, so a more specific prefix must come before a broader one.
///
/// Format: `(prefix, input, output, cache_read, cache_write)`.
const PRICING_TABLE: &[(&str, f64, f64, f64, f64)] = &[
    // Anthropic Claude. Source: platform.claude.com/docs/en/about-claude/pricing
    // (fetched 2026-09-11). Cache read = 0.1x input (0.025x on the 5.1
    // frontier models); cache write = 1.25x input for the 5-minute TTL.
    //
    // Opus 4 and 4.1 are $15/$75 — three times Opus 4.5 and later — so their
    // ids must match before the `claude-opus-4` row that covers 4.5–4.8.
    ("claude-opus-4-1", 15.00, 75.00, 1.50, 18.75),
    ("claude-opus-4-0", 15.00, 75.00, 1.50, 18.75), // the Opus 4 alias
    ("claude-opus-4-20", 15.00, 75.00, 1.50, 18.75), // dated Opus 4, e.g. -20250514
    ("claude-opus-4", 5.00, 25.00, 0.50, 6.25),     // Opus 4.5–4.8
    // Sonnet 5 is $2/$10 (its launch price, made standard); Sonnet 4.x $3/$15.
    ("claude-sonnet-5", 2.00, 10.00, 0.20, 2.50),
    ("claude-sonnet", 3.00, 15.00, 0.30, 3.75),
    ("claude-haiku-3-5", 0.80, 4.00, 0.08, 1.00), // Haiku 3.5, newer spelling
    ("claude-haiku", 1.00, 5.00, 0.10, 1.25),     // Haiku 4.5
    ("claude-3-5-sonnet", 3.00, 15.00, 0.30, 3.75),
    ("claude-3-5-haiku", 0.80, 4.00, 0.08, 1.00),
    ("claude-3-opus", 15.00, 75.00, 1.50, 18.75), // legacy Opus 3
    ("claude-opus", 5.00, 25.00, 0.50, 6.25),     // generic Opus (Opus 5) → $5/$25
    // Fable and Mythos: $10/$50. The 5.1 models read cache at 0.025x ($0.25),
    // the 5 models at the standard 0.1x ($1.00); 5-minute write 1.25x ($12.50).
    // All must precede the generic "claude" fallback, else they resolve to the
    // Sonnet rate.
    ("claude-fable-5-1", 10.00, 50.00, 0.25, 12.50),
    ("claude-fable", 10.00, 50.00, 1.00, 12.50),
    ("claude-mythos-5-1", 10.00, 50.00, 0.25, 12.50),
    ("claude-mythos-5", 10.00, 50.00, 1.00, 12.50),
    ("claude", 3.00, 15.00, 0.30, 3.75), // generic Claude → Sonnet rate
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

/// [`estimate_cost_usd`] for a cache-write total that mixes lifetimes: the
/// `cache_write_1h_tokens` share of `cache_write_tokens` is billed at the
/// 1-hour rate ([`ModelPricing::with_hour_cache_write`], 2x input), the rest
/// at the pricing's 5-minute rate.
///
/// The 1-hour count is a subset of the total, never an addition. A larger
/// value is a caller bug; release builds clamp it to the total rather than
/// bill a negative 5-minute remainder.
#[must_use]
pub fn estimate_cost_usd_with_hour_writes(
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_write_tokens: u64,
    cache_write_1h_tokens: u64,
    pricing: &ModelPricing,
) -> f64 {
    debug_assert!(
        cache_write_1h_tokens <= cache_write_tokens,
        "the 1-hour share is a subset of the write total"
    );
    let hour_tokens = cache_write_1h_tokens.min(cache_write_tokens);
    let five_minute_tokens = cache_write_tokens - hour_tokens;
    let base = estimate_cost_usd(
        input_tokens,
        output_tokens,
        cache_read_tokens,
        five_minute_tokens,
        pricing,
    );
    let hour = estimate_cost_usd(0, 0, 0, hour_tokens, &pricing.with_hour_cache_write());
    let cost = base + hour;
    debug_assert!(cost >= base, "a 1-hour share can only add cost");
    cost
}

#[cfg(test)]
mod hour_write_tests {
    use super::*;

    #[test]
    fn with_no_hour_share_it_is_exactly_the_five_minute_estimate() {
        let p = ModelPricing::new(3.0, 15.0, 0.3, 3.75);
        let plain = estimate_cost_usd(1_000_000, 1_000_000, 2_000_000, 1_000_000, &p);
        let split =
            estimate_cost_usd_with_hour_writes(1_000_000, 1_000_000, 2_000_000, 1_000_000, 0, &p);
        assert!((plain - split).abs() < 1e-9, "{plain} vs {split}");
    }

    #[test]
    fn an_hour_write_costs_twice_input_not_one_and_a_quarter() {
        let p = ModelPricing::new(3.0, 15.0, 0.3, 3.75);
        // 1M written at 1h: $6.00 (2 x $3 input), where the 5-minute rate says $3.75.
        let hour = estimate_cost_usd_with_hour_writes(0, 0, 0, 1_000_000, 1_000_000, &p);
        assert!((hour - 6.0).abs() < 1e-9, "got {hour}");
        // A half-and-half mix: $1.875 + $3.00.
        let mixed = estimate_cost_usd_with_hour_writes(0, 0, 0, 1_000_000, 500_000, &p);
        assert!((mixed - 4.875).abs() < 1e-9, "got {mixed}");
    }
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

    // Fable 5 resolves to its own $10/$50 row, NOT the generic "claude" Sonnet
    // fallback (the row must sit before the generic prefix in the table).
    #[test]
    fn fable_5_resolves_to_its_own_rate() {
        let fable = default_pricing("claude-fable-5").expect("fable is priced");
        assert!((fable.input_usd_per_mtok - 10.0).abs() < f64::EPSILON);
        assert!((fable.output_usd_per_mtok - 50.0).abs() < f64::EPSILON);
        assert!((fable.cache_read_usd_per_mtok - 1.0).abs() < f64::EPSILON);
        assert!((fable.cache_write_usd_per_mtok - 12.5).abs() < f64::EPSILON);
        // Dated id resolves the same, and is pricier than the generic claude fallback.
        let generic = default_pricing("claude-instant-xyz").unwrap();
        assert!(fable.output_usd_per_mtok > generic.output_usd_per_mtok);
    }

    /// Every Claude row against Anthropic's published rates (pricing page,
    /// fetched 2026-09-11), by real model id so the prefix order is tested
    /// too: `(id, input, output, cache_read, cache_write_5m)` per 1M tokens.
    /// Until this date Sonnet 5 was billed at the Sonnet 4 rate, Opus 4 and
    /// 4.1 at the Opus 4.5 rate, Fable 5.1's cache reads at Fable 5's, and
    /// Mythos 5 at the generic Sonnet fallback.
    #[test]
    fn claude_rates_match_the_published_table() {
        let published: &[(&str, f64, f64, f64, f64)] = &[
            ("claude-opus-5", 5.0, 25.0, 0.5, 6.25),
            ("claude-opus-4-8", 5.0, 25.0, 0.5, 6.25),
            ("claude-opus-4-5-20251101", 5.0, 25.0, 0.5, 6.25),
            ("claude-opus-4-1-20250805", 15.0, 75.0, 1.5, 18.75),
            ("claude-opus-4-20250514", 15.0, 75.0, 1.5, 18.75),
            ("claude-opus-4-0", 15.0, 75.0, 1.5, 18.75),
            ("claude-sonnet-5", 2.0, 10.0, 0.2, 2.5),
            ("claude-sonnet-4-6", 3.0, 15.0, 0.3, 3.75),
            ("claude-sonnet-4-5-20250929", 3.0, 15.0, 0.3, 3.75),
            ("claude-haiku-4-5-20251001", 1.0, 5.0, 0.1, 1.25),
            ("claude-haiku-3-5", 0.8, 4.0, 0.08, 1.0),
            ("claude-3-5-haiku-20241022", 0.8, 4.0, 0.08, 1.0),
            ("claude-fable-5", 10.0, 50.0, 1.0, 12.5),
            ("claude-fable-5-1", 10.0, 50.0, 0.25, 12.5),
            ("claude-mythos-5", 10.0, 50.0, 1.0, 12.5),
            ("claude-mythos-5-1", 10.0, 50.0, 0.25, 12.5),
        ];
        for &(id, input, output, read, write) in published {
            let p = default_pricing(id).unwrap_or_else(|| panic!("{id} must be priced"));
            let got = (
                p.input_usd_per_mtok,
                p.output_usd_per_mtok,
                p.cache_read_usd_per_mtok,
                p.cache_write_usd_per_mtok,
            );
            let matches = (got.0 - input).abs() < 1e-9
                && (got.1 - output).abs() < 1e-9
                && (got.2 - read).abs() < 1e-9
                && (got.3 - write).abs() < 1e-9;
            assert!(
                matches,
                "{id}: got {got:?}, published ({input}, {output}, {read}, {write})"
            );
        }
        // A 1-hour write is 2x input on every model; Fable 5.1's is $20.
        let fable_5_1 = default_pricing("claude-fable-5-1").expect("priced");
        assert!((fable_5_1.with_hour_cache_write().cache_write_usd_per_mtok - 20.0).abs() < 1e-9);
    }

    // Batch API halves input/output and leaves cache rates alone.
    #[test]
    fn batch_discount_halves_input_and_output() {
        let base = ModelPricing::new(10.0, 50.0, 1.0, 12.5); // Fable 5
        let batch = base.with_batch_discount();
        assert!((batch.input_usd_per_mtok - 5.0).abs() < f64::EPSILON);
        assert!((batch.output_usd_per_mtok - 25.0).abs() < f64::EPSILON);
        // Cache classes are unchanged (batch does not cache).
        assert!((batch.cache_read_usd_per_mtok - 1.0).abs() < f64::EPSILON);
        assert!((batch.cache_write_usd_per_mtok - 12.5).abs() < f64::EPSILON);
    }

    // 1-hour cache raises the write rate to 2x input; nothing else moves.
    #[test]
    fn hour_cache_write_is_twice_input() {
        let base = ModelPricing::new(10.0, 50.0, 1.0, 12.5); // Fable 5, 5-min write
        let hourly = base.with_hour_cache_write();
        assert!((hourly.cache_write_usd_per_mtok - 20.0).abs() < f64::EPSILON);
        // Input/output/read untouched.
        assert!((hourly.input_usd_per_mtok - 10.0).abs() < f64::EPSILON);
        assert!((hourly.output_usd_per_mtok - 50.0).abs() < f64::EPSILON);
        assert!((hourly.cache_read_usd_per_mtok - 1.0).abs() < f64::EPSILON);
    }
}
