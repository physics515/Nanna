//! Dream-time clustering: how `cluster_memories` scales with store size.
//!
//! Run with: `cargo bench -p nanna-memory --bench clustering_scaling`
//!
//! **Why this exists.** The roadmap's headline P13 item replaces the greedy
//! single-pass `cluster_memories()` with ANN candidate neighbours, on the
//! stated grounds that the current pass is O(N²) and "scales past the ~50k
//! in-RAM ceiling". That ceiling was an estimate, never a measurement. This
//! benchmark is the baseline any ANN replacement has to beat, and it measures
//! the thing that actually matters: **wall-clock for one dream cycle's
//! clustering pass**, plus the pair count behind it.
//!
//! Two properties make the numbers trustworthy:
//!
//! - **Deterministic corpus.** Vectors come from a fixed-seed integer hash, not
//!   an RNG, so the same N produces byte-identical input on any host and a
//!   later comparison against an ANN implementation is exact rather than
//!   statistical.
//! - **Pair counting is exact, not sampled.** `pairs_considered` counts the
//!   inner-loop iterations the greedy pass actually performs. It is the
//!   hardware-independent half of the result — a number an ANN version can be
//!   held to even off the reference tier.
//!
//! The corpus is built as `topics` tight clusters so clustering has real work
//! to do; a corpus of mutually dissimilar vectors would make every memory a
//! singleton and understate the cost.

use std::collections::HashMap;
use std::time::Instant;

use nanna_memory::{
    ConsolidationConfig, cluster_memories, cluster_score_ceiling, composite_cluster_score,
};
use nanna_memory::{FsrsState, MemoryEntry};

/// Embedding width — MiniLM-class, matching the local embedder the memory path
/// is heading for (`all-minilm-l6-v2` is 384-dim).
const DIM: usize = 384;

/// Deterministic unit-norm vector near a topic centroid.
///
/// Integer hash rather than an RNG so the corpus is reproducible across hosts
/// and runs; `spread` controls how tight a topic is.
fn vector_for(topic: usize, index: usize, spread: f32) -> Vec<f32> {
    let mut v = Vec::with_capacity(DIM);
    let mut norm_sq = 0.0f32;
    for d in 0..DIM {
        // Topic component: identical for every member of a topic.
        let t = ((topic.wrapping_mul(2_654_435_761) ^ d.wrapping_mul(40_503)) % 1000) as f32
            / 1000.0
            - 0.5;
        // Member jitter: small, deterministic, unique per (topic, index, d).
        let j = ((index.wrapping_mul(2_246_822_519) ^ d.wrapping_mul(3_266_489_917)) % 1000) as f32
            / 1000.0
            - 0.5;
        let x = t + j * spread;
        norm_sq += x * x;
        v.push(x);
    }
    let norm = norm_sq.sqrt().max(f32::MIN_POSITIVE);
    for x in &mut v {
        *x /= norm;
    }
    v
}

/// A deterministic pseudo-random unit vector, unrelated to every other one.
///
/// Two independent unit vectors in 384 dimensions have expected cosine 0 with
/// standard deviation ~1/sqrt(384) ≈ 0.051, so essentially none of them clear
/// the semantic bar the shipped config demands. That is what makes this the
/// *sparse* corpus: no cluster ever fills, so `max_cluster_memories` never
/// breaks the inner loop and every seed scans to the end — the quadratic
/// regime an ANN candidate set is meant to remove.
fn unrelated_vector(index: usize) -> Vec<f32> {
    // xorshift over a per-(index, dim) seed: reproducible on any host, and
    // enough decorrelation between indices to keep the vectors independent.
    let mut v = Vec::with_capacity(DIM);
    let mut norm_sq = 0.0f32;
    for d in 0..DIM {
        let mut x = (index as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (d as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        let f = (x % 2_000_000) as f32 / 1_000_000.0 - 1.0; // -1.0 ..= 1.0
        norm_sq += f * f;
        v.push(f);
    }
    let norm = norm_sq.sqrt().max(f32::MIN_POSITIVE);
    for x in &mut v {
        *x /= norm;
    }
    v
}

fn corpus(count: usize, topics: usize, spread: f32, related: bool) -> Vec<MemoryEntry> {
    (0..count)
        .map(|i| {
            let topic = i % topics;
            MemoryEntry {
                id: format!("m{i}"),
                content: format!("memory {i} about topic {topic}"),
                embedding: if related {
                    vector_for(topic, i, spread)
                } else {
                    unrelated_vector(i)
                },
                embedding_model: None,
                embeddings: HashMap::new(),
                metadata: HashMap::new(),
                timestamp: 1_700_000_000 + i as i64,
                fsrs: FsrsState::default(),
                workspace_id: None,
            }
        })
        .collect()
}

/// The inner-loop iteration count the greedy pass performs, computed by
/// replaying its own admission rules.
///
/// Kept separate from the timed call so measuring costs nothing at bench time,
/// and so the number stays exact rather than inferred from N².
fn pairs_considered(memories: &[MemoryEntry], config: &ConsolidationConfig) -> u64 {
    let cap_count = config.max_cluster_memories.max(1);
    let cap_bytes = config.max_cluster_content_bytes;
    let mut assigned = vec![false; memories.len()];
    let mut pairs: u64 = 0;

    for i in 0..memories.len() {
        if assigned[i] {
            continue;
        }
        let mut len = 1usize;
        let mut bytes = memories[i].content.len();
        assigned[i] = true;
        for j in (i + 1)..memories.len() {
            if len >= cap_count {
                break;
            }
            if assigned[j] {
                continue;
            }
            pairs += 1;
            let score =
                composite_cluster_score(&memories[i], &memories[j], &config.clustering_weights);
            if score < config.cluster_threshold {
                continue;
            }
            if bytes.saturating_add(memories[j].content.len()) > cap_bytes {
                continue;
            }
            bytes += memories[j].content.len();
            len += 1;
            assigned[j] = true;
        }
    }
    pairs
}

fn main() {
    let config = ConsolidationConfig::default();
    println!(
        "clustering baseline — dim {DIM}, threshold {:.2}, max_cluster_memories {}",
        config.cluster_threshold, config.max_cluster_memories
    );
    println!("(deterministic fixed-seed corpus; pairs_considered is hardware-independent)\n");
    println!(
        "{:>8} {:>8} {:>10} {:>14} {:>12} {:>12}",
        "N", "topics", "clusters", "pairs", "wall_ms", "ns_per_pair"
    );

    // DENSE: memories fall into a modest number of tight topics, so a seed
    // fills `max_cluster_memories` and breaks out of the inner loop early.
    println!("-- dense: N/20 topics (clusters are easy to fill) --");
    for &(count, topics) in &[
        (1_000usize, 50usize),
        (2_000, 100),
        (4_000, 200),
        (8_000, 400),
        (16_000, 800),
    ] {
        run_case(count, topics, 0.25, true, &config);
    }

    // SPARSE: every memory is its own topic, so almost nothing clears the
    // threshold, no cluster ever fills, and each seed scans to the end. This
    // is the quadratic regime — and the one an ANN candidate set removes.
    println!("\n-- sparse: mutually unrelated vectors (nothing clusters) --");
    for &count in &[1_000usize, 2_000, 4_000, 8_000, 16_000] {
        run_case(count, count, 0.25, false, &config);
    }

    // AGED: the sparse regime again, but at the timescale and the FSRS spread
    // production actually presents.
    //
    // The two arms above leave `time_span_minutes` at its 1440-minute default
    // while spacing memories one second apart, so the age term never leaves the
    // top of its range — a configuration `MemoryService` never uses. Its
    // `with_store_timescale` sets the span to the store's own oldest-to-newest
    // gap, which is what makes a mature store discriminate at week scale. FSRS
    // state is varied too, since `FsrsState::default()` pins `access_count` at
    // 0 and `importance` at 1.0 for every memory, which makes the recall and
    // importance terms identically 1.0 by construction.
    println!(
        "\n-- aged: sparse vectors at the production timescale (spans 90 days, varied FSRS) --"
    );
    for &count in &[1_000usize, 2_000, 4_000, 8_000, 16_000] {
        run_aged_case(count, &config);
    }

    println!(
        "\nRead the `pairs` column, not just wall_ms: it is the quantity an ANN\n\
         candidate set replaces, and it is the same on every machine.\n\
         The two regimes are the whole point. Cost is governed by MATCH DENSITY,\n\
         not by N: `max_cluster_memories` bounds the inner loop only when matches\n\
         are plentiful enough to fill a cluster. A store of largely-unrelated\n\
         memories is the quadratic case, and it is the realistic one for a\n\
         long-lived personal store that has already been consolidated."
    );
}

/// A long-lived store: sparse vectors, spread over 90 days, with FSRS state
/// that actually varies between memories.
fn aged_corpus(count: usize) -> Vec<MemoryEntry> {
    const SPAN_SECS: i64 = 90 * 24 * 3600;
    (0..count)
        .map(|i| {
            let mut entry = MemoryEntry {
                id: format!("m{i}"),
                content: format!("memory {i}"),
                embedding: unrelated_vector(i),
                embedding_model: None,
                embeddings: HashMap::new(),
                metadata: HashMap::new(),
                // Spread evenly across the whole span, so pair age gaps cover
                // the full 0..1 range of `age_proximity`.
                timestamp: 1_700_000_000 + (i as i64) * SPAN_SECS / (count.max(1) as i64),
                fsrs: FsrsState::default(),
                workspace_id: None,
            };
            entry.fsrs.access_count = (i % 17) as u32;
            entry.fsrs.importance = 1.0 + (i % 5) as f32;
            entry
        })
        .collect()
}

/// `config` with the clustering timescale taken from the corpus, exactly as
/// `MemoryService::with_store_timescale` does it in production.
fn with_store_timescale(memories: &[MemoryEntry], config: &ConsolidationConfig) -> ConsolidationConfig {
    let mut config = config.clone();
    let span_minutes = match (
        memories.iter().map(|e| e.timestamp).min(),
        memories.iter().map(|e| e.timestamp).max(),
    ) {
        (Some(first), Some(last)) => (last - first) as f32 / 60.0,
        _ => 0.0,
    };
    config.clustering_weights.time_span_minutes = span_minutes.max(1.0);
    config
}

/// Time the aged corpus and print its row, with the pruned-pair count.
fn run_aged_case(count: usize, base: &ConsolidationConfig) {
    let memories = aged_corpus(count);
    let config = with_store_timescale(&memories, base);
    let pairs = pairs_considered(&memories, &config);
    let pruned = pairs_pruned(&memories, &config);

    let start = Instant::now();
    let clusters = cluster_memories(&memories, &config);
    drop(memories);
    let elapsed = start.elapsed();

    let wall_ms = elapsed.as_secs_f64() * 1000.0;
    let ns_per_pair = if pairs == 0 {
        0.0
    } else {
        elapsed.as_secs_f64() * 1e9 / pairs as f64
    };
    let pct = if pairs == 0 {
        0.0
    } else {
        pruned as f64 * 100.0 / pairs as f64
    };
    println!(
        "{count:>8} {count:>8} {:>10} {pairs:>14} {wall_ms:>12.1} {ns_per_pair:>12.1}           pruned {pruned:>12} ({pct:.1}%)",
        clusters.len()
    );
}

/// How many of the considered pairs the ceiling rejects before any cosine.
///
/// Hardware-independent, like `pairs_considered`, and computed outside the
/// timed region. Replays the same admission rules; `composite_cluster_score`
/// (unpruned) decides membership so the replay matches the real pass.
fn pairs_pruned(memories: &[MemoryEntry], config: &ConsolidationConfig) -> u64 {
    let weights = &config.clustering_weights;
    let total = weights.total();
    let cap_count = config.max_cluster_memories.max(1);
    let cap_bytes = config.max_cluster_content_bytes;
    let mut assigned = vec![false; memories.len()];
    let mut pruned: u64 = 0;

    for i in 0..memories.len() {
        if assigned[i] {
            continue;
        }
        let mut len = 1usize;
        let mut bytes = memories[i].content.len();
        assigned[i] = true;
        for j in (i + 1)..memories.len() {
            if len >= cap_count {
                break;
            }
            if assigned[j] {
                continue;
            }
            if total > 0.0 && cluster_score_ceiling(&memories[i], &memories[j], weights) < config.cluster_threshold {
                pruned += 1;
                continue;
            }
            let score =
                composite_cluster_score(&memories[i], &memories[j], &config.clustering_weights);
            if score < config.cluster_threshold {
                continue;
            }
            if bytes.saturating_add(memories[j].content.len()) > cap_bytes {
                continue;
            }
            bytes += memories[j].content.len();
            len += 1;
            assigned[j] = true;
        }
    }
    pruned
}

/// Time one corpus shape and print its row.
fn run_case(count: usize, topics: usize, spread: f32, related: bool, config: &ConsolidationConfig) {
    let memories = corpus(count, topics, spread, related);
    let pairs = pairs_considered(&memories, config);

    let start = Instant::now();
    let clusters = cluster_memories(&memories, config);
    // Dropped inside the timed region, where the by-value signature dropped it.
    drop(memories);
    let elapsed = start.elapsed();

    let wall_ms = elapsed.as_secs_f64() * 1000.0;
    let ns_per_pair = if pairs == 0 {
        0.0
    } else {
        elapsed.as_secs_f64() * 1e9 / pairs as f64
    };
    println!(
        "{count:>8} {topics:>8} {:>10} {pairs:>14} {wall_ms:>12.1} {ns_per_pair:>12.1}",
        clusters.len()
    );
}
