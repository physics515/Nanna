//! SQL-side exact k-NN over the stored embedding column.
//!
//! P13 sequences the "indexed clustering" work as: measure whether Turso's
//! `vector_distance_cos` gives correct exact k-NN over the embeddings we already
//! store (raw little-endian f32 BLOBs) *before* reaching for an external ANN
//! crate — because exact SQL k-NN trades no recall and drops the ~50k in-RAM
//! ceiling (it streams rows instead of `bulk_load`-ing every vector into RAM).
//!
//! These tests pin `MemoryRepository::search_by_embedding_sql` end to end through
//! the real dependency: its ranking must match an independent in-RAM cosine
//! ranking, its `octet_length` guard must skip differently-sized vectors (which
//! `vector_distance_cos` would otherwise error on) instead of aborting, NULL
//! embeddings must not appear, and workspace scope must match `recall_scoped`.

use nanna_storage::{NewMemory, Storage};

fn new_memory(id: &str, embedding: Option<Vec<f32>>, workspace: Option<&str>) -> NewMemory {
    NewMemory {
        memory_id: id.to_string(),
        content: format!("memory {id}"),
        embedding,
        embedding_model: Some("test".to_string()),
        session_id: None,
        metadata: None,
        tags: Vec::new(),
        workspace_id: workspace.map(str::to_string),
        fsrs_stability: 1.0,
        fsrs_difficulty: 5.0,
        fsrs_last_access: 0,
        fsrs_access_count: 0,
        fsrs_importance: 3.0,
        fsrs_storage_strength: 1.0,
        fsrs_generation: 0,
    }
}

/// Independent reference cosine distance (0 = identical … 2 = opposite), computed
/// in plain Rust so the SQL result has something external to be checked against.
fn cosine_distance(a: &[f32], b: &[f32]) -> f64 {
    assert_eq!(a.len(), b.len());
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

async fn store() -> Storage {
    Storage::in_memory().await.expect("open in-memory storage")
}

#[tokio::test]
async fn sql_knn_ranks_like_an_in_ram_cosine_scan() {
    let storage = store().await;
    let repo = storage.memories();

    // Six 4-d vectors at varying angles from the query [1,0,0,0].
    let vectors = [
        ("v-identical", vec![1.0_f32, 0.0, 0.0, 0.0]),
        ("v-near", vec![0.9, 0.1, 0.0, 0.0]),
        ("v-mid", vec![0.5, 0.5, 0.0, 0.0]),
        ("v-ortho", vec![0.0, 1.0, 0.0, 0.0]),
        ("v-far", vec![-0.5, 0.8, 0.0, 0.0]),
        ("v-opposite", vec![-1.0, 0.0, 0.0, 0.0]),
    ];
    for (id, v) in &vectors {
        repo.create(new_memory(id, Some(v.clone()), None))
            .await
            .expect("insert");
    }

    let query = [1.0_f32, 0.0, 0.0, 0.0];

    // Reference ranking: sort ids by independent cosine distance, nearest first.
    let mut expected: Vec<(&str, f64)> = vectors
        .iter()
        .map(|(id, v)| (*id, cosine_distance(&query, v)))
        .collect();
    expected.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    let expected_order: Vec<&str> = expected.iter().map(|(id, _)| *id).collect();

    let got = repo
        .search_by_embedding_sql(&query, 6, None)
        .await
        .expect("sql knn");
    let got_order: Vec<&str> = got.iter().map(|(id, _)| id.as_str()).collect();

    assert_eq!(
        got_order, expected_order,
        "SQL k-NN order must match the in-RAM cosine ranking"
    );
    // Distances must agree with the reference to f32 kernel tolerance.
    for ((id, sql_dist), (_, ref_dist)) in got.iter().zip(&expected) {
        assert!(
            (sql_dist - ref_dist).abs() < 1e-5,
            "distance for {id}: sql {sql_dist} vs ref {ref_dist}"
        );
    }
}

#[tokio::test]
async fn sql_knn_respects_limit_and_skips_null_embeddings() {
    let storage = store().await;
    let repo = storage.memories();

    for i in 0..5 {
        let v = vec![1.0_f32, i as f32 * 0.1, 0.0, 0.0];
        repo.create(new_memory(&format!("has-{i}"), Some(v), None))
            .await
            .expect("insert");
    }
    // A memory with no embedding must never appear in k-NN results.
    repo.create(new_memory("no-embedding", None, None))
        .await
        .expect("insert null");

    let query = [1.0_f32, 0.0, 0.0, 0.0];
    let got = repo
        .search_by_embedding_sql(&query, 3, None)
        .await
        .expect("sql knn");

    assert_eq!(got.len(), 3, "LIMIT must cap the result count");
    assert!(
        got.iter().all(|(id, _)| id != "no-embedding"),
        "a NULL-embedding memory must not be returned"
    );
}

#[tokio::test]
async fn sql_knn_dimension_guard_skips_mismatched_vectors() {
    let storage = store().await;
    let repo = storage.memories();

    // Same-dimension neighbours plus one 3-d vector that vector_distance_cos would
    // ERROR on if it reached the kernel. The octet_length guard must exclude it.
    repo.create(new_memory("d4-a", Some(vec![1.0, 0.0, 0.0, 0.0]), None))
        .await
        .expect("insert");
    repo.create(new_memory("d4-b", Some(vec![0.0, 1.0, 0.0, 0.0]), None))
        .await
        .expect("insert");
    repo.create(new_memory("d3-wrong", Some(vec![1.0, 0.0, 0.0]), None))
        .await
        .expect("insert");

    let query = [1.0_f32, 0.0, 0.0, 0.0];
    let got = repo
        .search_by_embedding_sql(&query, 10, None)
        .await
        .expect("sql knn must not error on a mixed-dimension store");

    let ids: Vec<&str> = got.iter().map(|(id, _)| id.as_str()).collect();
    assert!(ids.contains(&"d4-a") && ids.contains(&"d4-b"));
    assert!(
        !ids.contains(&"d3-wrong"),
        "the 3-d vector must be filtered out, not error the scan"
    );
}

#[tokio::test]
async fn sql_knn_workspace_scope_matches_recall_semantics() {
    let storage = store().await;
    let repo = storage.memories();

    repo.create(new_memory("global", Some(vec![1.0, 0.0, 0.0, 0.0]), None))
        .await
        .expect("insert");
    repo.create(new_memory(
        "ws-a",
        Some(vec![0.9, 0.1, 0.0, 0.0]),
        Some("A"),
    ))
    .await
    .expect("insert");
    repo.create(new_memory(
        "ws-b",
        Some(vec![0.8, 0.2, 0.0, 0.0]),
        Some("B"),
    ))
    .await
    .expect("insert");

    let query = [1.0_f32, 0.0, 0.0, 0.0];

    // Some(A): global + workspace A, never workspace B.
    let scoped = repo
        .search_by_embedding_sql(&query, 10, Some("A"))
        .await
        .expect("scoped knn");
    let scoped_ids: Vec<&str> = scoped.iter().map(|(id, _)| id.as_str()).collect();
    assert!(scoped_ids.contains(&"global") && scoped_ids.contains(&"ws-a"));
    assert!(
        !scoped_ids.contains(&"ws-b"),
        "workspace B must be out of scope A"
    );

    // None: all memories.
    let all = repo
        .search_by_embedding_sql(&query, 10, None)
        .await
        .expect("global knn");
    assert_eq!(all.len(), 3, "None scope returns all memories");
}
