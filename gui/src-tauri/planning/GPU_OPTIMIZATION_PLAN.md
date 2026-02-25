# GPU Optimization Plan

## Overview
Three interconnected GPU optimization tasks to improve vector store performance and memory efficiency.

---

## 1. GPU Threshold Benchmarking (Study Phase)

### Goal
Determine if 1000-dimension threshold is conservative; find optimal cutoff for GPU acceleration.

### Current State
- Threshold: 1000 dimensions (heuristic)
- GPU used when vector_dim >= 1000
- Conservative estimate; likely underutilizes GPU

### Benchmark Approach

#### Setup
1. Create benchmark harness in `src/memory/gpu_bench.rs`
2. Test on representative hardware:
   - NVIDIA RTX 3090, 4090 (high-end)
   - RTX 3060 (mid-range)
   - RTX 2060 (entry-level)
   - Fallback CPU baseline

#### Test Matrix
| Vector Dim | Batch Size | Operation | Metric |
|-----------|-----------|-----------|--------|
| 128       | 1, 10, 100 | search    | latency, throughput |
| 256       | 1, 10, 100 | search    | latency, throughput |
| 512       | 1, 10, 100 | search    | latency, throughput |
| 768       | 1, 10, 100 | search    | latency, throughput |
| 1024      | 1, 10, 100 | search    | latency, throughput |
| 1536      | 1, 10, 100 | search    | latency, throughput |
| 2048      | 1, 10, 100 | search    | latency, throughput |

#### Metrics to Collect
- **Latency**: Time to execute single operation (ms)
- **Throughput**: Operations per second
- **GPU Utilization**: %
- **GPU Memory Used**: MB
- **CPU Time**: % CPU during GPU ops
- **Break-even Point**: Where GPU >= CPU performance

#### Implementation Steps
1. Build GPU benchmark executable
2. Run on available hardware (document specs)
3. Generate comparison tables
4. Plot break-even curves
5. Identify hardware-specific thresholds
6. Update threshold logic with findings

### Expected Outcomes
- Threshold likely 512-768 (not 1000)
- Hardware-specific tuning possible
- Decision: fixed vs. adaptive threshold

### Timeline
- **Week 1**: Benchmark harness + local testing
- **Week 2**: Collect data, analyze results
- **Week 3**: Document findings, propose new threshold

---

## 2. GPU Memory Management for Large Vector Stores

### Goal
Prevent OOM errors when storing millions of vectors in GPU memory.

### Current State
- No memory limits; vectors loaded until OOM
- No spilling to CPU fallback
- No streaming/windowing strategy

### Solution: Tiered Memory Strategy

#### Tier 1: Hot Memory (GPU)
- **Size**: 80% of GPU VRAM
- **Content**: Most frequently accessed vectors
- **Strategy**: LRU eviction when full

#### Tier 2: Warm Memory (CPU RAM)
- **Size**: Unlimited (system RAM)
- **Content**: Recently evicted vectors
- **Strategy**: Pinned memory for fast transfers

#### Tier 3: Cold Storage (Disk)
- **Size**: Full vector store on disk
- **Content**: Infrequently accessed vectors
- **Strategy**: Memory-mapped file access

### Implementation Steps

1. **GPU Memory Tracker** (`src/memory/gpu_memory.rs`)
   ```rust
   struct GpuMemoryManager {
       total_capacity: u64,
       allocated: u64,
       vectors: HashMap<VectorId, GpuAllocation>,
       lru_cache: LruCache<VectorId, ()>,
   }
   
   impl GpuMemoryManager {
       fn allocate(&mut self, vector_id: VectorId, size: u64) -> Result<()>
       fn evict_lru(&mut self) -> Option<VectorId>
       fn get_allocation(&self, vector_id: VectorId) -> Option<&GpuAllocation>
   }
   ```

2. **CPU Pinned Memory Pool** (`src/memory/pinned_pool.rs`)
   - Pre-allocate pinned CPU memory
   - Fast PCIe transfers (10-20 GB/s)
   - Track pinned allocations

3. **Disk-Backed Vectors** (`src/memory/disk_vectors.rs`)
   - Memory-mapped file format
   - Lazy loading on access
   - Configurable page size

4. **Vector Store Integration**
   - Modify `VectorStore::search()` to handle tiered access
   - Automatic promotion from disk → pinned → GPU
   - Demotion when GPU pressure high

### Configuration
```toml
[gpu]
memory_limit_gb = 8  # 80% of typical VRAM
pinned_memory_gb = 16
disk_cache_path = "/var/cache/nanna/vectors"
```

### Monitoring
- GPU memory usage %
- Cache hit rates (GPU, CPU, disk)
- Eviction frequency
- Promotion/demotion rates

### Timeline
- **Week 1**: Memory tracker + LRU cache
- **Week 2**: Pinned memory pool
- **Week 3**: Disk-backed vectors + integration
- **Week 4**: Testing + tuning

---

## 3. Batched GPU Operations

### Goal
Reduce kernel launch overhead and improve throughput for large-scale operations.

### Current State
- Single-vector operations
- Individual kernel launches
- High overhead for small batches

### Solution: Batching Framework

#### Batch Types

1. **Search Batches**
   - Multiple queries in one kernel call
   - Shared memory optimization
   - Coalesced memory access

2. **Index Update Batches**
   - Insert multiple vectors at once
   - Single tree rebalance
   - Reduced synchronization

3. **Rerank Batches**
   - Parallel reranking of candidates
   - Shared score computation
   - Memory-efficient scoring

#### Implementation Steps

1. **Batch Queue** (`src/memory/batch_queue.rs`)
   ```rust
   pub struct BatchQueue {
       max_batch_size: usize,
       timeout_ms: u64,
       queues: HashMap<BatchType, Vec<BatchItem>>,
   }
   
   impl BatchQueue {
       fn enqueue(&mut self, item: BatchItem) -> Option<Batch>
       fn flush(&mut self) -> Vec<Batch>
   }
   ```

2. **Batched Search Kernel**
   ```rust
   fn gpu_batch_search(
       vectors: &[Vector],
       query_batch: &[Vector],
       k: usize,
   ) -> Vec<Vec<SearchResult>>
   ```
   - Process N queries in parallel
   - Shared distance computation
   - Atomic writes to result buffer

3. **Batched Insert Kernel**
   ```rust
   fn gpu_batch_insert(
       index: &mut GpuIndex,
       vectors: &[Vector],
   ) -> Result<()>
   ```
   - Parallel leaf node insertion
   - Deferred rebalancing
   - Atomic index updates

4. **Adaptive Batching**
   - Measure latency vs. throughput
   - Auto-tune batch sizes
   - Dynamic timeout adjustment

#### Performance Targets
- **Search**: 10x throughput improvement (100 → 1000 ops/sec)
- **Insert**: 5x improvement (10 → 50 ops/sec)
- **Latency**: <5ms overhead for batching

#### Monitoring
- Batch utilization %
- Queue depth
- Kernel launch frequency
- Memory bandwidth utilization

### Timeline
- **Week 1**: Batch queue infrastructure
- **Week 2**: Batched search kernel
- **Week 3**: Batched insert + rerank
- **Week 4**: Adaptive tuning + integration

---

## Integration & Testing

### Phase 1: Independent Validation (Weeks 1-4)
- Each task has separate test suite
- Benchmark GPU threshold separately
- Memory management stress tests
- Batching throughput tests

### Phase 2: Integration (Week 5)
- Combine all three optimizations
- End-to-end benchmarks
- Real workload testing

### Phase 3: Production (Week 6)
- Feature flags for gradual rollout
- Monitoring + telemetry
- User feedback loop

### Testing Strategy
```
Tests/
├── gpu_threshold_bench/
│   ├── latency_sweep.rs
│   ├── throughput_sweep.rs
│   └── break_even_analysis.rs
├── gpu_memory/
│   ├── oom_prevention.rs
│   ├── tiered_access.rs
│   └── cache_efficiency.rs
└── gpu_batching/
    ├── batch_queue.rs
    ├── search_kernel.rs
    └── adaptive_tuning.rs
```

---

## Success Criteria

| Metric | Target | Current |
|--------|--------|---------|
| Vector search latency (1M vectors) | <10ms | ~50ms |
| GPU memory efficiency | >80% utilization | ~40% |
| Throughput (ops/sec) | 1000+ | 100 |
| OOM errors | 0 | Occasional |
| Break-even threshold | 512-768 dims | 1000 dims |

---

## Dependencies & Risks

### Dependencies
- CUDA toolkit (already required)
- GPU memory profiling tools
- Benchmark harness infrastructure

### Risks
| Risk | Mitigation |
|------|-----------|
| Hardware variance | Test on multiple GPUs |
| Regression in latency | Benchmark before/after |
| Memory fragmentation | Allocator tuning |
| Kernel launch overhead | Profile with nsys |

---

## Deliverables

1. **Benchmark Report**: Threshold analysis + recommendations
2. **Memory Manager**: Tiered storage implementation
3. **Batching Framework**: Queue + kernels
4. **Documentation**: Configuration + tuning guide
5. **Monitoring**: Metrics + dashboards
6. **Tests**: Comprehensive test suite

