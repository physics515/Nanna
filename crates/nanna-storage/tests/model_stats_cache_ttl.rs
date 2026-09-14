//! Migration 015: the 1-hour share of prompt-cache writes is persisted beside the
//! write total, so a restarted daemon still prices those writes at 2x input.

use nanna_storage::{Storage, StoredModelStats};

fn stats(writes: u64, writes_1h: u64) -> StoredModelStats {
    StoredModelStats {
        model: "claude-opus-5".to_string(),
        total_requests: 3,
        successful_requests: 3,
        failed_requests: 0,
        total_input_tokens: 100,
        total_output_tokens: 20,
        total_cache_read_tokens: 500,
        total_cache_creation_tokens: writes,
        total_cache_creation_1h_tokens: writes_1h,
        consecutive_failures: 0,
        last_success_epoch_ms: 1,
        last_failure_epoch_ms: 0,
        tier_successes_simple: 0,
        tier_successes_medium: 0,
        tier_successes_complex: 0,
        tier_failures_simple: 0,
        tier_failures_medium: 0,
        tier_failures_complex: 0,
        escalations: 0,
        latencies_ms: vec![10, 20],
        throughput_tps: vec![1.5],
    }
}

#[tokio::test]
async fn the_one_hour_cache_write_share_round_trips_through_insert_and_upsert() {
    let storage = Storage::in_memory().await.expect("open in-memory storage");

    storage
        .save_model_stats(&[stats(900, 600)])
        .await
        .expect("insert");
    let inserted = storage.load_model_stats().await.expect("load");
    assert_eq!(inserted.len(), 1);
    assert_eq!(inserted[0].total_cache_creation_1h_tokens, 600);

    // The upsert branch must overwrite the new column too, not only set it on insert.
    storage
        .save_model_stats(&[stats(1_000, 700)])
        .await
        .expect("upsert");
    let updated = storage.load_model_stats().await.expect("load");
    assert_eq!(updated.len(), 1, "one row per model");
    assert_eq!(updated[0].total_cache_creation_tokens, 1_000);
    assert_eq!(updated[0].total_cache_creation_1h_tokens, 700);
}
