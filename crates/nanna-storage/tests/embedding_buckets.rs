//! Per-model embedding buckets and the durable queue.
//!
//! The property under test throughout: **a model's vectors are never destroyed
//! by another model's**. Before buckets, adopting a new embedder overwrote the
//! single vector column, so a provider that flapped away and back cost two full
//! re-embeds of the entire store — and a provider that failed mid-session left
//! the store with no usable index at all.

use nanna_storage::{NewMemory, NewMemoryChunk, Storage, StorageConfig, ROW_ORDINAL};

fn temp_db_path(tag: &str) -> String {
    let dir = std::env::temp_dir().join(format!(
        "nanna_buckets_{tag}_{}_{:p}",
        std::process::id(),
        &tag as *const _
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.join("mem.db").to_string_lossy().to_string()
}

async fn open(db_path: &str) -> Storage {
    Storage::new(&StorageConfig {
        path: db_path.to_string(),
    })
    .await
    .expect("storage opens")
}

fn memory(id: &str) -> NewMemory {
    NewMemory {
        memory_id: id.to_string(),
        content: format!("content of {id}"),
        embedding: None,
        embedding_model: None,
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

/// Scan the database AND its write-ahead log.
///
/// turso 0.6.1 keeps every committed page in the WAL and does not checkpoint on
/// close, so a test that reads only the `.db` file proves nothing — the bytes
/// it is looking for have simply not been written there yet.
async fn file_contains(db_path: &str, needle: &[u8]) -> bool {
    let bytes = std::fs::read(db_path).unwrap_or_default();
    let wal = std::fs::read(format!("{db_path}-wal")).unwrap_or_default();
    bytes.windows(needle.len()).any(|w| w == needle)
        || wal.windows(needle.len()).any(|w| w == needle)
}

/// A distinctive vector whose little-endian bytes will not occur by chance.
fn sentinel() -> Vec<f32> {
    (0..96).map(|i| (i as f32) * 7.531_9 + 3.140_1 + 404.5).collect()
}

fn chunk(parent: &str, ordinal: i64, text: &str) -> NewMemoryChunk {
    NewMemoryChunk {
        memory_id: parent.to_string(),
        ordinal,
        content: text.to_string(),
        char_start: ordinal * 100,
        char_end: ordinal * 100 + text.len() as i64,
        embedding: None,
        embedding_model: None,
        chunk_max_chars: 3200,
        chunker_version: 1,
        workspace_id: None,
    }
}

/// Two models' vectors coexist on one memory. This is the entire reason the
/// table exists: the previous single-column layout meant adopting a new model
/// destroyed the old one's work, so returning to a provider after an outage
/// cost a full re-embed instead of a lookup.
#[tokio::test]
async fn two_models_hold_separate_vectors_for_one_memory() {
    let db = temp_db_path("coexist");
    let storage = open(&db).await;
    let repo = storage.memories();
    repo.create(memory("m1")).await.expect("create");

    repo.put_memory_vector("m1", "openrouter:nemotron", &[1.0, 0.0, 0.0, 0.0])
        .await
        .expect("first bucket");
    repo.put_memory_vector("m1", "ollama:nomic", &[0.0, 1.0, 0.0])
        .await
        .expect("second bucket, different width");

    let mut buckets = repo.memory_vectors("m1").await.expect("read");
    buckets.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(buckets.len(), 2, "both models must survive");
    assert_eq!(buckets[0].0, "ollama:nomic");
    assert_eq!(buckets[0].1, vec![0.0, 1.0, 0.0], "widths may differ per bucket");
    assert_eq!(buckets[1].0, "openrouter:nemotron");
    assert_eq!(buckets[1].1, vec![1.0, 0.0, 0.0, 0.0]);
}

/// Re-embedding under the same model replaces that bucket only.
#[tokio::test]
async fn rewriting_one_bucket_leaves_the_others_intact() {
    let db = temp_db_path("rewrite");
    let storage = open(&db).await;
    let repo = storage.memories();
    repo.create(memory("m1")).await.expect("create");

    repo.put_memory_vector("m1", "a", &[1.0, 0.0]).await.expect("a");
    repo.put_memory_vector("m1", "b", &[0.0, 1.0]).await.expect("b");
    repo.put_memory_vector("m1", "a", &[0.5, 0.5]).await.expect("a again");

    let buckets = repo.memory_vectors("m1").await.expect("read");
    assert_eq!(buckets.len(), 2, "an update must not add a row");
    let a = buckets.iter().find(|(m, _)| m == "a").expect("a present");
    let b = buckets.iter().find(|(m, _)| m == "b").expect("b present");
    assert_eq!(a.1, vec![0.5, 0.5], "a was replaced");
    assert_eq!(b.1, vec![0.0, 1.0], "b was untouched");
}

/// The census drives bucket selection, so it has to be ordered and accurate.
#[tokio::test]
async fn the_bucket_census_counts_per_model_largest_first() {
    let db = temp_db_path("census");
    let storage = open(&db).await;
    let repo = storage.memories();
    for i in 0..5 {
        repo.create(memory(&format!("m{i}"))).await.expect("create");
        repo.put_memory_vector(&format!("m{i}"), "big", &[1.0, 0.0])
            .await
            .expect("big");
    }
    repo.put_memory_vector("m0", "small", &[0.0, 1.0]).await.expect("small");

    let census = repo.bucket_counts().await.expect("census");
    assert_eq!(census[0], ("big".to_string(), 5), "largest bucket first");
    assert_eq!(census[1], ("small".to_string(), 1));
}

/// A queue that lives only in RAM is rebuilt from scratch on every restart and
/// forgets what it already tried. This one is a table, so pending work and its
/// failure history survive the process.
#[tokio::test]
async fn queued_work_survives_and_carries_its_failure_reason() {
    let db = temp_db_path("queue");
    let storage = open(&db).await;
    let repo = storage.memories();
    repo.create(memory("m1")).await.expect("create");

    repo.enqueue_embedding("m1", ROW_ORDINAL, "ollama:nomic").await.expect("enqueue");
    repo.enqueue_embedding("m1", 0, "ollama:nomic").await.expect("enqueue chunk");

    let pending = repo.pending_embeddings("ollama:nomic", 10).await.expect("pending");
    assert_eq!(pending.len(), 2, "row work and chunk work share one queue");

    repo.record_embedding_failure("m1", 0, "ollama:nomic", "402 Insufficient credits")
        .await
        .expect("record");
    let health = repo.embedding_queue_health().await.expect("health");
    assert_eq!(health[0].0, "ollama:nomic");
    assert_eq!(health[0].1, 2, "depth is visible");
    assert!(
        health[0].2.as_deref().unwrap_or("").contains("402"),
        "the reason must survive, or a stuck queue is indistinguishable from a slow one"
    );

    repo.dequeue_embedding("m1", 0, "ollama:nomic").await.expect("dequeue");
    assert_eq!(
        repo.pending_embeddings("ollama:nomic", 10).await.expect("pending").len(),
        1
    );
}

/// Work is queued per model, so switching embedders does not disturb another
/// model's outstanding work.
#[tokio::test]
async fn the_queue_is_keyed_by_model() {
    let db = temp_db_path("queue_model");
    let storage = open(&db).await;
    let repo = storage.memories();
    repo.create(memory("m1")).await.expect("create");

    repo.enqueue_embedding("m1", 0, "model-a").await.expect("a");
    repo.enqueue_embedding("m1", 0, "model-b").await.expect("b");

    assert_eq!(repo.pending_embeddings("model-a", 10).await.expect("a").len(), 1);
    repo.dequeue_embedding("m1", 0, "model-a").await.expect("dequeue a");
    assert_eq!(repo.pending_embeddings("model-a", 10).await.expect("a").len(), 0);
    assert_eq!(
        repo.pending_embeddings("model-b", 10).await.expect("b").len(),
        1,
        "finishing one model's work must not clear another's"
    );
}

/// Re-enqueueing must not reset the attempt count, or an item that fails
/// forever could disguise itself as fresh work and hold the front of the queue.
#[tokio::test]
async fn re_enqueueing_does_not_erase_attempt_history() {
    let db = temp_db_path("requeue");
    let storage = open(&db).await;
    let repo = storage.memories();
    repo.create(memory("m1")).await.expect("create");
    repo.create(memory("m2")).await.expect("create");

    repo.enqueue_embedding("m1", 0, "m").await.expect("enqueue");
    repo.record_embedding_failure("m1", 0, "m", "boom").await.expect("fail");
    repo.record_embedding_failure("m1", 0, "m", "boom").await.expect("fail");
    repo.enqueue_embedding("m1", 0, "m").await.expect("re-enqueue");
    repo.enqueue_embedding("m2", 0, "m").await.expect("fresh work");

    let pending = repo.pending_embeddings("m", 10).await.expect("pending");
    assert_eq!(
        pending[0].0, "m2",
        "fresh work outranks an item that has already failed twice"
    );
}

/// Nothing sets `PRAGMA foreign_keys`, so the cascade is ours to write. An
/// orphaned bucket is worse than an orphaned chunk: it is a live vector for
/// text that no longer exists, and it keeps matching queries forever.
#[tokio::test]
async fn deleting_a_memory_takes_its_buckets_and_queue_with_it() {
    let db = temp_db_path("cascade");
    let storage = open(&db).await;
    let repo = storage.memories();
    repo.create(memory("m1")).await.expect("create");
    repo.replace_chunks("m1", &[chunk("m1", 0, "a")]).await.expect("chunks");
    repo.put_memory_vector("m1", "model", &[1.0, 0.0]).await.expect("row vector");
    repo.put_chunk_vector("m1", 0, "model", &[0.0, 1.0]).await.expect("chunk vector");
    repo.enqueue_embedding("m1", 0, "model").await.expect("queue");

    assert!(repo.delete("m1").await.expect("delete"));

    assert!(repo.memory_vectors("m1").await.expect("read").is_empty());
    assert_eq!(repo.bucket_counts().await.expect("census").len(), 0);
    assert!(repo.pending_embeddings("model", 10).await.expect("pending").is_empty());
}

/// Ghost Vectors (arXiv 2606.18497): an embedding inverts back to its source
/// text, so every bucket is an independent copy of the memory. Zeroing one and
/// leaving another reconstructs the memory just as well as leaving the first.
#[tokio::test]
async fn deleting_a_memory_zeroes_every_bucket_not_just_one() {
    let db = temp_db_path("ghost");
    let storage = open(&db).await;
    let repo = storage.memories();
    repo.create(memory("m1")).await.expect("create");
    repo.replace_chunks("m1", &[chunk("m1", 0, "a")]).await.expect("chunks");

    let secret = sentinel();
    repo.put_memory_vector("m1", "model-a", &secret).await.expect("a");
    repo.put_memory_vector("m1", "model-b", &secret).await.expect("b");
    repo.put_chunk_vector("m1", 0, "model-a", &secret).await.expect("chunk a");

    let needle: Vec<u8> = secret.iter().flat_map(|f| f.to_le_bytes()).collect();
    assert!(
        file_contains(&db, &needle).await,
        "precondition: the vector is on disk before the delete"
    );

    repo.delete("m1").await.expect("delete");

    assert!(
        !file_contains(&db, &needle).await,
        "a surviving bucket inverts back to the deleted memory's text — zeroing          one model's vector while another remains is not a delete"
    );
}
