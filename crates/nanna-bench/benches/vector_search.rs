//! Suite 2 — Vector search (SIMD default path).
//!
//! Criterion body that unifies the ad-hoc `nanna-gpu` benches onto the
//! `nanna-bench` harness. Measures batch cosine search latency on the SIMD
//! path at the sizes that matter for the GPU crossover story
//! (`GPU_THRESHOLD = 50_000` in production).
//!
//! Run with:
//!   cargo bench -p nanna-bench --bench vector_search
//!
//! GPU half of the crossover stays in `nanna-gpu` benches (needs a live
//! adapter); this body is the deterministic, hardware-independent SIMD gate.

use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nanna_bench::{Suite, fixture_vectors};

/// Embedding dimension used by the memory store's default MiniLM-class path.
const DIM: usize = 768;

/// Fixture seed — fixed so criterion samples are comparable across runs.
const SEED: u64 = 0x0A_11A_B01;

/// Store sizes that bracket the SIMD-default regime and the documented
/// GPU crossover (`GPU_THRESHOLD = 50_000`).
const SIZES: &[usize] = &[1_000, 10_000, 50_000];

fn simd_batch_search(query: &[f32], vectors: &[Vec<f32>]) -> Vec<f32> {
	vectors
		.iter()
		.map(|v| nanna_simd::cosine_similarity_f32(query, v))
		.collect()
}

fn vector_search_simd(c: &mut Criterion) {
	// Sanity: this bench belongs to Suite::VectorSearch.
	assert_eq!(Suite::VectorSearch.id(), "vector_search");

	let mut group = c.benchmark_group(Suite::VectorSearch.id());
	// Vector search is sub-millisecond at 1k and tens-of-ms at 50k on the
	// reference tier; keep sample count modest so a full suite run finishes
	// in under a minute on a laptop CPU.
	group.sample_size(30);
	group.measurement_time(Duration::from_secs(8));
	group.warm_up_time(Duration::from_secs(1));

	for &n in SIZES {
		let vectors = fixture_vectors(n, DIM, SEED);
		// Query is a fresh vector from a different seed so it is not trivially
		// identical to a store member (which would short-circuit nothing, but
		// keeps the fixture honest).
		let query = fixture_vectors(1, DIM, SEED ^ 0x9E37_79B9)
			.into_iter()
			.next()
			.expect("fixture_vectors(1, ..) yields one vector");

		group.throughput(Throughput::Elements(n as u64));
		group.bench_with_input(BenchmarkId::new("simd_batch", n), &n, |b, &_n| {
			b.iter(|| {
				let scores = simd_batch_search(black_box(&query), black_box(&vectors));
				black_box(scores)
			});
		});
	}

	group.finish();
}

criterion_group!(benches, vector_search_simd);
criterion_main!(benches);
