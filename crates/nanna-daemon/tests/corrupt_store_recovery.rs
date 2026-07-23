//! Boot-level corruption recovery.
//!
//! A `nanna.db` whose memories table has page-level corruption (a zeroed
//! btree leaf — the "Invalid page type: 0" failure mode observed 2026-07-22)
//! used to boot the daemon with a silently empty, degraded memory store.
//! `DaemonBuilder::build()` must now quarantine the damaged file, rebuild a
//! fresh store at the same path, salvage the reachable rows, and surface a
//! `RecoveryReport` for /status and the `MemoryStoreRebuilt` event.

use nanna_daemon::server::DaemonBuilder;
use nanna_storage::{NewMemory, Storage, StorageConfig};
use std::path::Path;

const PAGE_SIZE: usize = 4096;
const ROWS: usize = 200;

fn memory_row(i: usize) -> NewMemory {
    NewMemory {
        memory_id: format!("mem-{i:04}"),
        // ~230 bytes/row spreads the table over many leaf pages, so zeroing
        // one middle leaf loses only some rows.
        content: format!("needle-{i:04} {}", "x".repeat(200)),
        embedding: Some(vec![0.1, 0.2, 0.3, 0.4]),
        embedding_model: Some("test".into()),
        session_id: None,
        metadata: None,
        tags: vec![],
        workspace_id: None,
        fsrs_stability: 1.0,
        fsrs_difficulty: 5.0,
        fsrs_last_access: 0,
        fsrs_access_count: 0,
        fsrs_importance: 1.0,
        fsrs_storage_strength: 1.0,
        fsrs_generation: 0,
    }
}

/// Zero the whole 4096-byte page containing `needle`, breaking the btree walk
/// through it ("Invalid page type: 0").
fn zero_needle_page(db_path: &Path, needle: &[u8]) {
    let mut bytes = std::fs::read(db_path).unwrap();
    let off = bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("needle must be checkpointed into the main db file");
    let page = off / PAGE_SIZE;
    assert!(page > 0, "refusing to zero page 1 (schema root)");
    bytes[page * PAGE_SIZE..(page + 1) * PAGE_SIZE].fill(0);
    std::fs::write(db_path, bytes).unwrap();
}

#[tokio::test]
async fn daemon_boots_with_rebuilt_store_after_page_corruption() {
    let data_dir =
        std::env::temp_dir().join(format!("nanna_daemon_recovery_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data_dir);
    std::fs::create_dir_all(&data_dir).unwrap();
    let db_path = data_dir.join("nanna.db");

    // Populate a store the way a past daemon run would have, then close it.
    {
        let config = StorageConfig {
            path: db_path.to_string_lossy().to_string(),
        };
        let storage = Storage::new(&config).await.unwrap();
        let repo = storage.memories();
        for i in 0..ROWS {
            repo.create(memory_row(i)).await.unwrap();
        }
        // Checkpoint so the page bytes below are the bytes reads actually use.
        let conn = storage.conn().lock().await;
        if let Ok(mut rows) = conn.query("PRAGMA wal_checkpoint(TRUNCATE)", ()).await {
            while let Ok(Some(_)) = rows.next().await {}
        }
    }

    zero_needle_page(&db_path, b"needle-0100");

    // Boot the daemon's storage init path (build() opens + recovers; no
    // sockets are bound until run()).
    let server = DaemonBuilder::new()
        .with_data_dir(&data_dir)
        .with_pid_file(false)
        .build()
        .await;

    let report = server
        .memory_recovery()
        .expect("page-level corruption at boot must produce a recovery report");
    assert!(
        report.quarantine_path.exists(),
        "corrupt file kept for diagnosis"
    );
    assert!(
        report.memories_recovered > 0,
        "reachable rows must be salvaged"
    );
    assert!(
        report.memories_recovered < ROWS,
        "rows on the zeroed page are gone"
    );

    // The daemon's working store is the rebuilt one: fully readable, writable,
    // and holding exactly the salvaged rows.
    drop(server);
    let config = StorageConfig {
        path: db_path.to_string_lossy().to_string(),
    };
    let storage = Storage::new(&config).await.unwrap();
    let loaded = storage.memories().bulk_load().await.unwrap();
    assert_eq!(loaded.len(), report.memories_recovered);
    storage.memories().create(memory_row(9999)).await.unwrap();

    drop(storage);
    let _ = std::fs::remove_dir_all(&data_dir);
}
