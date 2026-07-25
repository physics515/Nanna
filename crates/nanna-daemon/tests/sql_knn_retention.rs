//! SQL-side exact k-NN parity on a realistic, topic-clustered corpus.
//!
//! `crates/nanna-storage/tests/vector_knn.rs` proves `search_by_embedding_sql`
//! ranks a hand-built spread of vectors correctly. This raises the bar to the
//! **realistic** distribution P13's measurement cares about: the memory crate's
//! `RetentionCorpus` generator (8 topic centroids × 12 jittered members, 64-dim),
//! persisted to Turso, queried with the same centroid probes the recall harness
//! uses. It asserts the SQL nearest neighbour is the *same memory* an independent
//! in-RAM cosine scan picks, and that it belongs to the probe's own topic — i.e.
//! exact SQL k-NN is a faithful, drop-in stand-in for the in-RAM scan on real
//! embeddings, which is the feasibility gate before wiring it into recall.

use nanna_memory::retention::{CorpusParams, RetentionCorpus};
use nanna_storage::{NewMemory, Storage};

fn to_new_memory(id: &str, embedding: Vec<f32>) -> NewMemory {
    NewMemory {
        memory_id: id.to_string(),
        content: String::new(),
        embedding: Some(embedding),
        embedding_model: Some("retention".to_string()),
        session_id: None,
        metadata: None,
        tags: Vec::new(),
        workspace_id: None,
        fsrs_stability: 1.0,
        fsrs_difficulty: 5.0,
        fsrs_last_access: 0,
        fsrs_access_count: 0,
        fsrs_importance: 3.0,
        fsrs_storage_strength: 1.0,
        fsrs_generation: 0,
    }
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let na: f64 = a
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    let nb: f64 = b
        .iter()
        .map(|x| f64::from(*x) * f64::from(*x))
        .sum::<f64>()
        .sqrt();
    1.0 - dot / (na * nb)
}

#[tokio::test]
async fn sql_knn_matches_in_ram_scan_on_a_topic_clustered_corpus() {
    let corpus = RetentionCorpus::generate(42, CorpusParams::default());
    assert!(corpus.memories.len() >= 32, "corpus should be non-trivial");
    assert!(!corpus.probes.is_empty(), "corpus should have probes");

    let storage = Storage::in_memory().await.expect("open storage");
    let repo = storage.memories();

    // Persist every memory, and keep an id → (embedding, topic) map for the
    // independent in-RAM reference and the topic check.
    let mut reference: Vec<(String, Vec<f32>, String)> = Vec::with_capacity(corpus.memories.len());
    for mem in &corpus.memories {
        repo.create(to_new_memory(&mem.id, mem.embedding.clone()))
            .await
            .expect("insert memory");
        let topic = mem.metadata.get("topic").cloned().unwrap_or_default();
        reference.push((mem.id.clone(), mem.embedding.clone(), topic));
    }

    for probe in &corpus.probes {
        // SQL exact k-NN.
        let sql = repo
            .search_by_embedding_sql(&probe.query_embedding, 5, None)
            .await
            .expect("sql knn");
        assert!(!sql.is_empty(), "every probe must return neighbours");

        // Independent in-RAM ranking over the same embeddings.
        let mut in_ram: Vec<(&str, f64)> = reference
            .iter()
            .map(|(id, emb, _)| (id.as_str(), cosine_distance(&probe.query_embedding, emb)))
            .collect();
        in_ram.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        // The nearest neighbour must be the same memory (centroid probes are well
        // separated, so the top-1 is unambiguous — no tie-break flake).
        assert_eq!(
            sql[0].0, in_ram[0].0,
            "SQL and in-RAM must agree on the nearest memory for topic {}",
            probe.topic
        );

        // And that memory must belong to the probe's own topic (recall is correct,
        // not merely self-consistent).
        let top_topic = reference
            .iter()
            .find(|(id, _, _)| *id == sql[0].0)
            .map(|(_, _, topic)| topic.as_str())
            .unwrap_or("");
        assert_eq!(
            top_topic, probe.topic,
            "the nearest memory should be in the probe's topic cluster"
        );
    }
}
