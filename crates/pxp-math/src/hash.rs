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
