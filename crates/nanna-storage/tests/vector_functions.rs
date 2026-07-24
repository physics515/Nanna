//! Capability probe: does the pinned `turso` really compute vector distance in SQL?
//!
//! Both `ROADMAP.md` and the `daily-dev` Appendix C have long asserted that Turso "does NO vector
//! search — cosine is entirely in RAM after `bulk_load`". A source read of `turso_core-0.6.1`
//! showed it registering `vector32`, `vector_extract` and four `vector_distance_*` functions, but
//! a registered function is not a working one, and P13's whole indexed-clustering plan turns on
//! the answer. So this asserts it end to end through the crate we actually depend on.
//!
//! What it deliberately does *not* claim: there is no dense ANN index in 0.6.1 (`index_method/`
//! holds only a btree, FTS, and a self-described "toy" sparse IVF), so this is **exact** k-NN —
//! O(N) per query, but O(1) in RAM, versus today's `bulk_load`-everything-then-scan. Sequencing
//! that trade is a P13 decision; this test only pins the primitive so the decision rests on a
//! fact rather than a blog post about the unrelated libSQL fork.

use turso::{Builder, Connection};

/// Distances are compared with a tolerance: these are f32 kernels reached through SQL,
/// so exact float equality is the wrong assertion even for identical vectors.
const DISTANCE_EPSILON: f64 = 1e-5;

/// Dimensions of the probe vectors. Small on purpose — this tests the *contract*, not throughput.
const EMBEDDING_DIMENSIONS: usize = 4;

async fn connect() -> Connection {
    let db = Builder::new_local(":memory:")
        .build()
        .await
        .expect("in-memory database should open");
    db.connect().expect("connection should open")
}

/// Read a single `f64` from a one-row, one-column query.
///
/// Consumes the cursor before returning: an unfinished `Rows` on a shared connection silently
/// swallows subsequent writes, which is a debugging afternoon nobody needs twice.
async fn query_one_f64(conn: &Connection, sql: &str) -> f64 {
    let mut rows = conn.query(sql, ()).await.expect("query should execute");
    let row = rows
        .next()
        .await
        .expect("query should not error")
        .expect("query should return a row");
    let value = row.get_value(0).expect("column 0 should exist");
    let distance = match value {
        turso::Value::Real(real) => real,
        turso::Value::Integer(int) => int as f64,
        other => panic!("expected a numeric distance, got {other:?}"),
    };
    drop(rows);
    distance
}

#[tokio::test]
async fn cosine_distance_is_computable_in_sql() {
    let conn = connect().await;

    // Identical direction → distance 0; opposite → 2; orthogonal → 1.
    let identical = query_one_f64(
        &conn,
        "SELECT vector_distance_cos(vector32('[1,0,0,0]'), vector32('[1,0,0,0]'))",
    )
    .await;
    assert!(
        (identical - 0.0).abs() < DISTANCE_EPSILON,
        "identical vectors should be at cosine distance 0, got {identical}"
    );

    let orthogonal = query_one_f64(
        &conn,
        "SELECT vector_distance_cos(vector32('[1,0,0,0]'), vector32('[0,1,0,0]'))",
    )
    .await;
    assert!(
        (orthogonal - 1.0).abs() < DISTANCE_EPSILON,
        "orthogonal vectors should be at cosine distance 1, got {orthogonal}"
    );

    let opposite = query_one_f64(
        &conn,
        "SELECT vector_distance_cos(vector32('[1,0,0,0]'), vector32('[-1,0,0,0]'))",
    )
    .await;
    assert!(
        (opposite - 2.0).abs() < DISTANCE_EPSILON,
        "opposed vectors should be at cosine distance 2, got {opposite}"
    );

    // Scale invariance is the property that makes cosine the right metric for embeddings.
    let scaled = query_one_f64(
        &conn,
        "SELECT vector_distance_cos(vector32('[1,2,3,4]'), vector32('[10,20,30,40]'))",
    )
    .await;
    assert!(
        (scaled - 0.0).abs() < DISTANCE_EPSILON,
        "cosine distance should ignore magnitude, got {scaled}"
    );
}

#[tokio::test]
async fn nearest_neighbour_search_can_be_pushed_into_sql() {
    let conn = connect().await;

    conn.execute(
        "CREATE TABLE probe (id TEXT PRIMARY KEY, embedding BLOB NOT NULL)",
        (),
    )
    .await
    .expect("table should create");

    // `far` is deliberately the *opposite* of the query, so a broken kernel that returned a
    // constant (or sorted by rowid) would not accidentally produce the expected order.
    for (id, vector) in [
        ("near", "[1,0,0,0]"),
        ("middle", "[0,1,0,0]"),
        ("far", "[-1,0,0,0]"),
    ] {
        conn.execute(
            &format!("INSERT INTO probe (id, embedding) VALUES ('{id}', vector32('{vector}'))"),
            (),
        )
        .await
        .expect("row should insert");
    }

    let mut rows = conn
        .query(
            "SELECT id FROM probe \
             ORDER BY vector_distance_cos(embedding, vector32('[1,0,0,0]')) ASC",
            (),
        )
        .await
        .expect("k-NN query should execute");

    let mut ranked = Vec::with_capacity(3);
    while let Some(row) = rows.next().await.expect("row read should not error") {
        match row.get_value(0).expect("column 0 should exist") {
            turso::Value::Text(id) => ranked.push(id),
            other => panic!("expected a text id, got {other:?}"),
        }
    }
    drop(rows);

    assert_eq!(
        ranked,
        vec!["near".to_string(), "middle".to_string(), "far".to_string()],
        "SQL-side ordering must match cosine similarity to the query vector"
    );
}

#[tokio::test]
async fn stored_vectors_round_trip_through_extract() {
    let conn = connect().await;

    // A stored embedding must be readable back, or SQL-side search would be a one-way door:
    // the memory layer still needs the raw vector for clustering and dedup.
    let mut rows = conn
        .query("SELECT vector_extract(vector32('[1,2,3,4]'))", ())
        .await
        .expect("extract should execute");
    let row = rows
        .next()
        .await
        .expect("extract should not error")
        .expect("extract should return a row");
    let extracted = match row.get_value(0).expect("column 0 should exist") {
        turso::Value::Text(text) => text,
        other => panic!("expected text, got {other:?}"),
    };
    drop(rows);

    let components: Vec<f64> = extracted
        .trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|part| part.trim().parse::<f64>().expect("component should parse"))
        .collect();

    assert_eq!(
        components.len(),
        EMBEDDING_DIMENSIONS,
        "round-trip must preserve dimensionality, got {extracted}"
    );
    for (index, expected) in [1.0, 2.0, 3.0, 4.0].iter().enumerate() {
        assert!(
            (components[index] - expected).abs() < DISTANCE_EPSILON,
            "component {index} should round-trip as {expected}, got {}",
            components[index]
        );
    }
}
