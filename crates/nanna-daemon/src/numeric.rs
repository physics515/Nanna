//! Numeric conversions with no lossless `From`/`TryFrom` form.
//!
//! Every lossy or range-changing conversion the daemon needs lives here, each
//! with the reason it is sound at the call sites that use it, so a reader
//! auditing precision or truncation has one file to read instead of casts
//! scattered through the crate.

use std::time::Duration;

/// Whole milliseconds in `duration`, saturating at `u64::MAX`.
///
/// `Duration::as_millis` is `u128`. `u64::MAX` milliseconds is roughly 584
/// million years, so for any elapsed time or wall-clock reading the daemon
/// takes the saturation is unreachable; it only stands where an `as u64` cast
/// would have wrapped silently.
pub fn millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// `value` as a `usize`: exact on 64-bit targets.
///
/// On a 32-bit target a value past `usize::MAX` saturates rather than
/// truncating to its low bits — for the counts and limits read from tool
/// parameters here, "as many as possible" is the reading of an oversized
/// number, where truncation would turn `2^32 + 5` into `5`.
#[expect(
    dead_code,
    reason = "the callers are the five `params[\"limit\"] as usize` reads in \
              server.rs; the v0.3.21-beta.30 merge restored that file's \
              pre-cleanup copy, so they are casts again until it is re-split. \
              Removing this attribute is part of that: with the calls back, \
              the expectation goes unfulfilled and says so."
)]
pub fn usize_saturating(value: u64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// A collection length or index as an `i64`, saturating at `i64::MAX`.
///
/// Lengths and indices never exceed `isize::MAX`, which fits `i64` on every
/// supported target, so the saturation is unreachable.
pub fn i64_saturating(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// A stored counter back to the `u32` it was written from, clamped to range.
///
/// Every write path binds `i64::from(u32)`, so an out-of-range value is
/// unreachable short of an external edit to the row; such a value clamps to
/// the nearest end of `u32` rather than wrapping into an arbitrary count.
pub fn u32_clamped(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(if value < 0 { 0 } else { u32::MAX })
}

/// `value` as an `f32`, rounding to the nearest representable value.
#[expect(
    clippy::cast_possible_truncation,
    reason = "f64 -> f32 has no lossless conversion; the callers pass importance \
              weights and cosine similarities, which the memory store keeps as f32"
)]
pub const fn f32_from_f64(value: f64) -> f32 {
    value as f32
}

/// `value` as an `f64`, rounding to the nearest representable value.
#[expect(
    clippy::cast_precision_loss,
    reason = "u64 -> f64 has no lossless conversion; the callers compute rates and \
              display averages from request counts and latencies, far below 2^52"
)]
pub const fn f64_from_u64(value: u64) -> f64 {
    value as f64
}

/// `value` as an `i64`: truncates toward zero, clamps to the `i64` range, and
/// maps NaN to 0.
#[expect(
    clippy::cast_possible_truncation,
    reason = "f64 -> i64 has no lossless conversion; the caller only converts whole \
              values within 2^53, which are exact, and `as` saturates otherwise"
)]
pub const fn i64_from_f64(value: f64) -> i64 {
    value as i64
}

/// `value` as a `u64`: truncates toward zero, clamps to `0..=u64::MAX`, and
/// maps NaN to 0.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "f64 -> u64 has no lossless conversion; the callers round an estimated \
              count first, and `as` already saturates at both ends"
)]
pub const fn u64_from_f64(value: f64) -> u64 {
    value as u64
}
