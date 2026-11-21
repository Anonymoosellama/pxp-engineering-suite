// SPDX-License-Identifier: LGPL-2.1-or-later
//! The explicit binary encoding of object properties and whole objects.
//!
//! ## Why not serde
//!
//! A `.pxp` file must stay readable for decades. Deriving its format from Rust
//! struct layouts (serde) would let an innocent refactor change the file
//! format. This codec is written out by hand, versioned, and documented here,
//! so the bytes on disk change only when this file says so.
//!
//! ## Layout (codec version 1, all integers little-endian)
//!
//! ```text
//! props   := version:u8(=1) count:u32 (key:str value)*      keys strictly ascending
//! value   := 1 bool:u8(0|1)
//!          | 2 int:i64
//!          | 3 text:str
//!          | 4 quantity: value:f64 dim_length:i8 dim_mass:i8 dim_time:i8 dim_angle:i8
//!          | 5 ref: id:[u8;16]
//!          | 6 ref_list: count:u32 id:[u8;16]*
//!          | 7 bytes: bytes
//! str     := len:u32 utf8[len]
//! bytes   := len:u32 u8[len]
//!
//! object  := version:u8(=1) id:[u8;16] kind:str kind_ver:u32 owner_app:str
//!            parent:opt_id name:opt_str layer:u16 state:u8 diagnostic:opt_str
//!            deps_count:u32 (on:[u8;16] role:str)*                 strictly ascending
//!            props_body (props without its version byte) payload:bytes
//! opt_id  := 0 | 1 id:[u8;16]          opt_str := 0 | 1 str
//! state   := 0 up to date | 1 out of date | 2 failed | 3 suppressed
//! ```
//!
//! The encoding is **canonical**: map keys and dependency sets are written in
//! sorted order and the decoder rejects anything else, so one document has
//! exactly one byte representation (stable hashes, reproducible files).
//!
//! ## Hostile input
//!
//! Every length is checked against the bytes actually remaining before
//! anything is allocated, so a forged length cannot request gigabytes, and
//! every slice access is bounds-checked. Decoding is O(input length) and
//! never panics; it is the first `cargo fuzz` target (`fuzz/`).

use pxp_doc::{AppId, Dep, Layer, NewObject, Object, ObjectId, ObjectKind, ObjectState, Value};
use pxp_units::{Dim, Quantity};
use std::collections::BTreeMap;

/// Version byte written at the start of every encoded props map and object.
pub const CODEC_VERSION: u8 = 1;

/// Why decoding failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CodecError {
    /// The input ended before the structure did.
    #[error("data ends early (needed {needed} more bytes at offset {offset})")]
    Truncated {
        /// Byte offset of the read that failed.
        offset: usize,
        /// Bytes it needed.
        needed: usize,
    },
    /// The version byte is not one this build understands (a newer writer).
    #[error("codec version {0} is newer than this build understands (max {CODEC_VERSION})")]
    UnknownVersion(u8),
    /// A value tag is not defined in this codec version.
    #[error("unknown value tag {0}")]
    UnknownTag(u8),
    /// Bytes remain after the structure ended.
    #[error("{0} unexpected trailing bytes")]
    TrailingBytes(usize),
    /// Structurally valid bytes with invalid meaning (bad UTF-8, a boolean
    /// that is neither 0 nor 1, unsorted keys, an invalid kind name...).
    #[error("invalid data: {0}")]
    Invalid(String),
}

// ------------------------------------------------------------------ writer --

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_bits().to_le_bytes());
    }
    fn i8(&mut self, v: i8) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn id(&mut self, id: ObjectId) {
        self.0.extend_from_slice(&id.to_bytes());
    }
    /// Lengths above `u32::MAX` cannot occur for in-memory data written to
    /// SQLite (whose blob limit is far lower); they are clamped defensively
    /// and would then fail to decode rather than corrupt silently.
    fn len(&mut self, n: usize) {
        self.u32(u32::try_from(n).unwrap_or(u32::MAX));
    }
    fn bytes(&mut self, b: &[u8]) {
        self.len(b.len());
        self.0.extend_from_slice(b);
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    fn opt_str(&mut self, s: Option<&str>) {
        match s {
            None => self.u8(0),
            Some(s) => {
                self.u8(1);
                self.str(s);
            }
        }
    }
}

fn write_value(w: &mut Writer, v: &Value) {
    match v {
        Value::Bool(b) => {
            w.u8(1);
            w.u8(u8::from(*b));
        }
        Value::Int(i) => {
            w.u8(2);
            w.i64(*i);
        }
        Value::Text(s) => {
            w.u8(3);
            w.str(s);
        }
        Value::Quantity(q) => {
            w.u8(4);
            w.f64(q.value());
            let d = q.dim();
            w.i8(d.length);
            w.i8(d.mass);
            w.i8(d.time);
            w.i8(d.angle);
        }
        Value::Ref(id) => {
            w.u8(5);
            w.id(*id);
        }
        Value::RefList(ids) => {
            w.u8(6);
            w.len(ids.len());
            for id in ids {
                w.id(*id);
            }
        }
        Value::Bytes(b) => {
            w.u8(7);
            w.bytes(b);
        }
    }
}

fn write_props_body(w: &mut Writer, props: &BTreeMap<String, Value>) {
    w.len(props.len());
    for (k, v) in props {
        w.str(k);
        write_value(w, v);
    }
}

/// Encodes a property map (canonical: keys in sorted order).
#[must_use]
pub fn encode_props(props: &BTreeMap<String, Value>) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.u8(CODEC_VERSION);
    write_props_body(&mut w, props);
    w.0
}

fn state_code(s: ObjectState) -> u8 {
    match s {
        ObjectState::UpToDate => 0,
        ObjectState::OutOfDate => 1,
        ObjectState::Failed => 2,
        ObjectState::Suppressed => 3,
    }
}

/// The integer stored in the `objects.state` column.
#[must_use]
pub fn state_to_i64(s: ObjectState) -> i64 {
    i64::from(state_code(s))
}

/// Inverse of [`state_to_i64`].
///
/// # Errors
/// [`CodecError::Invalid`] for an unknown state number.
pub fn state_from_i64(v: i64) -> Result<ObjectState, CodecError> {
    match v {
        0 => Ok(ObjectState::UpToDate),
        1 => Ok(ObjectState::OutOfDate),
        2 => Ok(ObjectState::Failed),
        3 => Ok(ObjectState::Suppressed),
        other => Err(CodecError::Invalid(format!("unknown object state {other}"))),
    }
}

/// Encodes a whole object (journal rows, fuzzing, tests).
#[must_use]
pub fn encode_object(o: &Object) -> Vec<u8> {
    let mut w = Writer(Vec::new());
    w.u8(CODEC_VERSION);
    w.id(o.id());
    w.str(o.kind.as_str());
    w.u32(o.kind_ver);
    w.str(o.owner_app.as_str());
    match o.parent {
        None => w.u8(0),
        Some(p) => {
            w.u8(1);
            w.id(p);
        }
    }
    w.opt_str(o.name.as_deref());
    w.u16(o.layer.get());
    w.u8(state_code(o.state));
    w.opt_str(o.diagnostic.as_deref());
    w.len(o.deps.len());
    for d in &o.deps {
        w.id(d.on);
        w.str(&d.role);
    }
    write_props_body(&mut w, &o.props);
    w.bytes(&o.payload);
    w.0
}

// ------------------------------------------------------------------ reader --

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CodecError> {
        let truncated = CodecError::Truncated {
            offset: self.pos,
            needed: n,
        };
        let end = self.pos.checked_add(n).ok_or(truncated.clone())?;
        let slice = self.buf.get(self.pos..end).ok_or(truncated)?;
        self.pos = end;
        Ok(slice)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let s = self.take(N)?;
        s.try_into().map_err(|_| CodecError::Truncated {
            offset: self.pos,
            needed: N,
        })
    }
    fn u8(&mut self) -> Result<u8, CodecError> {
        Ok(u8::from_le_bytes(self.array()?))
    }
    fn u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, CodecError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn i64(&mut self) -> Result<i64, CodecError> {
        Ok(i64::from_le_bytes(self.array()?))
    }
    fn f64(&mut self) -> Result<f64, CodecError> {
        Ok(f64::from_bits(u64::from_le_bytes(self.array()?)))
    }
    fn i8(&mut self) -> Result<i8, CodecError> {
        Ok(i8::from_le_bytes(self.array()?))
    }
    fn id(&mut self) -> Result<ObjectId, CodecError> {
        Ok(ObjectId::from_bytes(self.array()?))
    }
    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }
    /// A count of items each at least `min_item` bytes long, rejected early
    /// if the remaining input could not possibly hold that many.
    fn count(&mut self, min_item: usize) -> Result<usize, CodecError> {
        let at = self.pos;
        let n = usize::try_from(self.u32()?).map_err(|_| CodecError::Invalid("count".into()))?;
        let needed = n.saturating_mul(min_item.max(1));
        if needed > self.remaining() {
            return Err(CodecError::Truncated { offset: at, needed });
        }
        Ok(n)
    }
    fn bytes(&mut self) -> Result<&'a [u8], CodecError> {
        let n = self.count(1)?;
        self.take(n)
    }
    fn str(&mut self) -> Result<&'a str, CodecError> {
        let at = self.pos;
        std::str::from_utf8(self.bytes()?)
            .map_err(|_| CodecError::Invalid(format!("text at offset {at} is not UTF-8")))
    }
    fn opt_str(&mut self) -> Result<Option<String>, CodecError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.str()?.to_owned())),
            f => Err(CodecError::Invalid(format!("option flag {f}"))),
        }
    }
    fn version(&mut self) -> Result<(), CodecError> {
        match self.u8()? {
            CODEC_VERSION => Ok(()),
            v if v > CODEC_VERSION => Err(CodecError::UnknownVersion(v)),
            v => Err(CodecError::Invalid(format!("codec version {v}"))),
        }
    }
    fn finish(&self) -> Result<(), CodecError> {
        match self.remaining() {
            0 => Ok(()),
            n => Err(CodecError::TrailingBytes(n)),
        }
    }
}

fn read_value(r: &mut Reader<'_>) -> Result<Value, CodecError> {
    Ok(match r.u8()? {
        1 => match r.u8()? {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            b => return Err(CodecError::Invalid(format!("boolean byte {b}"))),
        },
        2 => Value::Int(r.i64()?),
        3 => Value::Text(r.str()?.to_owned()),
        4 => {
            let v = r.f64()?;
            let dim = Dim::new(r.i8()?, r.i8()?, r.i8()?, r.i8()?);
            Value::Quantity(Quantity::from_internal(v, dim))
        }
        5 => Value::Ref(r.id()?),
        6 => {
            let n = r.count(16)?;
            let mut ids = Vec::with_capacity(n);
            for _ in 0..n {
                ids.push(r.id()?);
            }
            Value::RefList(ids)
        }
        7 => Value::Bytes(r.bytes()?.to_vec()),
        tag => return Err(CodecError::UnknownTag(tag)),
    })
}

fn read_props_body(r: &mut Reader<'_>) -> Result<BTreeMap<String, Value>, CodecError> {
    // Each entry is at least a 4-byte key length plus a 1-byte tag.
    let n = r.count(5)?;
    let mut props = BTreeMap::new();
    let mut last: Option<String> = None;
    for _ in 0..n {
        let key = r.str()?.to_owned();
        if last.as_ref().is_some_and(|prev| *prev >= key) {
            return Err(CodecError::Invalid(format!(
                "property keys not strictly ascending at '{key}'"
            )));
        }
        let value = read_value(r)?;
        last = Some(key.clone());
        props.insert(key, value);
    }
    Ok(props)
}

/// Decodes a property map written by [`encode_props`].
///
/// # Errors
/// [`CodecError`] for truncated, unknown-version, non-canonical or otherwise
/// invalid input.
pub fn decode_props(bytes: &[u8]) -> Result<BTreeMap<String, Value>, CodecError> {
    let mut r = Reader { buf: bytes, pos: 0 };
    r.version()?;
    let props = read_props_body(&mut r)?;
    r.finish()?;
    Ok(props)
}

/// Decodes an object written by [`encode_object`]. Kind and application
/// names and the layer are validated; dependency targets are not (the
/// document checks the whole graph when the object is loaded).
///
/// # Errors
/// [`CodecError`].
pub fn decode_object(bytes: &[u8]) -> Result<Object, CodecError> {
    let mut r = Reader { buf: bytes, pos: 0 };
    r.version()?;
    let id = r.id()?;
    let kind = ObjectKind::new(r.str()?).map_err(|e| CodecError::Invalid(e.to_string()))?;
    let kind_ver = r.u32()?;
    let owner = AppId::new(r.str()?).map_err(|e| CodecError::Invalid(e.to_string()))?;
    let parent = match r.u8()? {
        0 => None,
        1 => Some(r.id()?),
        f => return Err(CodecError::Invalid(format!("parent flag {f}"))),
    };
    let name = r.opt_str()?;
    let layer = Layer::new(r.u16()?).map_err(|e| CodecError::Invalid(e.to_string()))?;
    let state = state_from_i64(i64::from(r.u8()?))?;
    let diagnostic = r.opt_str()?;
    // Each dependency is a 16-byte ID plus a 4-byte role length.
    let n_deps = r.count(20)?;
    let mut deps = Vec::with_capacity(n_deps);
    for _ in 0..n_deps {
        let dep = Dep::new(r.id()?, r.str()?);
        if deps.last().is_some_and(|prev: &Dep| *prev >= dep) {
            return Err(CodecError::Invalid(
                "dependencies not strictly ascending".into(),
            ));
        }
        deps.push(dep);
    }
    let props = read_props_body(&mut r)?;
    let payload = r.bytes()?.to_vec();
    r.finish()?;
    let mut spec = NewObject::new(kind, owner)
        .kind_ver(kind_ver)
        .layer(layer)
        .state(state)
        .diagnostic(diagnostic)
        .payload(payload);
    if let Some(p) = parent {
        spec = spec.parent(p);
    }
    if let Some(n) = name {
        spec = spec.name(n);
    }
    for d in deps {
        spec = spec.dep(d.on, d.role);
    }
    for (k, v) in props {
        spec = spec.prop(k, v);
    }
    Ok(Object::restore(id, spec))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Object {
        let spec = NewObject::new(
            ObjectKind::from_static("modeling.block"),
            AppId::from_static("pxp.modeling"),
        )
        .name("Block(1)")
        .parent(ObjectId::from_u128(7))
        .dep(ObjectId::from_u128(3), "param")
        .dep(ObjectId::from_u128(2), "param")
        .prop("flag", Value::Bool(true))
        .prop("count", Value::Int(-42))
        .prop("note", Value::Text("héllo".into()))
        .prop(
            "len",
            Value::Quantity(Quantity::from_internal(25.4, Dim::LENGTH)),
        )
        .prop("ref", Value::Ref(ObjectId::from_u128(9)))
        .prop(
            "refs",
            Value::RefList(vec![ObjectId::from_u128(1), ObjectId::from_u128(2)]),
        )
        .prop("raw", Value::Bytes(vec![0, 255, 7]))
        .diagnostic(Some("tolerance relaxed".into()))
        .payload((0..=255).collect());
        Object::restore(
            ObjectId::from_u128(0x0123_4567_89ab_7def_8123_4567_89ab_cdef),
            spec,
        )
    }

    #[test]
    fn object_round_trip_is_exact() {
        let o = sample();
        let bytes = encode_object(&o);
        assert_eq!(decode_object(&bytes).unwrap(), o);
        assert_eq!(
            encode_object(&decode_object(&bytes).unwrap()),
            bytes,
            "canonical"
        );
    }

    #[test]
    fn props_round_trip() {
        let props = sample().props;
        assert_eq!(decode_props(&encode_props(&props)).unwrap(), props);
        assert_eq!(
            decode_props(&encode_props(&BTreeMap::new())).unwrap(),
            BTreeMap::new()
        );
    }

    #[test]
    fn every_truncation_is_an_error_not_a_panic() {
        let bytes = encode_object(&sample());
        for cut in 0..bytes.len() {
            assert!(decode_object(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut long = bytes.clone();
        long.push(0);
        assert_eq!(decode_object(&long), Err(CodecError::TrailingBytes(1)));
    }

    #[test]
    fn forged_lengths_do_not_allocate() {
        // Version, then a props count of u32::MAX with nothing behind it.
        let forged = [CODEC_VERSION, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(matches!(
            decode_props(&forged),
            Err(CodecError::Truncated { .. })
        ));
    }

    #[test]
    fn non_canonical_and_unknown_input_is_rejected() {
        let mut w = Writer(Vec::new());
        w.u8(CODEC_VERSION);
        w.u32(2);
        w.str("b");
        write_value(&mut w, &Value::Int(1));
        w.str("a"); // out of order
        write_value(&mut w, &Value::Int(2));
        assert!(matches!(decode_props(&w.0), Err(CodecError::Invalid(_))));
        assert_eq!(
            decode_props(&[9, 0, 0, 0, 0]),
            Err(CodecError::UnknownVersion(9))
        );
        let unknown_tag = [CODEC_VERSION, 1, 0, 0, 0, 1, 0, 0, 0, b'k', 99];
        assert_eq!(decode_props(&unknown_tag), Err(CodecError::UnknownTag(99)));
    }

    #[test]
    fn random_bytes_never_panic() {
        // Deterministic xorshift input; cargo-fuzz explores far more in CI.
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let reference = encode_object(&sample());
        for round in 0..20_000_u32 {
            let mut data = reference.clone();
            for _ in 0..=(round % 8) {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let len = data.len();
                if let Some(b) = data.get_mut(usize::try_from(x).unwrap_or(0) % len) {
                    *b = (x >> 32).to_le_bytes()[0];
                }
            }
            let _ = decode_object(&data);
            let _ = decode_props(&data);
        }
    }
}
