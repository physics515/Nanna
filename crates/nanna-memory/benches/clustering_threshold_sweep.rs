//! What raising the clustering bar actually costs.
//!
//! Run with: `cargo bench -p nanna-memory --bench clustering_threshold_sweep`
//!
//! The 2026 agent-memory literature puts the clustering similarity bar around
//! **θ_sim ≈ 0.7**; after the 2026-09-09 similarity-veto fix Nanna's composite
//! still only demands **cosine 0.10**, because the non-semantic floor (0.50)
//! consumes most of the 0.55 threshold. Closing that gap is a
//! compression-versus-fidelity trade, and this is the instrument for choosing
//! where to sit on it rather than picking a number.
//!
//! Two things to know about reading the output:
//!
//! - **`min cosine` is the honest axis**, not `threshold`. The threshold is
//!   judged against the *composite* score, so 0.55 demands only cosine 0.10.
//!   `ConsolidationConfig::min_required_similarity()` converts one to the
//!   other, and it is what compares to the literature's numbers.
//! - **The tight corpus is a control, and it should be flat.** Its members sit
//!   at cosine ~0.999, above the `IngestAction::Reinforce` line, so dream phase
//!   (b) folds them with zero summarizer calls and clustering never runs. A
//!   sweep that moved on *that* corpus would mean the threshold was reaching
//!   something it should not.
//!
//! Deterministic: fixed-seed corpus, echo summarizer, no network, no clock
//! dependence beyond the corpus's own aging.

use nanna_memory::retention::{CorpusParams, RetentionCorpus, run_retention_cycle};
use nanna_memory::{ConsolidationConfig, MemoryService, MemoryServiceConfig};

/// Echo summarizer: re-emits the cluster's topic tag so the consolidated entry
/// is deterministically re-embeddable and recall stays measurable.
async fn echo_summarize(prompt: String) -> Result<String, String> {
    let topic = prompt
        .find("topic:")
        .and_then(|i| prompt[i + 6..].split(|c: char| !c.is_ascii_digit()).next())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(topic.map_or_else(
        || "untagged consolidated".to_string(),
        |t| format!("topic:{t} consolidated"),
    ))
}

async fn sweep(label: &str, member_spread: f32) {
    let dim = 32;
    let params = CorpusParams {
        topic_count: 6,
        per_topic: 10,
        dimension: dim,
        member_spread,
        ..CorpusParams::default()
    };

    println!("\n-- {label} (member_spread {member_spread}) --");
    println!(
        "{:>10} {:>11} {:>9} {:>8} {:>8} {:>12} {:>10}",
        "threshold", "min cosine", "clusters", "merged", "deduped", "compression", "recall"
    );

    for &threshold in &[0.55_f32, 0.65, 0.75, 0.85, 0.95] {
        let corpus = RetentionCorpus::generate(11, params);
        let service = MemoryService::new(MemoryServiceConfig {
            dimension: dim,
            ..MemoryServiceConfig::default()
        })
        .with_embed_fn(corpus.topic_embed_fn());
        corpus.load_into(&service).await.expect("seed corpus");

        let consolidation = ConsolidationConfig {
            cluster_threshold: threshold,
            min_remaining_memories: 1,
            max_compression_ratio: 0.9,
            ..ConsolidationConfig::default()
        };
        // Every swept point must be a configuration we would actually allow.
        consolidation
            .validate()
            .expect("swept threshold must satisfy the clustering invariant");

        let (report, result) =
            run_retention_cycle(&service, &corpus.probes, 3, &consolidation, echo_summarize)
                .await
                .expect("retention cycle");

        println!(
            "{threshold:>10.2} {:>11.2} {:>9} {:>8} {:>8} {:>12.3} {:>10.3}",
            consolidation.min_required_similarity(),
            result.clusters_formed,
            result.memories_merged,
            result.memories_deduped,
            report.compression_ratio(),
            report.recall_retention(),
        );
    }
}

fn main() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    println!("clustering threshold sweep — compression vs fidelity");
    println!(
        "(deterministic fixed-seed corpus; `min cosine` is the axis to compare with the literature)"
    );

    rt.block_on(async {
        // The regime the decision is about: related but not near-identical.
        sweep("loose corpus — clustering decides", 0.6).await;
        // Control: phase (b) owns this corpus, so the sweep should be flat.
        sweep(
            "tight corpus — phase (b) folds, clustering never runs",
            0.02,
        )
        .await;
    });

    println!(
        "\nThe tight arm is a control and should not move. Movement there would mean the\n\
         clustering threshold is reaching pairs the dedup phase already owns."
    );
}
