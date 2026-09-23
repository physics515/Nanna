#![warn(clippy::pedantic, clippy::nursery, clippy::all)]
//! Suite 2c — the recall path: SQL exact k-NN vs the in-RAM SIMD scan.
//!
//! P13's "indexed clustering" item sequences the ANN question behind one
//! measurement, and this is it. `MemoryRepository::search_by_embedding_sql`
//! already exists and is proven *correct* — `crates/nanna-storage/tests/
//! vector_knn.rs` pins its ranking against an independent cosine scan and
//! `crates/nanna-daemon/tests/sql_knn_retention.rs` repeats that on the
//! retention corpus. What was never measured is the **trade**: SQL k-NN is
//! O(1) RAM but reads rows from Turso on every query, while the in-RAM path
//! holds every embedding resident and pays only compute per query.
//!
//! The two arms are therefore deliberately *not* symmetric, and the asymmetry
//! is the point:
//!
//! * `sql_knn/<N>` — one `search_by_embedding_sql` call against a **file-backed**
//!   Turso database. Steady RAM is O(limit). This is the whole per-query cost.
//! * `ram_scan/<N>` — the same top-k selection over vectors **already resident**,
//!   in the exact shape `VectorStore::search_with_coverage` runs it: normalize
//!   the query once, `cosine_or_stale` every entry, rank by index, sort, truncate.
//!   Steady RAM is O(N x dim). This is only the per-query cost; the arm below is
//!   what buys it.
//! * `bulk_load/<N>` — `MemoryRepository::bulk_load`, the one-time boot cost the
//!   in-RAM arm must pay before it can answer anything, and the allocation that
//!   sets the ~50k ceiling.
//!
//! A fair reading needs all three rows: comparing `sql_knn` to `ram_scan` alone
//! credits the in-RAM path with a load it never paid for.
//!
//! Run with:
//!   cargo bench -p nanna-bench --bench `recall_path`
//!
//! The in-RAM arm is reproduced here from `nanna-memory`'s scan rather than
//! calling `VectorStore` itself: `VectorStore::new` probes for a GPU adapter and
//! owns a persistence handle, neither of which belongs in a latency fixture, and
//! the GPU branch cannot engage below `GPU_THRESHOLD = 50_000` anyway. The shape
//! below is checked against that function line by line; see `ram_scan_top_k`.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nanna_bench::{Suite, fixture_vectors};
use nanna_storage::{NewMemory, Storage, StorageConfig};

/// Embedding dimension of the memory store's default MiniLM-class path.
/// Same constant as Suite 2 so the two suites' rows can be read together.
const DIM: usize = 768;

/// Fixture seed — shared with Suite 2 so both suites see the same vectors.
const SEED: u64 = 0x0A_11A_B01;

/// Store sizes bracketing the SIMD-default regime and the documented GPU
/// crossover (`GPU_THRESHOLD = 50_000` in `nanna-memory`).
const SIZES: &[usize] = &[1_000, 10_000, 50_000];

/// Neighbours requested per query. `search_scoped` asks for `top_k * 3` before
/// filtering, so 10 here is the shape of a `top_k = 3` scoped recall.
const LIMIT: usize = 10;

/// Cosine against a stored embedding, tolerating one of the wrong width.
///
/// Mirrors `nanna_memory::VectorStore::cosine_or_stale`: the raw kernel asserts
/// equal widths and the release profile aborts on panic, so a stale row scores
/// at the floor instead of taking the process down. Kept here so the benched
/// shape is the shipped shape — a bench that skipped the width check would
/// measure a function the daemon never calls.
fn cosine_or_stale(query: &[f32], embedding: &[f32]) -> f32 {
	if query.len() == embedding.len() {
		nanna_simd::cosine_similarity_f32(query, embedding)
	} else {
		-1.0
	}
}

/// The in-RAM recall scan, in `VectorStore::search_with_coverage`'s shape.
///
/// Three steps, all of which the shipped path performs and a cosine-only bench
/// omits: the comparable-width count that fills `SearchCoverage`, the full
/// similarity map, and the **sort of all N** before `truncate(top_k)`. That
/// last one is O(N log N) on top of the O(N) cosine and is invisible in
/// Suite 2's `simd_batch` row.
fn ram_scan_top_k(query: &[f32], vectors: &[Vec<f32>], top_k: usize) -> Vec<(usize, f32)> {
	let mut ranked = ram_scan_candidates(query, vectors);

	// The shipped selection: partition in O(N), order only the kept prefix.
	if top_k < ranked.len() {
		ranked.select_nth_unstable_by(top_k, rank_order);
		ranked.truncate(top_k);
	}
	ranked.sort_by(rank_order);

	debug_assert!(ranked.len() <= top_k, "returned more than top_k");
	ranked
}

/// Control A — **sort** all N with the *same* comparator, then `truncate`.
///
/// Isolates one variable: selection versus sorting. "Selection is faster than
/// sorting" is the kind of claim that should cost a bench row rather than an
/// argument, and holding the comparator fixed is what makes the row mean that
/// and nothing else.
fn ram_scan_top_k_sort_same_cmp(
	query: &[f32],
	vectors: &[Vec<f32>],
	top_k: usize,
) -> Vec<(usize, f32)> {
	let mut ranked = ram_scan_candidates(query, vectors);
	ranked.sort_by(rank_order);
	ranked.truncate(top_k);
	ranked
}

/// Control B — the **literal** pre-change ranking: a stable sort on similarity
/// alone, then `truncate`.
///
/// Control A alone would overstate the change. The shipped comparator is
/// strictly more work per comparison than the one it replaced — two finiteness
/// checks and an index tiebreak — so a reader comparing A to `ram_scan` sees
/// the algorithmic win with that cost already paid by both arms. This arm is
/// what the daemon actually ran before, cheap intransitive comparator and all,
/// so `ram_scan` against *this* is the honest end-to-end delta.
fn ram_scan_top_k_prior_code(
	query: &[f32],
	vectors: &[Vec<f32>],
	top_k: usize,
) -> Vec<(usize, f32)> {
	let mut ranked = ram_scan_candidates(query, vectors);
	ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
	ranked.truncate(top_k);
	ranked
}

/// The part both ranking arms share: the coverage count and the cosine map.
fn ram_scan_candidates(query: &[f32], vectors: &[Vec<f32>]) -> Vec<(usize, f32)> {
	debug_assert!(!query.is_empty(), "query embedding must not be empty");

	let _comparable = vectors.iter().filter(|v| v.len() == query.len()).count();

	vectors
		.iter()
		.map(|v| cosine_or_stale(query, v))
		.enumerate()
		.collect()
}

/// `nanna_memory::VectorStore::rank_order`, reproduced for the same reason
/// `cosine_or_stale` is: the benched shape must be the shipped shape.
fn rank_order(a: &(usize, f32), b: &(usize, f32)) -> std::cmp::Ordering {
	let sa = if a.1.is_finite() { a.1 } else { f32::NEG_INFINITY };
	let sb = if b.1.is_finite() { b.1 } else { f32::NEG_INFINITY };
	sb.partial_cmp(&sa)
		.unwrap_or(std::cmp::Ordering::Equal)
		.then_with(|| a.0.cmp(&b.0))
}

/// A fixture memory row carrying one embedding.
fn new_memory(index: usize, embedding: Vec<f32>) -> NewMemory {
	NewMemory {
		memory_id: format!("m-{index:07}"),
		content: format!("fixture memory {index}"),
		embedding: Some(embedding),
		embedding_model: Some("bench-fixture".to_string()),
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

/// Build a **file-backed** store holding `n` fixture memories.
///
/// File-backed, not `Storage::in_memory()`: the claim under test is that SQL
/// k-NN streams rows instead of holding them, and an in-memory database would
/// hand the arm the very residency the comparison is about.
async fn seeded_store(dir: &std::path::Path, n: usize, vectors: &[Vec<f32>]) -> Storage {
	assert_eq!(vectors.len(), n, "fixture size must match the store size");

	let config = StorageConfig {
		path: dir
			.join(format!("recall-{n}.db"))
			.to_string_lossy()
			.into_owned(),
	};
	let storage = Storage::new(&config)
		.await
		.expect("open file-backed storage");
	let repo = storage.memories();
	for (i, v) in vectors.iter().enumerate() {
		repo.create(new_memory(i, v.clone()))
			.await
			.expect("insert fixture memory");
	}
	storage
}

fn recall_path(c: &mut Criterion) {
	// Sanity: these rows belong to Suite::VectorSearch.
	assert_eq!(Suite::VectorSearch.id(), "vector_search");

	let runtime = tokio::runtime::Builder::new_current_thread()
		.enable_all()
		.build()
		.expect("build bench runtime");
	let scratch = tempfile::tempdir().expect("create scratch dir for bench databases");

	let mut group = c.benchmark_group("recall_path");
	// Seeding 50k rows through Turso dominates setup, and the SQL arm is
	// tens of milliseconds per query at that size; keep the sample count low
	// enough that a full run stays inside a few minutes.
	group.sample_size(20);
	group.measurement_time(Duration::from_secs(10));
	group.warm_up_time(Duration::from_secs(2));

	for &n in SIZES {
		let vectors = fixture_vectors(n, DIM, SEED);
		// A query from a different seed: close to nothing in particular, so
		// neither arm can short-circuit on an exact hit.
		let query = fixture_vectors(1, DIM, SEED ^ 0x9E37_79B9)
			.into_iter()
			.next()
			.expect("fixture_vectors(1, ..) yields one vector");

		let storage = runtime.block_on(seeded_store(scratch.path(), n, &vectors));
		let repo = storage.memories();

		// Both arms must actually answer before either is timed. A bench that
		// measures an empty result set is worse than no bench: the SQL arm's
		// `octet_length` guard silently skips every row on a dimension
		// mismatch, so "fast" and "scanned nothing" look identical in the
		// output. Checked once per size, outside the timed loop.
		{
			let mut q = query.clone();
			nanna_simd::normalize_f32(&mut q);
			let ram = ram_scan_top_k(&q, &vectors, LIMIT);
			// Selection and the sort it replaced must produce the same answer,
			// or the speed-up below is measuring a different function.
			assert_eq!(
				ram,
				ram_scan_top_k_sort_same_cmp(&q, &vectors, LIMIT),
				"selection and full-sort ranking disagree at N={n}"
			);
			// The fixture has no ties and no NaN, so the prior ranking must
			// agree too — which is what makes control B a like-for-like row
			// rather than a measurement of a different answer.
			assert_eq!(
				ram,
				ram_scan_top_k_prior_code(&q, &vectors, LIMIT),
				"selection and the prior ranking disagree at N={n}"
			);
			let sql = runtime
				.block_on(repo.search_by_embedding_sql(&query, LIMIT, None))
				.expect("sql knn warm-up");
			assert_eq!(ram.len(), LIMIT, "in-RAM arm returned {} of {LIMIT} at N={n}", ram.len());
			assert_eq!(sql.len(), LIMIT, "SQL arm returned {} of {LIMIT} at N={n}", sql.len());
			// The two arms rank the same corpus by the same metric, so their
			// nearest neighbour is the same row. `vector_knn.rs` proves this
			// property in general; here it guards the fixture itself.
			let ram_top = format!("m-{:07}", ram[0].0);
			assert_eq!(
				sql[0].0, ram_top,
				"arms disagree on the nearest neighbour at N={n}: SQL {} vs RAM {ram_top}",
				sql[0].0
			);
		}

		group.throughput(Throughput::Elements(n as u64));

		group.bench_with_input(BenchmarkId::new("sql_knn", n), &n, |b, &_n| {
			b.to_async(&runtime).iter(|| async {
				let got = repo
					.search_by_embedding_sql(black_box(&query), LIMIT, None)
					.await
					.expect("sql knn");
				black_box(got)
			});
		});

		// The in-RAM arm scans vectors that are already resident, so the
		// normalize the shipped path performs on the query happens here too —
		// once per query, exactly as `search_with_coverage` does it.
		group.bench_with_input(BenchmarkId::new("ram_scan", n), &n, |b, &_n| {
			b.iter(|| {
				let mut q = query.clone();
				nanna_simd::normalize_f32(&mut q);
				black_box(ram_scan_top_k(black_box(&q), black_box(&vectors), LIMIT))
			});
		});

		// Control A: same comparator, sorted instead of selected.
		group.bench_with_input(BenchmarkId::new("ram_scan_sort_same_cmp", n), &n, |b, &_n| {
			b.iter(|| {
				let mut q = query.clone();
				nanna_simd::normalize_f32(&mut q);
				black_box(ram_scan_top_k_sort_same_cmp(
					black_box(&q),
					black_box(&vectors),
					LIMIT,
				))
			});
		});

		// Control B: the ranking the daemon actually ran before the change.
		group.bench_with_input(BenchmarkId::new("ram_scan_prior_code", n), &n, |b, &_n| {
			b.iter(|| {
				let mut q = query.clone();
				nanna_simd::normalize_f32(&mut q);
				black_box(ram_scan_top_k_prior_code(
					black_box(&q),
					black_box(&vectors),
					LIMIT,
				))
			});
		});

		// What the in-RAM arm costs before it can answer anything.
		group.bench_with_input(BenchmarkId::new("bulk_load", n), &n, |b, &_n| {
			b.to_async(&runtime).iter(|| async {
				let loaded = repo.bulk_load().await.expect("bulk load");
				black_box(loaded)
			});
		});
	}

	group.finish();
}

criterion_group!(benches, recall_path);
criterion_main!(benches);
