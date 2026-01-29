#![warn(clippy::all)]
#![warn(clippy::pedantic, clippy::nursery)]

//! SIMD-accelerated operations for Nanna
//!
//! Uses the `wide` crate for stable Rust SIMD. When the `nightly` feature is enabled,
//! falls back to `std::simd` for potentially better codegen.

use half::f16;
use wide::f32x8;

/// SIMD-accelerated dot product for f32 vectors
///
/// Processes 8 elements at a time using AVX/AVX2 instructions.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[inline]
#[must_use]
pub fn dot_product_f32(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "vectors must have equal length");

    let chunks = a.len() / 8;
    let remainder = a.len() % 8;

    let mut sum = f32x8::ZERO;

    // Process 8 elements at a time
    for i in 0..chunks {
        let offset = i * 8;
        let va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);
        sum += va * vb;
    }

    // Horizontal sum of SIMD vector
    let mut result: f32 = sum.reduce_add();

    // Handle remainder
    let remainder_start = chunks * 8;
    for i in 0..remainder {
        result += a[remainder_start + i] * b[remainder_start + i];
    }

    result
}

/// SIMD-accelerated cosine similarity for f32 vectors.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[inline]
#[must_use]
pub fn cosine_similarity_f32(a: &[f32], b: &[f32]) -> f32 {
    assert_eq!(a.len(), b.len(), "vectors must have equal length");

    let chunks = a.len() / 8;
    let remainder = a.len() % 8;

    let mut dot = f32x8::ZERO;
    let mut norm_a = f32x8::ZERO;
    let mut norm_b = f32x8::ZERO;

    for i in 0..chunks {
        let offset = i * 8;
        let va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);

        dot += va * vb;
        norm_a += va * va;
        norm_b += vb * vb;
    }

    let mut dot_sum: f32 = dot.reduce_add();
    let mut mag_a: f32 = norm_a.reduce_add();
    let mut mag_b: f32 = norm_b.reduce_add();

    // Handle remainder
    let remainder_start = chunks * 8;
    for i in 0..remainder {
        let ai = a[remainder_start + i];
        let bi = b[remainder_start + i];
        dot_sum += ai * bi;
        mag_a += ai * ai;
        mag_b += bi * bi;
    }

    dot_sum / (mag_a.sqrt() * mag_b.sqrt())
}

/// SIMD-accelerated vector normalization (L2)
#[inline]
pub fn normalize_f32(v: &mut [f32]) {
    let norm = dot_product_f32(v, v).sqrt();
    if norm > f32::EPSILON {
        let inv_norm = 1.0 / norm;
        let chunks = v.len() / 8;
        let remainder = v.len() % 8;

        let inv_norm_vec = f32x8::splat(inv_norm);

        for i in 0..chunks {
            let offset = i * 8;
            let mut chunk = f32x8::from(&v[offset..offset + 8]);
            chunk *= inv_norm_vec;
            v[offset..offset + 8].copy_from_slice(chunk.as_array_ref());
        }

        let remainder_start = chunks * 8;
        for i in 0..remainder {
            v[remainder_start + i] *= inv_norm;
        }
    }
}

/// Convert f16 array to f32 (for GPU compatibility).
///
/// # Panics
///
/// Panics if `src` and `dst` have different lengths.
#[inline]
pub fn f16_to_f32(src: &[f16], dst: &mut [f32]) {
    assert_eq!(src.len(), dst.len());
    for (s, d) in src.iter().zip(dst.iter_mut()) {
        *d = s.to_f32();
    }
}

/// Convert f32 array to f16 (for memory efficiency).
///
/// # Panics
///
/// Panics if `src` and `dst` have different lengths.
#[inline]
pub fn f32_to_f16(src: &[f32], dst: &mut [f16]) {
    assert_eq!(src.len(), dst.len());
    for (s, d) in src.iter().zip(dst.iter_mut()) {
        *d = f16::from_f32(*s);
    }
}

/// SIMD-accelerated argmax
#[inline]
#[must_use] 
pub fn argmax_f32(v: &[f32]) -> usize {
    if v.is_empty() {
        return 0;
    }

    let mut max_idx = 0;
    let mut max_val = v[0];

    // For argmax, scalar is often faster due to branch prediction
    // SIMD version would need horizontal reduction which has overhead
    for (i, &val) in v.iter().enumerate().skip(1) {
        if val > max_val {
            max_val = val;
            max_idx = i;
        }
    }

    max_idx
}

/// SIMD-accelerated vector addition in-place: a += b.
///
/// # Panics
///
/// Panics if `a` and `b` have different lengths.
#[inline]
pub fn add_f32(a: &mut [f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());

    let chunks = a.len() / 8;
    let remainder = a.len() % 8;

    for i in 0..chunks {
        let offset = i * 8;
        let mut va = f32x8::from(&a[offset..offset + 8]);
        let vb = f32x8::from(&b[offset..offset + 8]);
        va += vb;
        a[offset..offset + 8].copy_from_slice(va.as_array_ref());
    }

    let remainder_start = chunks * 8;
    for i in 0..remainder {
        a[remainder_start + i] += b[remainder_start + i];
    }
}

/// SIMD-accelerated scalar multiplication in-place
#[inline]
pub fn scale_f32(v: &mut [f32], scalar: f32) {
    let chunks = v.len() / 8;
    let remainder = v.len() % 8;
    let scalar_vec = f32x8::splat(scalar);

    for i in 0..chunks {
        let offset = i * 8;
        let mut chunk = f32x8::from(&v[offset..offset + 8]);
        chunk *= scalar_vec;
        v[offset..offset + 8].copy_from_slice(chunk.as_array_ref());
    }

    let remainder_start = chunks * 8;
    for i in 0..remainder {
        v[remainder_start + i] *= scalar;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let b = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let result = dot_product_f32(&a, &b);
        let expected: f32 = (1..=10).map(|x| (x * x) as f32).sum();
        assert!((result - expected).abs() < 1e-5);
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let b = a.clone();
        let result = cosine_similarity_f32(&a, &b);
        assert!((result - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_normalize() {
        let mut v = vec![3.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        normalize_f32(&mut v);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }
}
