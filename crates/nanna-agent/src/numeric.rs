//! The crate's lossy numeric conversions, in one place.
//!
//! Ratios, averages and heuristic scores here are computed in floating point
//! from integer counts, and a few float results are turned back into sizes.
//! Rust has no lossless `From` for these directions (`u64` -> `f64` rounds
//! above 2^53, a float -> integer cast truncates and saturates), so every such
//! conversion in the crate goes through the helpers below. Each helper is
//! exactly the `as` cast it replaces — same value for every input — which
//! keeps the numeric results identical while confining the lint expectation
//! to one spot per conversion kind.

use std::time::Duration;

/// `n as f64`: exact up to 2^53, the nearest representable `f64` above.
#[expect(
    clippy::cast_precision_loss,
    reason = "no lossless u64 -> f64 conversion exists; callers use counts and token totals as ratio/statistic inputs, where rounding above 2^53 is immaterial"
)]
pub const fn u64_to_f64(n: u64) -> f64 {
    n as f64
}

/// `n as f64` for a `usize` (widened to `u64` first, which is exact).
pub const fn usize_to_f64(n: usize) -> f64 {
    u64_to_f64(n as u64)
}

/// `n as f32`: exact up to 2^24, the nearest representable `f32` above.
#[expect(
    clippy::cast_precision_loss,
    reason = "no lossless u64 -> f32 conversion exists; callers feed counts into f32 heuristic scores and ratios, where rounding above 2^24 is immaterial"
)]
pub const fn u64_to_f32(n: u64) -> f32 {
    n as f32
}

/// `n as f32` for a `usize` (widened to `u64` first, which is exact).
pub const fn usize_to_f32(n: usize) -> f32 {
    u64_to_f32(n as u64)
}

/// `x as usize`: truncates toward zero, saturates at `0` and `usize::MAX`,
/// and maps NaN to `0`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "no lossless f64 -> usize conversion exists; callers turn a fractional size estimate back into a size, and the cast's truncate-and-saturate behaviour is the intended rounding"
)]
pub const fn f64_to_usize(x: f64) -> usize {
    x as usize
}

/// `x as usize` for an `f32` (widened to `f64` first, which is exact, so the
/// truncation and saturation are unchanged).
pub fn f32_to_usize(x: f32) -> usize {
    f64_to_usize(f64::from(x))
}

/// `x as u32` for an `f32`: truncates toward zero, saturates at `0` and
/// `u32::MAX`, and maps NaN to `0` — the same as `x as u32`, because the
/// `usize` it goes through is at least 32 bits wide.
pub fn f32_to_u32(x: f32) -> u32 {
    u32::try_from(f32_to_usize(x)).unwrap_or(u32::MAX)
}

/// Whole milliseconds in `d`, as `u64`.
///
/// Saturates at `u64::MAX` where the old `as_millis() as u64` wrapped; the two
/// differ only past 2^64 ms (about 584 million years), which no measured
/// duration or wall-clock timestamp reaches.
pub fn millis_u64(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_to_float_rounds_like_the_cast() {
        assert_eq!(u64_to_f64(7).to_bits(), 7.0_f64.to_bits());
        assert_eq!(u64_to_f64((1 << 53) + 1).to_bits(), 9_007_199_254_740_992.0_f64.to_bits());
        assert_eq!(usize_to_f64(3).to_bits(), 3.0_f64.to_bits());
        assert_eq!(u64_to_f32((1 << 24) + 1).to_bits(), 16_777_216.0_f32.to_bits());
        assert_eq!(usize_to_f32(5).to_bits(), 5.0_f32.to_bits());
    }

    #[test]
    fn float_to_integer_truncates_and_saturates_like_the_cast() {
        assert_eq!(f64_to_usize(1.9), 1);
        assert_eq!(f64_to_usize(-1.0), 0);
        assert_eq!(f64_to_usize(f64::NAN), 0);
        assert_eq!(f64_to_usize(f64::INFINITY), usize::MAX);
        assert_eq!(f32_to_usize(2.5), 2);
        assert_eq!(f32_to_u32(2.5), 2);
        assert_eq!(f32_to_u32(-3.0), 0);
        assert_eq!(f32_to_u32(f32::NAN), 0);
        assert_eq!(f32_to_u32(4_294_967_296.0), u32::MAX);
    }

    #[test]
    fn millis_saturate_instead_of_wrapping() {
        assert_eq!(millis_u64(Duration::from_millis(1234)), 1234);
        assert_eq!(millis_u64(Duration::MAX), u64::MAX);
    }
}
