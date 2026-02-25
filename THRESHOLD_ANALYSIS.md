# GPU Threshold Analysis

## Current Status
- Threshold: 1000 vectors (conservative)
- Benchmark running: CPU vs GPU similarity operations across 100–100k vectors

## Expected Findings

Based on typical GPU characteristics:

### Overhead Costs
- GPU transfer (PCIe): ~1-2µs per KB
- Kernel launch: ~5-10µs
- Memory allocation: ~10-50µs

### Breakeven Analysis
- **100 vectors**: CPU likely faster (low overhead, small dataset)
- **500 vectors**: CPU still competitive 
- **1000 vectors**: Threshold zone (current setting)
- **5000+ vectors**: GPU should dominate (parallelism pays off)

## Recommendations (Preliminary)

1. **If GPU overhead is high** (>100µs per operation):
   - Lower threshold to 500-750 vectors
   - Only use GPU for batched operations (5+ searches)

2. **If GPU overhead is low** (<50µs):
   - Keep 1000 threshold
   - Consider lowering to 750 for frequent searches

3. **If GPU is consistently faster**:
   - Lower to 250-500 vectors
   - Use GPU for all vector operations

## Next Steps
- Analyze benchmark results
- Profile memory usage at each size
- Test on different hardware (NVIDIA, AMD, Intel Arc)
- Implement dynamic threshold based on hardware detection
