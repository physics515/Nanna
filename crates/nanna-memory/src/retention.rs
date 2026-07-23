//! Memory retention harness — recall accuracy before/after a dream cycle.
//!
//! The moat claim (ROADMAP P13) is that *dreaming* shrinks the memory footprint
//! **while holding recall**: same-topic memories are ranked, concatenated, and
//! summarized into one, so the store gets smaller but a query for that topic still
//! surfaces the topic's content. This module is the instrument that measures that
//! claim so it can be a **gate**, not a hope.
//!
//! Concretely it measures **topic recall@k** — the fraction of probe queries whose
//! raw top-`k` vector neighbours still include a memory tagged with the probe's
//! topic — once *before* and once *after* a consolidation run, and reports the
//! store's compression alongside the recall it retained.
//!
//! ## Why it gates the FSRS `w20` fix
//!
//! Consolidation only merges memories whose FSRS `weight` (retrievability ×
//! importance) lands in a *compressible* band; `w20` is the forgetting-curve decay
//! exponent, so it directly moves where an aged memory lands. The same aged corpus
//! consolidates differently under `w20 = 0.5` (the lagging FSRS-5 constant we ship)
//! versus `w20 = 0.0658` (the FSRS-6 default). This harness is how we tell whether
//! flipping that constant actually recalls better instead of guessing — run the same
//! [`RetentionCorpus`] through [`run_retention_cycle`] with each parameter set and
//! compare [`RetentionReport::recall_retention`].
//!
//! Everything here is deterministic and offline: embeddings are a pure function of a
//! `topic:<n>` tag carried in each memory's content, standing in for the semantic
//! clustering a real embedder would produce, so no model or network is involved.

// This module fabricates synthetic corpora with a small PRNG, so lossy numeric
// conversions are intentional and bounded by test-scale inputs: memory/topic
// counts (`usize`) and PRNG words (`u64`/`u32`) become `f32` vector components,
// and day counts (`f32`) become `i64` Unix seconds. Fused-multiply-add rounding
// differences are likewise immaterial to synthetic jitter. Silencing the pedantic
// numeric lints here is cleaner than a per-line allow on every cast.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::suboptimal_flops
)]

use std::collections::HashMap;

use crate::{
    ConsolidationConfig, ConsolidationResult, EmbedFn, FsrsState, MemoryEntry, MemoryError,
    MemoryService,
};

/// A single recall probe: a query embedding and the topic it should retrieve.
#[derive(Debug, Clone)]
pub struct RetentionProbe {
    /// The query vector (unnormalized is fine — the store normalizes on search).
    pub query_embedding: Vec<f32>,
    /// The topic this probe belongs to; a hit is any top-`k` memory whose
    /// `topic` metadata equals this.
    pub topic: String,
}

/// One recall measurement — the state of recall at a point in the cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetentionMeasurement {
    /// Live memory count in the store when measured.
    pub memory_count: usize,
    /// Number of probes evaluated.
    pub probe_count: usize,
    /// `top_k` used for the neighbour lookup.
    pub k: usize,
    /// Fraction of probes whose top-`k` neighbours included a same-topic memory,
    /// in `[0, 1]`.
    pub recall_at_k: f32,
}

/// Before/after comparison across a single dream (consolidation) cycle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RetentionReport {
    /// Recall + count measured before consolidation.
    pub before: RetentionMeasurement,
    /// Recall + count measured after consolidation.
    pub after: RetentionMeasurement,
    /// Memories merged away by the cycle (`ConsolidationResult::memories_merged`).
    pub memories_merged: usize,
}

impl RetentionReport {
    /// Fraction of memories removed by the cycle, in `[0, 1]`.
    ///
    /// `0.0` if the store was empty or grew (consolidation never grows the store,
    /// but the guard keeps the ratio well-defined).
    #[must_use]
    pub fn compression_ratio(&self) -> f32 {
        let before = self.before.memory_count;
        if before == 0 || self.after.memory_count >= before {
            return 0.0;
        }
        let removed = before - self.after.memory_count;
        removed as f32 / before as f32
    }

    /// Recall retained across the cycle: `after.recall / before.recall`, in `[0, ∞)`.
    ///
    /// `1.0` means recall was perfectly preserved; `< 1.0` means the cycle lost
    /// recall (over-merging across topics); `> 1.0` means it improved (denoising).
    /// If there was no recall to begin with, returns `1.0` when after is also zero
    /// (nothing to retain) and `f32::INFINITY` otherwise (recall appeared).
    #[must_use]
    pub fn recall_retention(&self) -> f32 {
        let before = self.before.recall_at_k;
        if before <= 0.0 {
            return if self.after.recall_at_k <= 0.0 {
                1.0
            } else {
                f32::INFINITY
            };
        }
        self.after.recall_at_k / before
    }
}

/// Metadata key under which a memory's topic is stored.
pub const TOPIC_METADATA_KEY: &str = "topic";

/// Measure topic recall@`k` over `probes` against the current store contents.
///
/// A probe *hits* when at least one of its top-`k` vector neighbours carries
/// `metadata[TOPIC_METADATA_KEY] == probe.topic`. `recall_at_k` is the hit fraction.
///
/// # Panics
/// Asserts `k >= 1` and that the returned fraction is a finite value in `[0, 1]`.
pub async fn measure_recall(
    service: &MemoryService,
    probes: &[RetentionProbe],
    k: usize,
) -> RetentionMeasurement {
    assert!(k >= 1, "measure_recall needs a positive k");

    let memory_count = service.count().await;
    let probe_count = probes.len();

    let mut hits: usize = 0;
    for probe in probes {
        let neighbours = service.search_by_embedding(&probe.query_embedding, k).await;
        let hit = neighbours
            .iter()
            .any(|(entry, _score)| entry.metadata.get(TOPIC_METADATA_KEY) == Some(&probe.topic));
        if hit {
            hits += 1;
        }
    }

    let recall_at_k = if probe_count == 0 {
        0.0
    } else {
        hits as f32 / probe_count as f32
    };

    debug_assert!(
        recall_at_k.is_finite() && (0.0..=1.0).contains(&recall_at_k),
        "recall_at_k out of range: {recall_at_k}"
    );
    debug_assert!(hits <= probe_count, "more hits than probes");

    RetentionMeasurement {
        memory_count,
        probe_count,
        k,
        recall_at_k,
    }
}

/// Run one full retention cycle: measure recall, run a dream (consolidation)
/// cycle, measure recall again, and report the delta.
///
/// The caller is responsible for having seeded `service` (e.g. via
/// [`RetentionCorpus::load_into`]) and for providing a `summarize_fn` — a stub
/// that echoes the topic tag is enough for the deterministic harness.
///
/// # Errors
/// Propagates any [`MemoryError`] from the consolidation run.
///
/// # Panics
/// Asserts `k >= 1`.
pub async fn run_retention_cycle<F, Fut>(
    service: &MemoryService,
    probes: &[RetentionProbe],
    k: usize,
    consolidation: &ConsolidationConfig,
    summarize_fn: F,
) -> Result<(RetentionReport, ConsolidationResult), MemoryError>
where
    F: Fn(String) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Result<String, String>> + Send,
{
    assert!(k >= 1, "run_retention_cycle needs a positive k");

    let before = measure_recall(service, probes, k).await;
    let result = service.consolidate(consolidation, summarize_fn).await?;
    let after = measure_recall(service, probes, k).await;

    // Consolidation removes `memories_merged` entries and adds one per cluster;
    // the store must not have grown.
    debug_assert!(
        after.memory_count <= before.memory_count,
        "consolidation grew the store: {} -> {}",
        before.memory_count,
        after.memory_count
    );

    let report = RetentionReport {
        before,
        after,
        memories_merged: result.memories_merged,
    };
    Ok((report, result))
}

/// Measure topic recall through the **FSRS-gated** [`MemoryService::recall`] path
/// — the one an agent actually uses — rather than raw vector search.
///
/// Unlike [`measure_recall`], `recall` drops memories whose FSRS `weight`
/// (retrievability × importance) is below `min_weight`, and retrievability is
/// governed by the forgetting-curve decay exponent `w20`. So this measurement is
/// **`w20`-sensitive**: an over-fast decay exponent marks aged-but-valid memories
/// "forgotten" and they never surface. Replaying one corpus under two
/// [`FsrsParameters`](crate::FsrsParameters) and comparing this fraction is the
/// concrete w20 experiment (see the crate tests).
///
/// A probe hits when any returned result carries `topic == probe.topic`. Returns
/// the hit fraction in `[0, 1]`. `service` must have an embedding function set
/// (e.g. [`RetentionCorpus::topic_embed_fn`]); probes are turned into `topic:<n>`
/// query strings that embedding function maps back to the topic centroid.
///
/// # Errors
/// Propagates any [`MemoryError`] from the recall path (e.g. no embedding fn).
pub async fn measure_gated_recall(
    service: &MemoryService,
    probes: &[RetentionProbe],
) -> Result<f32, MemoryError> {
    let probe_count = probes.len();
    if probe_count == 0 {
        return Ok(0.0);
    }

    let mut hits: usize = 0;
    for probe in probes {
        // The corpus's `topic_embed_fn` maps this tag back to the topic centroid.
        let query = format!("topic:{} recall", probe.topic);
        let results = service.recall(&query).await?;
        let hit = results
            .iter()
            .any(|r| r.metadata.get(TOPIC_METADATA_KEY) == Some(&probe.topic));
        if hit {
            hits += 1;
        }
    }

    let fraction = hits as f32 / probe_count as f32;
    debug_assert!(
        fraction.is_finite() && (0.0..=1.0).contains(&fraction),
        "gated recall fraction out of range: {fraction}"
    );
    Ok(fraction)
}

/// A deterministic synthetic corpus of topic clusters plus one probe per topic.
///
/// Several clusters of near-duplicate memories, reproducible from a single `seed`
/// so the same corpus can be replayed under different FSRS parameters.
#[derive(Debug, Clone)]
pub struct RetentionCorpus {
    /// The seeded memories (already tagged + aged), ready to add to a store.
    pub memories: Vec<MemoryEntry>,
    /// One probe per topic, aimed at that topic's centroid.
    pub probes: Vec<RetentionProbe>,
    /// Embedding dimension every vector shares.
    pub dimension: usize,
    /// Seed the corpus was generated from.
    pub seed: u64,
}

/// Parameters for [`RetentionCorpus::generate`].
#[derive(Debug, Clone, Copy)]
pub struct CorpusParams {
    /// Number of distinct topic clusters.
    pub topic_count: usize,
    /// Memories per topic.
    pub per_topic: usize,
    /// Embedding dimension.
    pub dimension: usize,
    /// How many days in the past the *most recent* topic was last accessed.
    /// Aging drops retrievability into a compressible band so consolidation has
    /// work to do (fresh memories sit in the skipped `Detailed` band).
    pub age_days: f32,
    /// Extra days of age added per topic index, giving each topic a distinct
    /// *era*. This is what lets the composite clusterer separate topics: without
    /// it, memories that share recall/importance/age proximity cluster across
    /// topic boundaries (the non-similarity signals dominate the fixed
    /// clustering weights), so dreaming would merge unrelated topics. A gap large
    /// relative to the 30-day age half-life keeps cross-topic age-proximity low.
    pub era_gap_days: f32,
    /// Initial FSRS stability (days) for each memory.
    pub stability_days: f32,
    /// Fixed importance for every memory, or `None` to vary it per topic
    /// (`1.0 + (topic % 3) * 0.5`). A fixed value isolates the effect of the
    /// FSRS decay exponent in the `w20` recall experiment (where importance must
    /// be held constant so only retrievability moves).
    pub importance: Option<f32>,
}

impl Default for CorpusParams {
    fn default() -> Self {
        Self {
            topic_count: 8,
            per_topic: 12,
            dimension: 64,
            age_days: 6.0,
            era_gap_days: 50.0,
            stability_days: 1.0,
            importance: None,
        }
    }
}

impl RetentionCorpus {
    /// Generate a deterministic corpus from `seed` and `params`.
    ///
    /// Each topic gets a pseudo-random unit-ish centroid; each memory is that
    /// centroid plus small deterministic jitter, tagged `topic:<n>` in both its
    /// content and `topic` metadata. Probes point at the bare centroid.
    ///
    /// # Panics
    /// Asserts a non-empty topic/dimension configuration and that every produced
    /// embedding has the requested dimension.
    #[must_use]
    pub fn generate(seed: u64, params: CorpusParams) -> Self {
        debug_assert!(params.topic_count >= 1, "need at least one topic");
        debug_assert!(params.per_topic >= 1, "need at least one memory per topic");
        debug_assert!(params.dimension >= 1, "need a positive dimension");

        let now = now_secs();

        let mut memories = Vec::with_capacity(params.topic_count * params.per_topic);
        let mut probes = Vec::with_capacity(params.topic_count);

        for topic in 0..params.topic_count {
            let centroid = topic_centroid(seed, topic, params.dimension);

            // Each topic lives in its own era and salience band so the composite
            // clusterer keeps them apart. Within a topic these are constant, so
            // members bind tightly; across topics they diverge.
            let topic_age_days = params.age_days + topic as f32 * params.era_gap_days;
            let topic_last_access = now - (topic_age_days * 86_400.0) as i64;
            let topic_importance = params.importance.unwrap_or(1.0 + (topic % 3) as f32 * 0.5);
            let topic_access_count = topic as u32 * 4;

            for member in 0..params.per_topic {
                // Deterministic jitter keeps members distinct but tightly clustered.
                let mut rng = SplitMix64::new(seed ^ mix_topic_member(topic as u64, member as u64));
                let embedding: Vec<f32> = centroid
                    .iter()
                    .map(|&c| c + (rng.next_unit() - 0.5) * 0.02)
                    .collect();
                debug_assert_eq!(embedding.len(), params.dimension);

                let mut metadata = HashMap::new();
                metadata.insert(TOPIC_METADATA_KEY.to_string(), topic.to_string());

                let fsrs = FsrsState {
                    stability: params.stability_days,
                    last_access: topic_last_access,
                    importance: topic_importance,
                    access_count: topic_access_count,
                    ..FsrsState::default()
                };

                memories.push(MemoryEntry {
                    id: format!("t{topic}-m{member}"),
                    content: format!("topic:{topic} memory {member}"),
                    embedding,
                    metadata,
                    timestamp: topic_last_access,
                    fsrs,
                    workspace_id: None,
                });
            }

            probes.push(RetentionProbe {
                query_embedding: centroid,
                topic: topic.to_string(),
            });
        }

        Self {
            memories,
            probes,
            dimension: params.dimension,
            seed,
        }
    }

    /// Add every memory in this corpus to `service`'s store.
    ///
    /// # Errors
    /// Propagates the first add error (e.g. a dimension mismatch).
    pub async fn load_into(&self, service: &MemoryService) -> Result<(), MemoryError> {
        for memory in &self.memories {
            service.add_entry(memory.clone()).await?;
        }
        Ok(())
    }

    /// An [`EmbedFn`] that re-embeds a consolidation summary back onto its topic
    /// centroid by parsing the `topic:<n>` tag the harness stamps into content.
    ///
    /// A summary with no recognizable tag falls back to a stable hash-derived
    /// vector, so a mis-tagged summary simply won't match any topic probe (a
    /// truthful recall miss) rather than panicking.
    #[must_use]
    pub fn topic_embed_fn(&self) -> EmbedFn {
        let seed = self.seed;
        let dimension = self.dimension;
        std::sync::Arc::new(move |text: &str| {
            let vector = parse_topic_tag(text).map_or_else(
                || hash_vector(text, dimension),
                |topic| topic_centroid(seed, topic, dimension),
            );
            Box::pin(async move { Ok(vector) })
                as std::pin::Pin<
                    Box<dyn std::future::Future<Output = Result<Vec<f32>, String>> + Send>,
                >
        })
    }
}

/// Deterministic per-topic centroid vector.
#[must_use]
pub fn topic_centroid(seed: u64, topic: usize, dimension: usize) -> Vec<f32> {
    let mut rng = SplitMix64::new(
        seed.wrapping_add(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(topic as u64),
    );
    (0..dimension).map(|_| rng.next_unit() - 0.5).collect()
}

/// Parse a leading/embedded `topic:<n>` tag from consolidation-summary text.
fn parse_topic_tag(text: &str) -> Option<usize> {
    let idx = text.find("topic:")?;
    let rest = &text[idx + "topic:".len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Stable hash-derived fallback vector for untagged text.
fn hash_vector(text: &str, dimension: usize) -> Vec<f32> {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    let mut rng = SplitMix64::new(hash);
    (0..dimension).map(|_| rng.next_unit() - 0.5).collect()
}

/// Combine a topic and member index into a well-mixed jitter seed.
const fn mix_topic_member(topic: u64, member: u64) -> u64 {
    topic
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(member.wrapping_mul(0xBF58_476D_1CE4_E5B9))
}

/// Current Unix time in seconds (0 if the clock is before the epoch).
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// A tiny `SplitMix64` PRNG — pure, dependency-free, deterministic. Used only for
/// synthetic corpus generation, never for anything security-sensitive.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Next value in `[0, 1)`.
    fn next_unit(&mut self) -> f32 {
        // Top 24 bits → a float with full mantissa precision in [0, 1).
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryService, MemoryServiceConfig};

    fn service_for(dim: usize) -> MemoryService {
        MemoryService::new(MemoryServiceConfig {
            dimension: dim,
            ..MemoryServiceConfig::default()
        })
    }

    /// A `summarize_fn` that preserves the cluster's `topic:<n>` tag so the
    /// consolidated entry re-embeds onto the same centroid. Takes `String` by
    /// value to match the `Fn(String) -> Fut` contract the consolidator requires.
    #[allow(clippy::needless_pass_by_value)]
    fn echo_summarize(prompt: String) -> impl std::future::Future<Output = Result<String, String>> {
        // The prompt is built from member contents, each carrying `topic:<n>`.
        // Echo the first tag so the summary is deterministically re-embeddable.
        let summary = parse_topic_tag(&prompt).map_or_else(
            || "untagged consolidated".to_string(),
            |topic| format!("topic:{topic} consolidated"),
        );
        async move { Ok(summary) }
    }

    #[test]
    fn corpus_generation_is_deterministic() {
        let params = CorpusParams::default();
        let a = RetentionCorpus::generate(42, params);
        let b = RetentionCorpus::generate(42, params);
        assert_eq!(a.memories.len(), b.memories.len());
        for (ma, mb) in a.memories.iter().zip(&b.memories) {
            assert_eq!(ma.id, mb.id);
            assert_eq!(
                ma.embedding, mb.embedding,
                "same seed must give same vectors"
            );
        }
        // A different seed gives different centroids.
        let c = RetentionCorpus::generate(43, params);
        assert_ne!(a.probes[0].query_embedding, c.probes[0].query_embedding);
    }

    #[test]
    fn topic_tag_parsing() {
        assert_eq!(parse_topic_tag("topic:5 memory 3"), Some(5));
        assert_eq!(parse_topic_tag("prefix topic:12 suffix"), Some(12));
        assert_eq!(parse_topic_tag("no tag here"), None);
        assert_eq!(parse_topic_tag("topic:"), None);
        assert_eq!(parse_topic_tag("topic:abc"), None);
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact ratios of exactly-representable inputs
    fn report_ratio_math() {
        let m = |count, recall| RetentionMeasurement {
            memory_count: count,
            probe_count: 8,
            k: 5,
            recall_at_k: recall,
        };
        // 100 -> 40 memories, recall 1.0 -> 0.9.
        let r = RetentionReport {
            before: m(100, 1.0),
            after: m(40, 0.9),
            memories_merged: 60,
        };
        assert!((r.compression_ratio() - 0.60).abs() < 1e-6);
        assert!((r.recall_retention() - 0.9).abs() < 1e-6);

        // Empty / grown store → compression 0, not NaN.
        let empty = RetentionReport {
            before: m(0, 0.0),
            after: m(0, 0.0),
            memories_merged: 0,
        };
        assert_eq!(empty.compression_ratio(), 0.0);
        assert_eq!(empty.recall_retention(), 1.0);

        // Recall appeared from nothing → infinity, not divide-by-zero.
        let appeared = RetentionReport {
            before: m(10, 0.0),
            after: m(5, 0.5),
            memories_merged: 5,
        };
        assert!(appeared.recall_retention().is_infinite());
    }

    #[tokio::test]
    async fn recall_measures_topic_hits() {
        let dim = 32;
        let corpus = RetentionCorpus::generate(
            7,
            CorpusParams {
                topic_count: 4,
                per_topic: 5,
                dimension: dim,
                ..CorpusParams::default()
            },
        );
        let service = service_for(dim);
        corpus.load_into(&service).await.unwrap();

        // Every probe should retrieve its own topic in the top-3.
        let measurement = measure_recall(&service, &corpus.probes, 3).await;
        assert_eq!(measurement.probe_count, 4);
        assert_eq!(measurement.memory_count, 20);
        assert!(
            (measurement.recall_at_k - 1.0).abs() < 1e-6,
            "fresh corpus should recall every topic, got {}",
            measurement.recall_at_k
        );
    }

    #[tokio::test]
    async fn dreaming_shrinks_store_while_holding_recall() {
        let dim = 48;
        // Under the FSRS-6 decay exponent (w20 = 0.0658, now the default) memories
        // decay slowly, so a corpus must be aged well past a year for its weight to
        // fall into a compressible band; importance is held uniform so no topic sits
        // in the never-consolidated high-weight tail.
        let corpus = RetentionCorpus::generate(
            99,
            CorpusParams {
                topic_count: 6,
                per_topic: 10,
                dimension: dim,
                age_days: 1000.0,
                era_gap_days: 60.0,
                stability_days: 1.0,
                importance: Some(1.0),
            },
        );
        let mut service = service_for(dim);
        service.set_embed_fn(corpus.topic_embed_fn());
        corpus.load_into(&service).await.unwrap();

        // A low floor so the 60-memory store actually consolidates. The raised
        // cluster threshold keeps consolidation to genuinely-similar memories
        // (within-topic composite score ~1.0) and refuses borderline cross-topic
        // pairs (~0.45) that would otherwise over-merge and drop a topic.
        let consolidation = ConsolidationConfig {
            min_remaining_memories: 6,
            max_compression_ratio: 0.9,
            cluster_threshold: 0.65,
            ..ConsolidationConfig::default()
        };

        let (report, result) =
            run_retention_cycle(&service, &corpus.probes, 3, &consolidation, echo_summarize)
                .await
                .unwrap();

        // The cycle must have actually done work and shrunk the store.
        assert!(
            result.memories_merged > 0,
            "nothing consolidated: {result:?}"
        );
        assert!(
            report.after.memory_count < report.before.memory_count,
            "store did not shrink: {report:?}"
        );
        assert!(report.compression_ratio() > 0.0);

        // And recall must be fully retained — same-topic merges keep the topic
        // reachable at its centroid.
        assert!(
            (report.before.recall_at_k - 1.0).abs() < 1e-6,
            "baseline recall should be perfect: {report:?}"
        );
        assert!(
            report.recall_retention() >= 1.0,
            "dreaming lost recall: retention {} ({report:?})",
            report.recall_retention()
        );
    }

    /// Build a service whose FSRS decay exponent is `w20`, with the corpus's
    /// tag-aware embedding function attached so gated recall works.
    fn service_with_w20(dim: usize, w20: f32, embed: crate::EmbedFn) -> MemoryService {
        let fsrs = crate::FsrsParameters {
            w20,
            ..crate::FsrsParameters::default()
        };
        MemoryService::new(MemoryServiceConfig {
            dimension: dim,
            fsrs,
            ..MemoryServiceConfig::default()
        })
        .with_embed_fn(embed)
    }

    #[tokio::test]
    async fn gated_recall_reaches_fresh_memories() {
        let dim = 32;
        let corpus = RetentionCorpus::generate(
            3,
            CorpusParams {
                topic_count: 4,
                per_topic: 5,
                dimension: dim,
                age_days: 0.0,
                era_gap_days: 0.0,
                importance: Some(1.0),
                ..CorpusParams::default()
            },
        );
        let service = service_with_w20(dim, 0.5, corpus.topic_embed_fn());
        corpus.load_into(&service).await.unwrap();

        // Fresh memories (retrievability ~1) pass the weight gate under any w20.
        let gated = measure_gated_recall(&service, &corpus.probes)
            .await
            .unwrap();
        assert!(
            (gated - 1.0).abs() < 1e-6,
            "fresh gated recall should be perfect, got {gated}"
        );
    }

    /// The w20 experiment the harness exists to run: on a heavily-aged corpus,
    /// the FSRS-5 constant we currently ship (`w20 = 0.5`) decays retrievability
    /// so fast that valid memories fall below the recall weight gate and vanish,
    /// while the correct FSRS-6 default (`w20 = 0.0658`) keeps them retrievable.
    ///
    /// This is the evidence that flipping the default is an improvement — kept as
    /// a live gate so a future default change is validated, not guessed.
    #[tokio::test]
    async fn w20_experiment_aged_recall() {
        let dim = 32;
        // One age, uniform importance, no era spread → w20 is the only variable.
        let params = CorpusParams {
            topic_count: 6,
            per_topic: 4,
            dimension: dim,
            age_days: 800.0,
            era_gap_days: 0.0,
            stability_days: 1.0,
            importance: Some(1.0),
        };

        let corpus = RetentionCorpus::generate(2024, params);

        // Ship-current (wrong) exponent.
        let svc_fast = service_with_w20(dim, 0.5, corpus.topic_embed_fn());
        corpus.load_into(&svc_fast).await.unwrap();
        let recall_fast = measure_gated_recall(&svc_fast, &corpus.probes)
            .await
            .unwrap();

        // FSRS-6 default exponent.
        let svc_slow = service_with_w20(dim, 0.0658, corpus.topic_embed_fn());
        corpus.load_into(&svc_slow).await.unwrap();
        let recall_slow = measure_gated_recall(&svc_slow, &corpus.probes)
            .await
            .unwrap();

        // The FSRS-6 exponent keeps every aged topic retrievable...
        assert!(
            recall_slow >= 0.99,
            "FSRS-6 w20 should still recall aged memories, got {recall_slow}"
        );
        // ...while the ship-current fast-decay exponent has lost some of them.
        assert!(
            recall_fast < recall_slow,
            "fast decay (w20=0.5) should lose aged recall the FSRS-6 default keeps: \
             fast={recall_fast} slow={recall_slow}"
        );
    }
}
