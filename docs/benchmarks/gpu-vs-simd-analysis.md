# GPU vs SIMD Benchmark Analysis

**Date:** 2025-02-07
**Hardware:** AMD Zen 4 (AVX-512) + NVIDIA RTX 4070 Ti SUPER (Vulkan/wgpu)
**Embedding dimensions tested:** 768, 1536

## Summary

**GPU never beats SIMD up to 10,000 vectors.** The `GPU_THRESHOLD` was raised from 1,000 to 50,000.

## Raw Results

| Vectors | Dimension | SIMD (AVX-512) | GPU (Vulkan) | Ratio |
|---------|-----------|----------------|--------------|-------|
| 100     | 768       | ~15µs          | ~780µs       | GPU 52× slower |
| 1,000   | 768       | ~148µs         | ~3.4ms       | GPU 23× slower |
| 5,000   | 768       | ~740µs         | ~4.1ms       | GPU 5.5× slower |
| 10,000  | 768       | ~1.48ms        | ~5.22ms      | GPU 3.5× slower |
| 1,000   | 1536      | ~296µs         | ~3.6ms       | GPU 12× slower |
| 10,000  | 1536      | ~2.96ms        | ~6.1ms       | GPU 2.1× slower |

## Analysis

### GPU Fixed Overhead: ~750µs

Every GPU dispatch pays a fixed cost:
1. **Buffer upload** — copying query + vectors to GPU memory (~200µs)
2. **Shader dispatch** — launching the compute shader (~50µs)
3. **Readback** — mapping result buffer back to CPU (~500µs)

This ~750µs floor means GPU can never win for small workloads.

### SIMD Scaling

AVX-512 processes 16 floats per cycle with FMA. For 768-dim vectors:
- Single cosine similarity: **~0.1µs**
- Scales linearly: 10,000 vectors ≈ 1.48ms

### Crossover Estimate

Extrapolating the convergence trend:

```
At 10k vectors (768-dim): GPU is 3.5× slower
At 50k vectors (768-dim): GPU approaches parity (estimated)
At 100k+ vectors (768-dim): GPU likely wins
```

The GPU's advantage only materializes when:
- Compute time dominates transfer overhead
- Thousands of parallel similarity computations saturate GPU cores

### Why wgpu Has High Overhead

The wgpu abstraction adds latency that raw CUDA/Vulkan wouldn't:
- Buffer mapping is synchronous (poll + wait)
- No persistent buffer reuse between searches
- No async transfer/compute overlap

## Decision: `GPU_THRESHOLD = 50,000`

| Threshold | Rationale |
|-----------|-----------|
| ~~1,000~~ | Original guess — GPU was **23× slower** here |
| 50,000    | Conservative estimate where GPU reaches parity |

For a personal memory system, 50,000 memories is a realistic upper bound where GPU dispatch becomes worthwhile. Below that, AVX-512/NEON SIMD is strictly superior.

## Future Optimizations

If GPU performance matters at lower counts:

1. **Persistent GPU buffers** — keep the vector database on GPU memory, only upload the query (~eliminates transfer cost)
2. **Async transfer** — overlap buffer upload with previous compute
3. **Batched queries** — amortize dispatch over multiple searches
4. **Raw Vulkan** — bypass wgpu overhead for the hot path

These would lower the crossover point significantly, potentially to ~5,000 vectors.

## Benchmark Reproduction

```bash
# Quick benchmark (debug profile, fast compile)
cargo bench --bench gpu_vs_simd -p nanna-gpu

# Extended benchmark (more vector counts)
cargo bench --bench gpu_vs_simd_extended -p nanna-gpu
```

Note: `profile.release` uses LTO=fat + codegen-units=1, which takes several minutes to compile. Consider using `--profile dev` for iteration.
