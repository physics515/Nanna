//! Changing a memory's content invalidates its stored vector.
//!
//! A vector describes the words that produced it. A row whose content moved on
//! while its vector did not is findable by text it no longer contains — and
//! silently, because nothing about the row looks wrong. The in-RAM half of this
//! was fixed in 2026-08; these tests cover the durable half, without which a
//! restart reloads the stale vector from disk and undoes it.

#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

use nanna_storage::{NewMemory, Storage};

async fn storage() -> Storage {
    Storage::in_memory().await.expect("in-memory storage")
}

fn embedded(id: &str, content: &str) -> NewMemory {
    NewMemory {
        memory_id: id.to_string(),
        content: content.to_string(),
        embedding: Some(vec![0.25, 0.5, 0.75]),
        embedding_model: Some("test-embedder".to_string()),
        session_id: None,
        metadata: None,
        tags: Vec::new(),
        workspace_id: None,
        fsrs_stability: 1.0,
        fsrs_difficulty: 5.0,
        fsrs_last_access: 0,
        fsrs_access_count: 0,
        fsrs_importance: 0.5,
        fsrs_storage_strength: 1.0,
        fsrs_generation: 0,
    }
}

#[tokio::test]
async fn updating_content_drops_the_vector_that_described_the_old_text() {
    let storage = storage().await;
    let memories = storage.memories();
    memories
        .create(embedded("m1", "the harbor light was green"))
        .await
        .expect("created");

    let before = memories.get("m1").await.expect("read back");
    assert!(
        before.embedding.is_some_and(|e| !e.is_empty()),
        "the fixture starts with a vector, or the test proves nothing"
    );

    memories
        .update_content("m1", "the harbor light was red")
        .await
        .expect("content updated");

    // This is the durable half: re-reading the row is what a restart does.
    let after = memories.get("m1").await.expect("read back");
    assert_eq!(after.content, "the harbor light was red");
    assert!(
        after.embedding.is_none_or(|e| e.is_empty()),
        "the vector still described 'green' and must not survive the rewrite"
    );
    assert!(
        after.embedding_model.is_none(),
        "a model stamp with no vector would make the row look embedded"
    );
}

#[tokio::test]
async fn the_cleared_row_reads_as_queued_for_backfill() {
    // `NULL` is not a special case invented here: it is the state the loader
    // already counts as awaiting embedding, so the drain re-embeds the new text
    // and the row is unsearchable rather than wrongly searchable meanwhile.
    let storage = storage().await;
    let memories = storage.memories();
    memories
        .create(embedded("m1", "original text"))
        .await
        .expect("created");
    memories
        .update_content("m1", "replacement text")
        .await
        .expect("updated");

    let loaded = memories.bulk_load().await.expect("bulk load");
    let row = loaded
        .iter()
        .find(|m| m.memory_id == "m1")
        .expect("the row is still there");
    assert_eq!(row.content, "replacement text", "the content is intact");
    assert!(
        row.embedding.as_ref().is_none_or(Vec::is_empty),
        "and it loads as awaiting a vector"
    );
}

#[tokio::test]
async fn a_fresh_vector_written_after_the_update_survives() {
    // The save path clears content then writes the new embedding. The clear
    // must not defeat the write that follows it.
    let storage = storage().await;
    let memories = storage.memories();
    memories
        .create(embedded("m1", "original text"))
        .await
        .expect("created");
    memories
        .update_content("m1", "replacement text")
        .await
        .expect("updated");

    memories
        .update_embedding("m1", &[1.0, 2.0, 3.0], Some("test-embedder"))
        .await
        .expect("re-embedded");

    let after = memories.get("m1").await.expect("read back");
    assert_eq!(
        after.embedding.expect("a vector"),
        vec![1.0, 2.0, 3.0],
        "the new vector describes the new text and is kept"
    );
    assert_eq!(after.embedding_model.as_deref(), Some("test-embedder"));
}

#[tokio::test]
async fn updating_a_missing_memory_reports_no_row() {
    let storage = storage().await;
    let updated = storage
        .memories()
        .update_content("nope", "text")
        .await
        .expect("the call succeeds");
    assert!(!updated, "no row matched");
}
