//! AVX-512 accelerated SIMD operations.
//!
//! These functions require AVX-512F support and process 16 f32 elements at a time,
//! doubling throughput compared to AVX2's 8-wide operations.
//!
//! Safety: All functions are marked `unsafe` and require `#[target_feature(enable = "avx512f")]`.
//! They must only be called after verifying CPU support via `is_x86_feature_detected!("avx512f")`.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// AVX-512 dot product: processes 16 floats per iteration.
///
/// # Safety
/// Caller must ensure AVX-512F is supported on the current CPU.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub unsafe fn dot_product_f32_avx512(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    let len = a.len();
    let chunks = len / 16;
    let remainder = len % 16;

    let mut acc = _mm512_setzero_ps();

    for i in 0..chunks {
        let offset = i * 16;
        // SAFETY: offset + 16 <= len (loop bound), and avx512f is guaranteed by caller.
        unsafe {
            let va = _mm512_loadu_ps(a.as_ptr().add(offset));
            let vb = _mm512_loadu_ps(b.as_ptr().add(offset));
            acc = _mm512_fmadd_ps(va, vb, acc);
        }
    }

    let mut result = _mm512_reduce_add_ps(acc);

    // Scalar remainder
    let rem_start = chunks * 16;
    for i in 0..remainder {
        result += a[rem_start + i] * b[rem_start + i];
    }

    result
}

/// AVX-512 cosine similarity: computes dot(a,b) / (|a| * |b|) with 16-wide FMA.
///
/// # Safety
/// Caller must ensure AVX-512F is supported on the current CPU.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub unsafe fn cosine_similarity_f32_avx512(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());

    let len = a.len();
    let chunks = len / 16;
    let remainder = len % 16;

    let mut dot_acc = _mm512_setzero_ps();
    let mut norm_a_acc = _mm512_setzero_ps();
    let mut norm_b_acc = _mm512_setzero_ps();

    for i in 0..chunks {
        let offset = i * 16;
        // SAFETY: offset + 16 <= len (loop bound), and avx512f is guaranteed by caller.
        unsafe {
            let va = _mm512_loadu_ps(a.as_ptr().add(offset));
            let vb = _mm512_loadu_ps(b.as_ptr().add(offset));

            dot_acc = _mm512_fmadd_ps(va, vb, dot_acc);
            norm_a_acc = _mm512_fmadd_ps(va, va, norm_a_acc);
            norm_b_acc = _mm512_fmadd_ps(vb, vb, norm_b_acc);
        }
    }

    let mut dot_sum = _mm512_reduce_add_ps(dot_acc);
    let mut mag_a = _mm512_reduce_add_ps(norm_a_acc);
    let mut mag_b = _mm512_reduce_add_ps(norm_b_acc);

    // Scalar remainder
    let rem_start = chunks * 16;
    for i in 0..remainder {
        let ai = a[rem_start + i];
        let bi = b[rem_start + i];
        dot_sum += ai * bi;
        mag_a += ai * ai;
        mag_b += bi * bi;
    }

    dot_sum / (mag_a.sqrt() * mag_b.sqrt())
}

/// AVX-512 vector normalization (L2 in-place): processes 16 elements per iteration.
///
/// # Safety
/// Caller must ensure AVX-512F is supported on the current CPU.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub unsafe fn normalize_f32_avx512(v: &mut [f32]) {
    // SAFETY: dot_product_f32_avx512 has the same target_feature requirement.
    let norm = unsafe { dot_product_f32_avx512(v, v) }.sqrt();

    if norm > f32::EPSILON {
        let inv_norm = 1.0 / norm;
        let inv_norm_vec = _mm512_set1_ps(inv_norm);

        let len = v.len();
        let chunks = len / 16;
        let remainder = len % 16;

        for i in 0..chunks {
            let offset = i * 16;
            // SAFETY: offset + 16 <= len (loop bound), and avx512f is guaranteed by caller.
            unsafe {
                let chunk = _mm512_loadu_ps(v.as_ptr().add(offset));
                let scaled = _mm512_mul_ps(chunk, inv_norm_vec);
                _mm512_storeu_ps(v.as_mut_ptr().add(offset), scaled);
            }
        }

        let rem_start = chunks * 16;
        for i in 0..remainder {
            v[rem_start + i] *= inv_norm;
        }
    }
}

/// AVX-512 vector addition in-place: a += b, 16 elements per iteration.
///
/// # Safety
/// Caller must ensure AVX-512F is supported on the current CPU.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub unsafe fn add_f32_avx512(a: &mut [f32], b: &[f32]) {
    debug_assert_eq!(a.len(), b.len());

    let len = a.len();
    let chunks = len / 16;
    let remainder = len % 16;

    for i in 0..chunks {
        let offset = i * 16;
        // SAFETY: offset + 16 <= len (loop bound), and avx512f is guaranteed by caller.
        unsafe {
            let va = _mm512_loadu_ps(a.as_ptr().add(offset));
            let vb = _mm512_loadu_ps(b.as_ptr().add(offset));
            let sum = _mm512_add_ps(va, vb);
            _mm512_storeu_ps(a.as_mut_ptr().add(offset), sum);
        }
    }

    let rem_start = chunks * 16;
    for i in 0..remainder {
        a[rem_start + i] += b[rem_start + i];
    }
}

/// AVX-512 scalar multiplication in-place: v *= scalar, 16 elements per iteration.
///
/// # Safety
/// Caller must ensure AVX-512F is supported on the current CPU.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx512f")]
pub unsafe fn scale_f32_avx512(v: &mut [f32], scalar: f32) {
    let scalar_vec = _mm512_set1_ps(scalar);

    let len = v.len();
    let chunks = len / 16;
    let remainder = len % 16;

    for i in 0..chunks {
        let offset = i * 16;
        // SAFETY: offset + 16 <= len (loop bound), and avx512f is guaranteed by caller.
        unsafe {
            let chunk = _mm512_loadu_ps(v.as_ptr().add(offset));
            let scaled = _mm512_mul_ps(chunk, scalar_vec);
            _mm512_storeu_ps(v.as_mut_ptr().add(offset), scaled);
        }
    }

    let rem_start = chunks * 16;
    for i in 0..remainder {
        v[rem_start + i] *= scalar;
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "x86_64")]
    use super::*;

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_avx512_dot_product() {
        if !is_x86_feature_detected!("avx512f") {
            eprintln!("skipping AVX-512 test: not supported on this CPU");
            return;
        }
        // 20 elements: 16 in SIMD + 4 remainder
        let a: Vec<f32> = (1..=20).map(|x| x as f32).collect();
        let b: Vec<f32> = (1..=20).map(|x| x as f32).collect();
        let expected: f32 = (1..=20).map(|x: i32| (x * x) as f32).sum();
        let result = unsafe { dot_product_f32_avx512(&a, &b) };
        assert!(
            (result - expected).abs() < 1e-3,
            "got {result}, expected {expected}"
        );
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_avx512_cosine_similarity_identical() {
        if !is_x86_feature_detected!("avx512f") {
            eprintln!("skipping AVX-512 test: not supported on this CPU");
            return;
        }
        let a: Vec<f32> = (1..=32).map(|x| x as f32).collect();
        let b = a.clone();
        let result = unsafe { cosine_similarity_f32_avx512(&a, &b) };
        assert!(
            (result - 1.0).abs() < 1e-5,
            "identical vectors should have cosine similarity ~1.0, got {result}"
        );
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_avx512_normalize() {
        if !is_x86_feature_detected!("avx512f") {
            eprintln!("skipping AVX-512 test: not supported on this CPU");
            return;
        }
        let mut v: Vec<f32> = (1..=20).map(|x| x as f32).collect();
        unsafe { normalize_f32_avx512(&mut v) };
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "normalized vector should have unit length, got {norm}"
        );
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_avx512_add() {
        if !is_x86_feature_detected!("avx512f") {
            eprintln!("skipping AVX-512 test: not supported on this CPU");
            return;
        }
        let mut a: Vec<f32> = (1..=20).map(|x| x as f32).collect();
        let b: Vec<f32> = (1..=20).map(|x| x as f32).collect();
        unsafe { add_f32_avx512(&mut a, &b) };
        for (i, &val) in a.iter().enumerate() {
            let expected = ((i + 1) * 2) as f32;
            assert!(
                (val - expected).abs() < 1e-5,
                "a[{i}] = {val}, expected {expected}"
            );
        }
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn test_avx512_scale() {
        if !is_x86_feature_detected!("avx512f") {
            eprintln!("skipping AVX-512 test: not supported on this CPU");
            return;
        }
        let mut v: Vec<f32> = (1..=20).map(|x| x as f32).collect();
        unsafe { scale_f32_avx512(&mut v, 3.0) };
        for (i, &val) in v.iter().enumerate() {
            let expected = ((i + 1) * 3) as f32;
            assert!(
                (val - expected).abs() < 1e-5,
                "v[{i}] = {val}, expected {expected}"
            );
        }
    }
}
