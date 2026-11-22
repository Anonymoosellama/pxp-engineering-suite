// SPDX-License-Identifier: LGPL-2.1-or-later
//! A stable-toolchain soak of the journal codec: the same properties as the
//! `cargo fuzz` targets in `fuzz/` (decoding never panics or hangs, and
//! whatever decodes re-encodes to the same bytes), driven by seeded
//! mutation instead of coverage guidance.
//!
//! It complements the fuzzers, which need a nightly toolchain (CI and the
//! nightly workflow run them). It is not a substitute for coverage-guided
//! fuzzing. It runs a short burst in every `cargo test`; set
//! `PXP_SOAK_SECS=600` for a long run:
//!
//! ```sh
//! PXP_SOAK_SECS=600 cargo test -p pxp-format --test codec_soak -- --nocapture
//! ```
#![allow(clippy::unwrap_used, clippy::indexing_slicing, missing_docs)]

use pxp_doc::{AppId, Document, NewObject, ObjectKind, Value};
use pxp_format::codec::{decode_object, decode_props, encode_object, encode_props};
use pxp_units::{Dim, Quantity};
use std::time::{Duration, Instant};

/// xorshift64*: small, fast, and reproducible from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % (n.max(1) as u64)).unwrap_or(0)
    }
}

/// Valid encodings to mutate: objects with every value type.
fn seeds() -> Vec<Vec<u8>> {
    let mut d = Document::new();
    d.transact("seed", |tx| {
        let w = tx.set_expression("width", "4in")?;
        tx.set_expression("p1", "width * 2 + 1mm")?;
        tx.create(
            NewObject::new(
                ObjectKind::from_static("modeling.block"),
                AppId::from_static("pxp.modeling"),
            )
            .name("Block(1)")
            .dep(w, "param")
            .prop("note", Value::Text("héllo".into()))
            .prop(
                "len",
                Value::Quantity(Quantity::from_internal(12.5, Dim::LENGTH)),
            )
            .prop("count", Value::Int(-7))
            .prop("on", Value::Bool(true))
            .prop("ref", Value::Ref(w))
            .payload((0..64).collect()),
        )?;
        Ok(())
    })
    .unwrap();
    let snap = d.snapshot();
    let mut out: Vec<Vec<u8>> = snap.objects().map(encode_object).collect();
    out.extend(snap.objects().map(|o| encode_props(&o.props)));
    out
}

fn mutate(rng: &mut Rng, seed: &[u8], pool: &[Vec<u8>]) -> Vec<u8> {
    let mut v = seed.to_vec();
    for _ in 0..=rng.below(4) {
        match rng.below(6) {
            0 if !v.is_empty() => {
                let i = rng.below(v.len());
                v[i] ^= 1 << rng.below(8);
            }
            1 if !v.is_empty() => {
                let i = rng.below(v.len());
                v[i] = [0, 0xff, 0x7f, 0x80][rng.below(4)];
            }
            2 => v.truncate(rng.below(v.len() + 1)),
            3 => {
                let i = rng.below(v.len() + 1);
                let b = u8::try_from(rng.next() & 0xff).unwrap_or(0);
                v.insert(i, b);
            }
            4 => {
                // Splice a slice of another seed in: realistic length fields
                // in the wrong place.
                let other = &pool[rng.below(pool.len())];
                let a = rng.below(other.len() + 1);
                let b = a + rng.below(other.len() - a + 1);
                let at = rng.below(v.len() + 1);
                v.splice(at..at, other[a..b].iter().copied());
            }
            _ => {
                // A huge length prefix at a random place: must not
                // over-allocate.
                let at = rng.below(v.len() + 1);
                v.splice(at..at, [0xff, 0xff, 0xff, 0x7f]);
            }
        }
    }
    v
}

#[test]
fn codec_survives_mutated_input() {
    let secs: u64 = std::env::var("PXP_SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let pool = seeds();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let deadline = Instant::now() + Duration::from_secs(secs);
    let (mut runs, mut decoded, mut slowest) = (0_u64, 0_u64, Duration::ZERO);
    while Instant::now() < deadline {
        for _ in 0..1000 {
            let seed = &pool[rng.below(pool.len())];
            let input = mutate(&mut rng, seed, &pool);
            let t = Instant::now();
            if let Ok(o) = decode_object(&input) {
                decoded += 1;
                assert_eq!(encode_object(&o), input, "object codec is canonical");
            }
            if let Ok(p) = decode_props(&input) {
                decoded += 1;
                assert_eq!(encode_props(&p), input, "props codec is canonical");
            }
            slowest = slowest.max(t.elapsed());
            runs += 1;
        }
    }
    eprintln!("codec soak: {runs} inputs in {secs} s, {decoded} decoded, slowest {slowest:?}");
    assert!(
        slowest < Duration::from_secs(1),
        "no input may take a second to decode"
    );
}
