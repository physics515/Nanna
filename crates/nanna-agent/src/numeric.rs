//! The crate's lossy numeric conversions, in one place.
//!
//! Ratios, averages and heuristic scores here are computed in floating point
//! from integer counts, and a few float results are turned back into sizes.
//! Rust has no lossless `From` for these directions (`u64` -> `f64` rounds
//! above 2^53, a float -> integer cast truncates and saturates), so every such
//! conversion in the crate goes through `nanna_numeric`, re-exported here so
//! the crate keeps a single import path for them. Each is value-for-value the
//! `as` cast it replaced; the rounding is stated (and tested) once, there.

use std::time::Duration;

pub use nanna_numeric::{
    f32_from_u64, f32_from_usize, f64_from_u64, f64_from_usize, u32_from_f32, usize_from_f32,
    usize_from_f64,
};

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
    fn millis_saturate_instead_of_wrapping() {
        assert_eq!(millis_u64(Duration::from_millis(1234)), 1234);
        assert_eq!(millis_u64(Duration::MAX), u64::MAX);
    }
}
