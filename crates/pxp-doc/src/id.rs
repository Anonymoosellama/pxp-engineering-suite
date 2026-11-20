// SPDX-License-Identifier: LGPL-2.1-or-later
//! Stable object identifiers (UUIDv7) and where they come from.
//!
//! ## Algorithm: UUIDv7 with a monotonic counter
//!
//! RFC 9562 §5.7 layout, most significant bit first:
//!
//! ```text
//! | unix_ts_ms (48) | ver = 0111 (4) | rand_a (12) | var = 10 (2) | rand_b (62) |
//! ```
//!
//! `rand_a` is used as a 12-bit counter (RFC 9562 §6.2, "Fixed Bit-Length
//! Dedicated Counter", method 1): on a new millisecond it is reseeded with a
//! random value below 2048 (leaving headroom), and within the same
//! millisecond it increments. If it would overflow, or the wall clock steps
//! backwards, the timestamp is advanced by one millisecond instead. IDs from
//! one source are therefore strictly increasing, which keeps B-tree inserts
//! (in memory and in SQLite) append-mostly.
//!
//! Randomness: the standard library offers no public RNG, so the source is
//! seeded from `RandomState` (randomly keyed per process from OS entropy)
//! and expanded with SplitMix64 (Steele, Lea, Flood 2014, "Fast splittable
//! pseudorandom number generators"). Uniqueness, not secrecy, is the goal;
//! object IDs are not security tokens.

use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

/// A stable, never-reused object identifier (UUIDv7, 128 bits).
///
/// Every object of every application is keyed by one; cross-application
/// references are an `ObjectId` plus, for topology, a persistent name. IDs
/// order by creation time, which keeps navigators and file B-trees tidy, but
/// code must not rely on that order for meaning (timestamp order is not
/// feature order).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(u128);

/// Error parsing the canonical text form of an [`ObjectId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdParseError(pub String);

impl fmt::Display for IdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a canonical UUID: '{}'", self.0)
    }
}

impl std::error::Error for IdParseError {}

impl ObjectId {
    /// Wraps a raw 128-bit value (used by file readers and tests).
    #[must_use]
    pub const fn from_u128(v: u128) -> Self {
        Self(v)
    }

    /// The raw 128-bit value.
    #[must_use]
    pub const fn as_u128(self) -> u128 {
        self.0
    }

    /// Big-endian bytes, the byte order UUIDs use in files and databases.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }

    /// Inverse of [`ObjectId::to_bytes`].
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(u128::from_be_bytes(bytes))
    }

    /// The UUID version nibble (7 for IDs made by this crate).
    #[must_use]
    pub fn version(self) -> u8 {
        // Masked to 4 bits, so the conversion cannot fail.
        u8::try_from((self.0 >> 76) & 0xF).unwrap_or(0)
    }

    /// Parses the canonical hyphenated form
    /// (`0190f3c8-1a2b-7c3d-8e4f-0123456789ab`), case-insensitive.
    ///
    /// # Errors
    /// [`IdParseError`] for anything else.
    pub fn parse(text: &str) -> Result<Self, IdParseError> {
        let bad = || IdParseError(text.to_owned());
        let bytes = text.as_bytes();
        if bytes.len() != 36 {
            return Err(bad());
        }
        let mut hex = String::with_capacity(32);
        for (i, &b) in bytes.iter().enumerate() {
            let hyphen_slot = matches!(i, 8 | 13 | 18 | 23);
            match (hyphen_slot, b) {
                (true, b'-') => {}
                (false, b) if b.is_ascii_hexdigit() => hex.push(char::from(b)),
                _ => return Err(bad()),
            }
        }
        u128::from_str_radix(&hex, 16).map(Self).map_err(|_| bad())
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let v = self.0;
        write!(
            f,
            "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
            (v >> 96) & 0xFFFF_FFFF,
            (v >> 80) & 0xFFFF,
            (v >> 64) & 0xFFFF,
            (v >> 48) & 0xFFFF,
            v & 0xFFFF_FFFF_FFFF
        )
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({self})")
    }
}

/// Where a [`Document`](crate::Document) gets new IDs.
///
/// Injectable so that tests and journal replay are deterministic: replaying
/// a recorded session with a [`SequentialIdSource`] reproduces the same IDs,
/// and therefore the same persistent names and geometry hashes.
pub trait IdSource: Send {
    /// Returns a new ID, never returned before by this source.
    fn next_id(&mut self) -> ObjectId;
}

/// Production ID source: time-ordered UUIDv7 (see the module docs).
#[derive(Debug, Clone)]
pub struct UuidV7Source {
    last_ms: u64,
    counter: u16,
    rng: SplitMix64,
}

impl UuidV7Source {
    /// A source seeded from process-random state and the clock.
    #[must_use]
    pub fn new() -> Self {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(unix_ms());
        Self {
            last_ms: 0,
            counter: 0,
            rng: SplitMix64(h.finish()),
        }
    }
}

impl Default for UuidV7Source {
    fn default() -> Self {
        Self::new()
    }
}

impl IdSource for UuidV7Source {
    fn next_id(&mut self) -> ObjectId {
        let now = unix_ms();
        if now > self.last_ms {
            self.last_ms = now;
            self.counter = u16::try_from(self.rng.next() & 0x7FF).unwrap_or(0);
        } else if self.counter >= 0xFFF {
            // Counter exhausted, or the clock went backwards: borrow the next
            // millisecond so IDs stay strictly increasing.
            self.last_ms += 1;
            self.counter = 0;
        } else {
            self.counter += 1;
        }
        let ts = u128::from(self.last_ms & 0xFFFF_FFFF_FFFF);
        let rand_b = u128::from(self.rng.next()) & ((1_u128 << 62) - 1);
        ObjectId(
            (ts << 80) | (0x7 << 76) | (u128::from(self.counter) << 64) | (0b10 << 62) | rand_b,
        )
    }
}

/// Deterministic ID source for tests and journal replay: a fixed timestamp
/// and an incrementing counter, laid out as valid UUIDv7 values.
#[derive(Debug, Clone)]
pub struct SequentialIdSource {
    timestamp_ms: u64,
    counter: u64,
}

impl SequentialIdSource {
    /// Starts at `counter = 0` with the given fixed timestamp.
    #[must_use]
    pub const fn new(timestamp_ms: u64) -> Self {
        Self {
            timestamp_ms,
            counter: 0,
        }
    }
}

impl IdSource for SequentialIdSource {
    fn next_id(&mut self) -> ObjectId {
        self.counter += 1;
        let ts = u128::from(self.timestamp_ms & 0xFFFF_FFFF_FFFF);
        let c = u128::from(self.counter);
        let rand_a = (c >> 62) & 0xFFF;
        let rand_b = c & ((1_u128 << 62) - 1);
        ObjectId((ts << 80) | (0x7 << 76) | (rand_a << 64) | (0b10 << 62) | rand_b)
    }
}

/// SplitMix64 generator (public-domain reference algorithm by Sebastiano
/// Vigna). One 64-bit state word; passes BigCrush; O(1) per output.
#[derive(Debug, Clone)]
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn unix_ms() -> u64 {
    // A clock before 1970 yields 0; IDs stay unique through the counter.
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v7_ids_are_valid_and_strictly_increasing() {
        let mut src = UuidV7Source::new();
        let mut prev = src.next_id();
        for _ in 0..20_000 {
            let id = src.next_id();
            assert_eq!(id.version(), 7);
            assert_eq!((id.as_u128() >> 62) & 0b11, 0b10, "RFC 9562 variant bits");
            assert!(id > prev, "ids must be strictly increasing");
            prev = id;
        }
    }

    #[test]
    fn text_round_trip() {
        let mut src = SequentialIdSource::new(1_700_000_000_000);
        let id = src.next_id();
        let text = id.to_string();
        assert_eq!(text.len(), 36);
        assert_eq!(ObjectId::parse(&text).unwrap(), id);
        assert_eq!(ObjectId::parse(&text.to_uppercase()).unwrap(), id);
        assert_eq!(ObjectId::from_bytes(id.to_bytes()), id);
    }

    #[test]
    fn parse_rejects_non_canonical() {
        for bad in [
            "",
            "not-a-uuid",
            "0190f3c81a2b7c3d8e4f0123456789ab",
            "0190f3c8-1a2b-7c3d-8e4f-0123456789ag",
        ] {
            assert!(ObjectId::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn sequential_source_is_deterministic() {
        let a: Vec<_> = {
            let mut s = SequentialIdSource::new(5);
            (0..3).map(|_| s.next_id()).collect()
        };
        let b: Vec<_> = {
            let mut s = SequentialIdSource::new(5);
            (0..3).map(|_| s.next_id()).collect()
        };
        assert_eq!(a, b);
        assert!(a.windows(2).all(|w| w[0] < w[1]));
    }
}
