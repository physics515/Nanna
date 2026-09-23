#![warn(clippy::pedantic, clippy::nursery, clippy::all)]
//! Numeric conversions the language only offers as `as` casts, rebuilt from
//! lossless pieces.
//!
//! Std has no `From`/`TryFrom` for integer-to-float above 32 bits, for any
//! float-to-integer direction, or for `f64` to `f32`. Every crate that needed
//! one used to carry an `as` cast behind a lint expectation; this crate is the
//! single place those conversions are implemented, and each function is
//! value-for-value identical to the `as` cast named in its doc for **every**
//! input:
//!
//! - integer to float rounds to nearest, ties to even;
//! - float to integer truncates toward zero, saturates at the target's bounds
//!   and maps NaN to zero;
//! - `f64` to `f32` rounds to nearest, ties to even, overflows to the signed
//!   infinity and underflows to the signed zero.
//!
//! The tests pin those semantics against IEEE 754 bit patterns and a bit-exact
//! integer oracle rather than against the cast, so no lossy cast appears
//! anywhere in this crate. The one `as` it does contain widens `usize` to
//! `u64`, which is exact on every target Rust supports.

/// The `(hi, lo)` 32-bit halves of `n`, so that `n == hi * 2^32 + lo`.
const fn halves_u64(n: u64) -> (u32, u32) {
    let b = n.to_le_bytes();
    (
        u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
        u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
    )
}

/// The low 32 bits of a value the caller has shown to be below `2^32`.
const fn low_u32(n: u64) -> u32 {
    debug_assert!(n >> 32 == 0, "value does not fit in 32 bits");
    halves_u64(n).1
}

/// `usize` widened to `u64`: exact, since `usize` is at most 64 bits wide on
/// every target Rust supports.
const fn widen_usize(n: usize) -> u64 {
    n as u64
}

/// `2^k` as an `f32`, for `0 <= k <= 32`, built from its exponent field.
fn pow2_f32(k: u32) -> f32 {
    debug_assert!(k <= 32, "2^{k} is outside the range this helper serves");
    f32::from_bits((127 + k) << 23)
}

/// `value / 2^shift` rounded to nearest, ties to even, for `1 <= shift`.
///
/// A shift of 64 or more yields 0: `value` is at most 53 bits wide at every
/// call site, so the quotient is then below one half.
fn round_shift_even(value: u64, shift: u64) -> u64 {
    if shift >= 64 {
        return 0;
    }
    let kept = value >> shift;
    let remainder = value & ((1_u64 << shift) - 1);
    let half = 1_u64 << (shift - 1);
    let round_up = remainder > half || (remainder == half && kept & 1 == 1);
    kept + u64::from(round_up)
}

// ---------------------------------------------------------------------------
// Integer -> float
// ---------------------------------------------------------------------------

/// `n as f64`: exact below `2^53`, the nearest `f64` (ties to even) above.
///
/// Both 32-bit halves are exact in `f64`, and `hi * 2^32` is a power-of-two
/// scaling of a 32-bit value, so it is exact too. The fused multiply-add then
/// rounds the exact sum `hi * 2^32 + lo == n` exactly once, which is the one
/// rounding the cast performs.
#[must_use]
pub fn f64_from_u64(n: u64) -> f64 {
    let (hi, lo) = halves_u64(n);
    f64::from(hi).mul_add(4_294_967_296.0, f64::from(lo))
}

/// `n as f64` for a signed value.
///
/// Round-to-nearest-even is symmetric under negation, so converting the
/// magnitude and restoring the sign is bit-identical to the cast.
#[must_use]
pub fn f64_from_i64(n: i64) -> f64 {
    let magnitude = f64_from_u64(n.unsigned_abs());
    if n < 0 { -magnitude } else { magnitude }
}

/// `n as f64` for a `usize` (widened to `u64` first, which is exact).
#[must_use]
pub fn f64_from_usize(n: usize) -> f64 {
    f64_from_u64(widen_usize(n))
}

/// `n as f32` for a 32-bit value: exact below `2^24`, nearest (ties to even)
/// above.
///
/// `hi * 2^16` is a 16-bit value scaled by a power of two, hence exact, so the
/// fused multiply-add rounds the exact sum `hi * 2^16 + lo == n` exactly once.
#[must_use]
pub fn f32_from_u32(n: u32) -> f32 {
    let [b0, b1, b2, b3] = n.to_le_bytes();
    let lo = u16::from_le_bytes([b0, b1]);
    let hi = u16::from_le_bytes([b2, b3]);
    f32::from(hi).mul_add(65_536.0, f32::from(lo))
}

/// `n as f32`: exact below `2^24`, the nearest `f32` (ties to even) above.
///
/// A value wider than 32 bits is first cut down to its top 32 bits, with a
/// *sticky* bit `OR`ed into the lowest of them whenever any dropped bit was set.
/// Rounding to `f32`'s 24 significant bits only ever looks at the 24 kept
/// bits, the next bit, and whether anything below that is non-zero; the sticky
/// bit preserves exactly that last fact, so rounding the reduced value gives
/// the same 24-bit result as rounding `n`. Scaling back by `2^dropped` is then
/// exact, since the result stays far inside `f32`'s exponent range.
#[must_use]
pub fn f32_from_u64(n: u64) -> f32 {
    let dropped = (64 - n.leading_zeros()).saturating_sub(32);
    if dropped == 0 {
        return f32_from_u32(low_u32(n));
    }
    let dropped_bits = n & ((1_u64 << dropped) - 1);
    let sticky = u64::from(dropped_bits != 0);
    let reduced = low_u32((n >> dropped) | sticky);
    f32_from_u32(reduced) * pow2_f32(dropped)
}

/// `n as f32` for a signed value; see [`f64_from_i64`] for why the sign can
/// be restored after the fact.
#[must_use]
pub fn f32_from_i64(n: i64) -> f32 {
    let magnitude = f32_from_u64(n.unsigned_abs());
    if n < 0 { -magnitude } else { magnitude }
}

/// `n as f32` for an `i32` (widened to `i64` first, which is exact).
#[must_use]
pub fn f32_from_i32(n: i32) -> f32 {
    f32_from_i64(i64::from(n))
}

/// `n as f32` for a `usize` (widened to `u64` first, which is exact).
#[must_use]
pub fn f32_from_usize(n: usize) -> f32 {
    f32_from_u64(widen_usize(n))
}

// ---------------------------------------------------------------------------
// f64 -> f32
// ---------------------------------------------------------------------------

/// `x as f32`: the nearest `f32` (ties to even); `±inf` and NaN are preserved,
/// a magnitude past `f32::MAX` becomes the signed infinity, and one below the
/// smallest `f32` subnormal becomes the signed zero.
///
/// Works on the IEEE 754 fields directly. A value in `f32`'s normal range
/// keeps its exponent and rounds 29 mantissa bits away; one below it is
/// rescaled to `f32`'s subnormal unit (`2^-149`) and rounded there. In both
/// cases a mantissa that carries out of its field lands in the next exponent
/// (or in infinity) by plain addition, which is the standard trick and why no
/// separate carry handling appears.
#[must_use]
pub fn f32_from_f64(x: f64) -> f32 {
    const MANTISSA_MASK: u64 = (1 << 52) - 1;
    const EXPONENT_MASK: u64 = 0x7FF;
    /// Biased `f64` exponent of `2^-126`, the smallest `f32` normal.
    const MIN_NORMAL: u64 = 1023 - 126;
    /// Biased `f64` exponent of `2^127`, the largest `f32` binade.
    const MAX_NORMAL: u64 = 1023 + 127;
    /// `f64` exponent field minus this is the `f32` exponent field.
    const REBIAS: u64 = 1023 - 127;
    /// `2^-149 == 2^(exponent - 1075)` solves to this biased exponent; a
    /// value below `2^-126` is `full * 2^(exponent - 1075)`, so in units of
    /// `2^-149` it is `full >> (SUBNORMAL_ORIGIN - exponent)`.
    const SUBNORMAL_ORIGIN: u64 = 1075 - 149;
    const F32_INFINITY: u64 = 0xFF << 23;
    const F32_QUIET_NAN: u64 = 0x7FC0_0000;

    let bits = x.to_bits();
    let sign = (bits >> 32) & 0x8000_0000;
    let exponent = (bits >> 52) & EXPONENT_MASK;
    let mantissa = bits & MANTISSA_MASK;

    let magnitude = if exponent == EXPONENT_MASK {
        if mantissa == 0 { F32_INFINITY } else { F32_QUIET_NAN }
    } else if exponent > MAX_NORMAL {
        F32_INFINITY
    } else if exponent >= MIN_NORMAL {
        ((exponent - REBIAS) << 23) + round_shift_even(mantissa, 29)
    } else if exponent == 0 {
        // An f64 zero or subnormal: below 2^-1022, so far below 2^-150.
        0
    } else {
        let full = mantissa | (1 << 52);
        round_shift_even(full, SUBNORMAL_ORIGIN - exponent)
    };
    f32::from_bits(low_u32(sign | magnitude))
}

// ---------------------------------------------------------------------------
// Float -> integer
// ---------------------------------------------------------------------------

/// `x as u64`: truncates toward zero, saturates at `0` and `u64::MAX`, and
/// maps NaN to `0`.
///
/// For `1 <= x < 2^64` the value is `full * 2^(exponent - 52)` with `full` the
/// 53-bit significand, so truncation is a shift of `full` in the direction the
/// exponent says; everything else is one of the saturating cases.
#[must_use]
pub fn u64_from_f64(x: f64) -> u64 {
    const MANTISSA_MASK: u64 = (1 << 52) - 1;
    /// `2^64` exactly (`f64::from_bits(0x43F0_0000_0000_0000)`).
    const TWO_POW_64: f64 = 18_446_744_073_709_551_616.0;

    if x.is_nan() || x < 1.0 {
        return 0;
    }
    if x >= TWO_POW_64 {
        return u64::MAX;
    }
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7FF) - 1023;
    let full = (bits & MANTISSA_MASK) | (1 << 52);
    if exponent >= 52 { full << (exponent - 52) } else { full >> (52 - exponent) }
}

/// `x as i64`: truncates toward zero, saturates at `i64::MIN` and `i64::MAX`,
/// and maps NaN to `0`.
#[must_use]
pub fn i64_from_f64(x: f64) -> i64 {
    if x < 0.0 {
        i64::try_from(u64_from_f64(-x)).map_or(i64::MIN, |magnitude| -magnitude)
    } else {
        i64::try_from(u64_from_f64(x)).unwrap_or(i64::MAX)
    }
}

/// `x as usize`: truncates toward zero, saturates at `0` and `usize::MAX`, and
/// maps NaN to `0`.
#[must_use]
pub fn usize_from_f64(x: f64) -> usize {
    usize::try_from(u64_from_f64(x)).unwrap_or(usize::MAX)
}

/// `x as usize` for an `f32` (widened to `f64` first, which is exact, so the
/// truncation and saturation are unchanged).
#[must_use]
pub fn usize_from_f32(x: f32) -> usize {
    usize_from_f64(f64::from(x))
}

/// `x as u32` for an `f32`: truncates toward zero, saturates at `0` and
/// `u32::MAX`, and maps NaN to `0`.
#[must_use]
pub fn u32_from_f32(x: f32) -> u32 {
    u32::try_from(u64_from_f64(f64::from(x))).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The integer an integer-valued float encodes, read off its bit fields.
    fn f32_integer(r: f32) -> u128 {
        let bits = r.to_bits();
        let exponent = (bits >> 23) & 0xFF;
        let significand = u128::from((bits & 0x7F_FFFF) | (1 << 23));
        if exponent >= 150 {
            significand << (exponent - 150)
        } else {
            significand >> (150 - exponent)
        }
    }

    fn f64_integer(r: f64) -> u128 {
        let bits = r.to_bits();
        let exponent = (bits >> 52) & 0x7FF;
        let significand = u128::from((bits & ((1 << 52) - 1)) | (1 << 52));
        if exponent >= 1075 {
            significand << (exponent - 1075)
        } else {
            significand >> (1075 - exponent)
        }
    }

    /// Independent oracle: `n` rounded to `precision` significant bits,
    /// nearest, ties to even, in pure integer arithmetic.
    fn nearest_with_precision(n: u128, precision: u32) -> u128 {
        let width = 128 - n.leading_zeros();
        if width <= precision {
            return n;
        }
        let shift = width - precision;
        let kept = n >> shift;
        let remainder = n & ((1_u128 << shift) - 1);
        let half = 1_u128 << (shift - 1);
        let round_up = remainder > half || (remainder == half && kept & 1 == 1);
        (kept + u128::from(round_up)) << shift
    }

    /// A small deterministic generator (xorshift64*) for sweep tests.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
    }

    fn assert_f32_nearest(n: u64) {
        let got = f32_from_u64(n);
        if n == 0 {
            assert_eq!(got.to_bits(), 0.0_f32.to_bits());
            return;
        }
        assert_eq!(
            f32_integer(got),
            nearest_with_precision(u128::from(n), 24),
            "f32_from_u64({n})"
        );
    }

    fn assert_f64_nearest(n: u64) {
        let got = f64_from_u64(n);
        if n == 0 {
            assert_eq!(got.to_bits(), 0.0_f64.to_bits());
            return;
        }
        assert_eq!(
            f64_integer(got),
            nearest_with_precision(u128::from(n), 53),
            "f64_from_u64({n})"
        );
    }

    #[test]
    fn integer_to_float_known_values() {
        assert_eq!(f64_from_u64(7).to_bits(), 7.0_f64.to_bits());
        assert_eq!(f64_from_u64((1 << 53) + 1).to_bits(), 9_007_199_254_740_992.0_f64.to_bits());
        assert_eq!(f64_from_u64((1 << 53) + 3).to_bits(), 9_007_199_254_740_996.0_f64.to_bits());
        assert_eq!(f64_from_u64(u64::MAX).to_bits(), 18_446_744_073_709_551_616.0_f64.to_bits());
        assert_eq!(f64_from_usize(3).to_bits(), 3.0_f64.to_bits());
        assert_eq!(f64_from_i64(-3).to_bits(), (-3.0_f64).to_bits());
        assert_eq!(f64_from_i64(i64::MIN).to_bits(), (-9_223_372_036_854_775_808.0_f64).to_bits());

        assert_eq!(f32_from_u32(5).to_bits(), 5.0_f32.to_bits());
        assert_eq!(f32_from_u32(u32::MAX).to_bits(), 4_294_967_296.0_f32.to_bits());
        assert_eq!(f32_from_u64((1 << 24) + 1).to_bits(), 16_777_216.0_f32.to_bits());
        assert_eq!(f32_from_u64((1 << 24) + 3).to_bits(), 16_777_220.0_f32.to_bits());
        assert_eq!(f32_from_u64(u64::MAX).to_bits(), 18_446_744_073_709_551_616.0_f32.to_bits());
        assert_eq!(f32_from_usize(5).to_bits(), 5.0_f32.to_bits());
        assert_eq!(f32_from_i64(-5).to_bits(), (-5.0_f32).to_bits());
        assert_eq!(f32_from_i32(i32::MIN).to_bits(), (-2_147_483_648.0_f32).to_bits());
        assert_eq!(f32_from_i32(60).to_bits(), 60.0_f32.to_bits());
    }

    #[test]
    fn sticky_bit_decides_a_tie_below_the_dropped_window() {
        // Top 24 bits, then a lone round bit, then a set bit far below the 32
        // bits the reduction keeps: not a tie, so it rounds up.
        let n = (1_u64 << 63) | (1 << 39) | 1;
        assert_f32_nearest(n);
        assert_eq!(f32_from_u64(n).to_bits(), f32_from_u64((1 << 63) | (1 << 40)).to_bits());
        // The same without the far bit is a true tie and rounds to even (down).
        let tie = (1_u64 << 63) | (1 << 39);
        assert_f32_nearest(tie);
        assert_eq!(f32_from_u64(tie).to_bits(), f32_from_u64(1 << 63).to_bits());
    }

    #[test]
    fn integer_to_float_matches_the_oracle_across_boundaries() {
        for n in 0..(1_u64 << 12) {
            assert_f32_nearest(n);
            assert_f64_nearest(n);
        }
        for base in [1_u64 << 24, 1 << 25, 1 << 31, 1 << 32, 1 << 33, 1 << 53, 1 << 63] {
            for delta in 0..64_u64 {
                assert_f32_nearest(base - 32 + delta);
                assert_f64_nearest(base - 32 + delta);
                assert_f32_nearest(base.wrapping_add(delta << 8));
                assert_f64_nearest(base.wrapping_add(delta << 8));
            }
        }
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for _ in 0..200_000 {
            let n = rng.next();
            assert_f32_nearest(n);
            assert_f64_nearest(n);
            assert_f32_nearest(n >> (n & 63));
            assert_f64_nearest(n >> (n & 63));
        }
    }

    #[test]
    fn f64_to_f32_known_values() {
        let cases: [(f64, u32); 14] = [
            (1.0, 0x3F80_0000),
            (-1.0, 0xBF80_0000),
            (0.0, 0),
            (-0.0, 0x8000_0000),
            (0.1, 0x3DCC_CCCD),
            (3.0, 0x4040_0000),
            (1e40, 0x7F80_0000),
            (-1e40, 0xFF80_0000),
            (1e-50, 0),
            (-1e-50, 0x8000_0000),
            (1e-40, 0x0001_16C2),
            (f64::INFINITY, 0x7F80_0000),
            (f64::NEG_INFINITY, 0xFF80_0000),
            (f64::from(f32::MAX), 0x7F7F_FFFF),
        ];
        for (input, expected) in cases {
            assert_eq!(f32_from_f64(input).to_bits(), expected, "f32_from_f64({input:e})");
        }
        assert!(f32_from_f64(f64::NAN).is_nan());
        assert!(f32_from_f64(-f64::NAN).is_nan());
        assert!(f32_from_f64(-f64::NAN).is_sign_negative());
    }

    #[test]
    fn f64_to_f32_ties_go_to_even_and_carry_into_the_exponent() {
        // 1 + 2^-24 is exactly halfway between 1 and 1 + 2^-23: even is 1.
        let halfway = f64::from_bits(0x3FF0_0000_1000_0000);
        assert_eq!(f32_from_f64(halfway).to_bits(), 1.0_f32.to_bits());
        // 1 + 3 * 2^-24 is halfway between 1 + 2^-23 and 1 + 2^-22: even is the latter.
        let halfway_up = f64::from_bits(0x3FF0_0000_3000_0000);
        assert_eq!(f32_from_f64(halfway_up).to_bits(), 0x3F80_0002);
        // The largest f64 below 2 rounds up to 2.0.
        let just_below_two = f64::from_bits(0x3FFF_FFFF_FFFF_FFFF);
        assert_eq!(f32_from_f64(just_below_two).to_bits(), 2.0_f32.to_bits());
        // f32::MAX plus half an f32 ulp (2^103) is a tie whose even side is infinity.
        let max_plus_half_ulp = f64::from(f32::MAX) + 2_f64.powi(103);
        assert_eq!(f32_from_f64(max_plus_half_ulp).to_bits(), 0x7F80_0000);
        // Just below that tie stays finite.
        let below_tie = f64::from(f32::MAX) + 2_f64.powi(102);
        assert_eq!(f32_from_f64(below_tie).to_bits(), 0x7F7F_FFFF);
        // The largest subnormal plus half a subnormal unit carries into the
        // smallest normal.
        let largest_subnormal = f64::from(f32::from_bits(0x007F_FFFF));
        let carry = largest_subnormal + 2_f64.powi(-150);
        assert_eq!(f32_from_f64(carry).to_bits(), 0x0080_0000);
        // Half the smallest subnormal is a tie with zero, which is even.
        assert_eq!(f32_from_f64(2_f64.powi(-150)).to_bits(), 0);
        // Anything above that half rounds up to the smallest subnormal.
        assert_eq!(f32_from_f64(2_f64.powi(-150) * 1.5).to_bits(), 1);
    }

    #[test]
    fn f64_to_f32_round_trips_every_f32_and_splits_every_midpoint() {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        let mut seen = 0;
        while seen < 200_000 {
            let candidate = f32::from_bits(low_u32(rng.next() & 0xFFFF_FFFF));
            if !candidate.is_finite() {
                continue;
            }
            seen += 1;
            let widened = f64::from(candidate);
            assert_eq!(f32_from_f64(widened).to_bits(), candidate.to_bits(), "round trip {candidate:e}");

            // The midpoint to the next f32 up is exactly representable in f64
            // and must land on whichever neighbour has an even mantissa.
            let next = f32::from_bits(candidate.to_bits() + 1);
            if candidate.is_sign_negative() || !next.is_finite() {
                continue;
            }
            let midpoint = widened.midpoint(f64::from(next));
            let even = if candidate.to_bits() & 1 == 0 { candidate } else { next };
            assert_eq!(f32_from_f64(midpoint).to_bits(), even.to_bits(), "midpoint above {candidate:e}");
        }
    }

    #[test]
    fn float_to_integer_known_values() {
        assert_eq!(u64_from_f64(1.9), 1);
        assert_eq!(u64_from_f64(0.999_999), 0);
        assert_eq!(u64_from_f64(-1.0), 0);
        assert_eq!(u64_from_f64(-0.0), 0);
        assert_eq!(u64_from_f64(f64::NAN), 0);
        assert_eq!(u64_from_f64(f64::INFINITY), u64::MAX);
        assert_eq!(u64_from_f64(f64::NEG_INFINITY), 0);
        assert_eq!(u64_from_f64(18_446_744_073_709_551_616.0), u64::MAX);
        // The largest f64 below 2^64 is 2^64 - 2048 and converts exactly.
        assert_eq!(u64_from_f64(f64::from_bits(0x43EF_FFFF_FFFF_FFFF)), u64::MAX - 2047);
        assert_eq!(u64_from_f64(9_223_372_036_854_775_808.0), 1 << 63);
        assert_eq!(u64_from_f64(4_294_967_296.5), 1 << 32);

        assert_eq!(i64_from_f64(-1.9), -1);
        assert_eq!(i64_from_f64(-0.5), 0);
        assert_eq!(i64_from_f64(f64::NAN), 0);
        assert_eq!(i64_from_f64(-9_223_372_036_854_775_808.0), i64::MIN);
        assert_eq!(i64_from_f64(-1e19), i64::MIN);
        assert_eq!(i64_from_f64(1e19), i64::MAX);
        assert_eq!(i64_from_f64(9_223_372_036_854_775_807.0), i64::MAX);
        assert_eq!(i64_from_f64(-9_223_372_036_854_774_784.0), -9_223_372_036_854_774_784);

        assert_eq!(usize_from_f64(f64::INFINITY), usize::MAX);
        assert_eq!(usize_from_f32(2.5), 2);
        assert_eq!(u32_from_f32(2.5), 2);
        assert_eq!(u32_from_f32(-3.0), 0);
        assert_eq!(u32_from_f32(f32::NAN), 0);
        assert_eq!(u32_from_f32(4_294_967_296.0), u32::MAX);
        assert_eq!(u32_from_f32(4_294_967_040.0), 4_294_967_040);
    }

    #[test]
    fn float_to_integer_truncates_exactly_where_f64_is_exact() {
        let mut rng = Rng(0x2545_F491_4F6C_DD1D);
        for _ in 0..200_000 {
            let magnitude = f64_from_u64(rng.next() >> 11);
            let x = magnitude / 2_f64.powi(i32::try_from(rng.next() % 40).unwrap_or(0));
            assert_eq!(f64_from_u64(u64_from_f64(x)).to_bits(), x.trunc().to_bits(), "trunc {x:e}");
            assert_eq!(f64_from_i64(i64_from_f64(-x)).to_bits(), (-x).trunc().to_bits(), "trunc {:e}", -x);
        }
    }
}
