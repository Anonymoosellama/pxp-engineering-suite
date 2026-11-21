// SPDX-License-Identifier: LGPL-2.1-or-later
//! A stable hasher for persisted, cross-platform hashes.
//!
//! ## Algorithm
//!
//! FNV-1a, 64-bit (Fowler, Noll, Vo; public domain; see
//! <http://www.isthe.com/chongo/tech/comp/fnv/>): for each byte,
//! `h = (h XOR byte) * 0x100000001b3`, starting from `0xcbf29ce484222325`.
//! O(n) in the bytes hashed, no allocation, and the output depends only on
//! the input bytes. Integers are written little-endian so the result is the
//! same on big- and little-endian machines.
//!
//! FNV-1a is **not** cryptographic. It is used to detect change (geometry
//! hash identical after save and reopen; identical across OSes), not to
//! defend against an adversary. The algorithm id is part of every persisted
//! hash ([`StableHasher::ALGORITHM`]) so it can be upgraded (for example to
//! BLAKE3 once crates.io is reachable) without misreading old files.
//!
//! ## Floating-point canonicalisation
//!
//! `-0.0` and `+0.0` compare equal but have different bits, and NaN has many
//! bit patterns. [`StableHasher::write_f64`] maps both zeros to `+0.0` and all
//! NaNs to one canonical NaN before hashing, so equal values hash equally.

/// Stable 64-bit FNV-1a hasher. See the module docs.
///
/// ```
/// use pxp_math::StableHasher;
/// let mut a = StableHasher::new();
/// a.write_f64(-0.0);
/// let mut b = StableHasher::new();
/// b.write_f64(0.0);
/// assert_eq!(a.finish(), b.finish());
/// ```
#[derive(Debug, Clone)]
pub struct StableHasher {
    state: u64,
}

impl StableHasher {
    /// Identifier stored next to persisted hashes.
    pub const ALGORITHM: &'static str = "fnv1a-64";
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    /// A hasher in its initial state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: Self::OFFSET,
        }
    }

    /// Feeds raw bytes.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.state ^= u64::from(b);
            self.state = self.state.wrapping_mul(Self::PRIME);
        }
    }

    /// Feeds a `u64`, little-endian.
    pub fn write_u64(&mut self, v: u64) {
        self.write_bytes(&v.to_le_bytes());
    }

    /// Feeds a string, length-prefixed so `("ab","c")` and `("a","bc")`
    /// hash differently.
    pub fn write_str(&mut self, s: &str) {
        self.write_u64(s.len() as u64);
        self.write_bytes(s.as_bytes());
    }

    /// Feeds an `f64` after canonicalising signed zero and NaN.
    pub fn write_f64(&mut self, v: f64) {
        let canonical = if v == 0.0 {
            0.0_f64
        } else if v.is_nan() {
            f64::NAN
        } else {
            v
        };
        self.write_u64(canonical.to_bits());
    }

    /// Feeds an `f64` rounded to a multiple of `resolution` (for example the
    /// part's distance tolerance divided by 1000). Use this for values that
    /// pass through transcendental functions, whose last bit may differ
    /// between platform maths libraries. A non-positive or non-finite
    /// resolution falls back to [`StableHasher::write_f64`].
    pub fn write_f64_quantized(&mut self, v: f64, resolution: f64) {
        if resolution.is_finite() && resolution > 0.0 && v.is_finite() {
            self.write_f64((v / resolution).round() * resolution);
        } else {
            self.write_f64(v);
        }
    }

    /// Feeds an `f64` rounded to `digits` significant decimal digits (1 to
    /// 15), for magnitudes with no natural absolute resolution (a volume, an
    /// area). What is hashed is the rounded value's decimal exponent and
    /// integer mantissa, never a re-scaled float, so two inputs that round
    /// to the same digits always hash alike, including across a power of
    /// ten (`99_999.999_999_9` and `100_000.000_000_1` at 9 digits). Zero,
    /// NaN and infinities hash as themselves.
    pub fn write_f64_significant(&mut self, v: f64, digits: u8) {
        let digits = i32::from(digits.clamp(1, 15));
        if v == 0.0 || !v.is_finite() {
            self.write_f64(v);
            return;
        }
        // Exponent of the last kept digit: v ≈ mantissa × 10^exp, with
        // mantissa in [10^(digits-1), 10^digits).
        // The f64 to i32 cast is exact: |log10| of a finite f64 is < 400.
        #[allow(clippy::cast_possible_truncation)]
        let mut exp = v.abs().log10().floor() as i32 - (digits - 1);
        let mut mantissa = (v.abs() / 10_f64.powi(exp)).round();
        let top = 10_f64.powi(digits);
        if mantissa >= top {
            // Rounding carried into a new digit (9.99… → 10.0…).
            mantissa = (mantissa / 10.0).round();
            exp += 1;
        } else if mantissa < top / 10.0 {
            // log10 rounded the other way at a power of ten.
            exp -= 1;
            mantissa = (v.abs() / 10_f64.powi(exp)).round();
        }
        self.write_bytes(&[u8::from(v < 0.0)]);
        self.write_u64(u64::from(exp.cast_unsigned()));
        // mantissa < 10^15 < 2^53: exact in f64 and in u64.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        self.write_u64(mantissa as u64);
    }

    /// The hash of everything written so far.
    #[must_use]
    pub const fn finish(&self) -> u64 {
        self.state
    }
}

impl Default for StableHasher {
    fn default() -> Self {
        Self::new()
    }
}

// Exact float equality is intended in these tests: the inputs are small
// integers or exact binary fractions, so the results are exact in IEEE-754.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn sig(v: f64, digits: u8) -> u64 {
        let mut h = StableHasher::new();
        h.write_f64_significant(v, digits);
        h.finish()
    }

    #[test]
    fn significant_digits_absorb_last_bit_noise_and_decades() {
        // 3 in = 76.19999999999999 mm in f64; 76.2 mm typed directly.
        let a: f64 = 25.4 * 50.8 * (3.0 * 25.4);
        let b: f64 = 25.4 * 50.8 * 76.2;
        assert_ne!(a.to_bits(), b.to_bits());
        assert_eq!(sig(a, 9), sig(b, 9));
        // Across a power of ten.
        assert_eq!(sig(99_999.999_999_9, 9), sig(100_000.000_000_1, 9));
        assert_eq!(sig(1_000.0, 9), sig(1_000.000_000_000_1, 9));
        // Different values, sign and digits still differ.
        assert_ne!(sig(98_322.38, 9), sig(98_322.39, 9));
        assert_ne!(sig(-5.0, 9), sig(5.0, 9));
        assert_ne!(sig(5.0, 9), sig(50.0, 9));
        assert_eq!(sig(0.0, 9), sig(-0.0, 9));
    }

    #[test]
    fn known_fnv1a_vectors() {
        // Reference values from the FNV specification test suite.
        let mut h = StableHasher::new();
        assert_eq!(h.finish(), 0xcbf2_9ce4_8422_2325);
        h.write_bytes(b"a");
        assert_eq!(h.finish(), 0xaf63_dc4c_8601_ec8c);
        let mut h = StableHasher::new();
        h.write_bytes(b"foobar");
        assert_eq!(h.finish(), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn nan_and_zero_are_canonical() {
        let hash = |v: f64| {
            let mut h = StableHasher::new();
            h.write_f64(v);
            h.finish()
        };
        assert_eq!(hash(0.0), hash(-0.0));
        assert_eq!(hash(f64::NAN), hash(-f64::NAN));
        assert_ne!(hash(1.0), hash(1.0 + f64::EPSILON));
    }

    #[test]
    fn quantization_absorbs_last_bit_noise() {
        let hash = |v: f64| {
            let mut h = StableHasher::new();
            h.write_f64_quantized(v, 1e-9);
            h.finish()
        };
        assert_eq!(hash(0.5), hash(0.5 + 1e-15));
    }

    #[test]
    fn strings_are_length_prefixed() {
        let pair = |a: &str, b: &str| {
            let mut h = StableHasher::new();
            h.write_str(a);
            h.write_str(b);
            h.finish()
        };
        assert_ne!(pair("ab", "c"), pair("a", "bc"));
    }
}
