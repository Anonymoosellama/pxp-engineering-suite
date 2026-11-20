// SPDX-License-Identifier: LGPL-2.1-or-later
//! Objects, their kinds, owning applications, states, layers and property
//! values.

use crate::ObjectId;
use pxp_units::Quantity;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Checks the shared syntax of [`AppId`] and [`ObjectKind`]: ASCII lowercase
/// letters, digits, `_` and `-`, in at least two non-empty segments separated
/// by single dots (`pxp.modeling`, `modeling.block`, `cam.operation.face_mill`).
fn valid_dotted(s: &str) -> bool {
    !s.is_empty()
        && s.contains('.')
        && s.split('.').all(|seg| {
            !seg.is_empty()
                && seg
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        })
}

/// Stable identifier of an NX-style application (`pxp.modeling`). Persisted
/// in files as each object's `owner_app`: never change one after release.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppId(Cow<'static, str>);

impl AppId {
    /// The core itself (owner of expressions and other shared objects).
    pub const CORE: Self = Self::from_static("pxp.core");

    /// Wraps a compile-time string. Not validated here (const context);
    /// [`AppId::is_valid`] is asserted for every registered application in
    /// `pxp-appkit`'s tests.
    #[must_use]
    pub const fn from_static(s: &'static str) -> Self {
        Self(Cow::Borrowed(s))
    }

    /// Parses and validates a runtime string (from a file or a plugin).
    ///
    /// # Errors
    /// [`crate::DocError::InvalidName`] if the syntax is wrong.
    pub fn new(s: impl Into<String>) -> Result<Self, crate::DocError> {
        let s = s.into();
        if valid_dotted(&s) {
            Ok(Self(Cow::Owned(s)))
        } else {
            Err(crate::DocError::InvalidName(s))
        }
    }

    /// The text form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True if the syntax is valid (see [`AppId::new`]).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        valid_dotted(&self.0)
    }
}

impl fmt::Display for AppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Namespaced object kind (`modeling.block`, `drafting.view`,
/// `cam.operation.cavity_mill`). One object table serves every application
/// because kinds are namespaced; objects of kinds this build does not know
/// are preserved untouched.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectKind(Cow<'static, str>);

impl ObjectKind {
    /// NX-style named expression (owned by the core).
    pub const EXPRESSION: Self = Self::from_static("core.expression");

    /// Wraps a compile-time string (see [`AppId::from_static`]).
    #[must_use]
    pub const fn from_static(s: &'static str) -> Self {
        Self(Cow::Borrowed(s))
    }

    /// Parses and validates a runtime string.
    ///
    /// # Errors
    /// [`crate::DocError::InvalidName`] if the syntax is wrong.
    pub fn new(s: impl Into<String>) -> Result<Self, crate::DocError> {
        let s = s.into();
        if valid_dotted(&s) {
            Ok(Self(Cow::Owned(s)))
        } else {
            Err(crate::DocError::InvalidName(s))
        }
    }

    /// The text form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The namespace: the text before the first dot (`modeling`).
    #[must_use]
    pub fn namespace(&self) -> &str {
        self.0.split('.').next().unwrap_or_default()
    }

    /// True if the syntax is valid.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        valid_dotted(&self.0)
    }
}

impl fmt::Display for ObjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Recompute state of an object, shown as a badge in the navigators.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ObjectState {
    /// Results reflect current inputs.
    #[default]
    UpToDate,
    /// An input changed; results are stale. For Modeling this triggers an
    /// immediate recompute; for CAM it means "regenerate needed" and waits
    /// for the programmer.
    OutOfDate,
    /// The last recompute failed; see [`Object::diagnostic`].
    Failed,
    /// Suppressed by the user; skipped by recompute. Its dependants are
    /// recomputed as if it were absent (or are blocked, per their kind).
    Suppressed,
}

/// One of 256 layers, as in NX. Always in `1..=256`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Layer(u16);

impl Layer {
    /// Layer 1, the default work layer.
    pub const DEFAULT: Self = Self(1);

    /// A validated layer number.
    ///
    /// # Errors
    /// [`crate::DocError::InvalidLayer`] outside `1..=256`.
    pub fn new(n: u16) -> Result<Self, crate::DocError> {
        if (1..=256).contains(&n) {
            Ok(Self(n))
        } else {
            Err(crate::DocError::InvalidLayer(n))
        }
    }

    /// The layer number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A typed, unit-aware property value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A flag.
    Bool(bool),
    /// An integer count (pattern instances, layer numbers...).
    Int(i64),
    /// Free text.
    Text(String),
    /// A unit-aware number (internal units; dimensionless allowed).
    Quantity(Quantity),
    /// A reference to another object. Pair it with a dependency
    /// ([`Dep`]) when the referencing object must be recomputed after the
    /// referenced one changes; feature parameters reference their
    /// expression objects this way.
    Ref(ObjectId),
    /// An ordered list of references.
    RefList(Vec<ObjectId>),
    /// Opaque bytes.
    Bytes(Vec<u8>),
}

impl Value {
    /// Rough heap footprint in bytes, for the undo memory cap.
    pub(crate) fn approx_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + match self {
                Self::Text(s) => s.len(),
                Self::RefList(v) => v.len() * 16,
                Self::Bytes(b) => b.len(),
                _ => 0,
            }
    }
}

/// A dependency edge: "this object depends on `on`, in the role `role`".
/// Roles name why (`"param"`, `"section"`, `"target-body"`, `"expr-ref"`) so
/// the Dependencies panel can explain the link.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Dep {
    /// The object depended on.
    pub on: ObjectId,
    /// Why.
    pub role: String,
}

impl Dep {
    /// Builds an edge.
    #[must_use]
    pub fn new(on: ObjectId, role: impl Into<String>) -> Self {
        Self {
            on,
            role: role.into(),
        }
    }
}

/// One object of any application.
///
/// Objects reach callers only as shared, immutable references from a
/// [`crate::Snapshot`] or a [`crate::Tx`]; the only way to change one is
/// [`crate::Tx::update`] inside a transaction. That is how "no command
/// mutates the model outside a transaction" is enforced by the type system.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    pub(crate) id: ObjectId,
    /// Namespaced kind; decides which application's schema the payload has.
    pub kind: ObjectKind,
    /// Payload schema version for this kind (migrations key on it).
    pub kind_ver: u32,
    /// Application that owns the kind's schema.
    pub owner_app: AppId,
    /// Navigator parent (CAM groups, drawing sheets...). Not a dependency.
    pub parent: Option<ObjectId>,
    /// Display name; for expressions, the unique expression name.
    pub name: Option<String>,
    /// Layer 1..=256.
    pub layer: Layer,
    /// Recompute state.
    pub state: ObjectState,
    /// Human-readable reason for `Failed` (or a warning), shown in the
    /// navigator. Never dropped silently.
    pub diagnostic: Option<String>,
    /// Typed properties.
    pub props: BTreeMap<String, Value>,
    /// What this object depends on. The document keeps the reverse index.
    pub deps: BTreeSet<Dep>,
    /// Kind-specific versioned payload, opaque to the core and preserved
    /// byte for byte even when the kind is unknown to this build.
    pub payload: Vec<u8>,
}

impl Object {
    /// The object's stable ID (read-only: an ID never changes).
    #[must_use]
    pub const fn id(&self) -> ObjectId {
        self.id
    }

    /// Convenience accessor for a property.
    #[must_use]
    pub fn prop(&self, key: &str) -> Option<&Value> {
        self.props.get(key)
    }

    /// True if everything except recompute state and diagnostic is equal.
    /// Used to decide whether an edit must invalidate dependants.
    pub(crate) fn content_eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.kind_ver == other.kind_ver
            && self.owner_app == other.owner_app
            && self.parent == other.parent
            && self.name == other.name
            && self.layer == other.layer
            && self.props == other.props
            && self.deps == other.deps
            && self.payload == other.payload
    }

    /// Rough heap footprint in bytes, for the undo memory cap.
    pub(crate) fn approx_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.name.as_ref().map_or(0, String::len)
            + self.diagnostic.as_ref().map_or(0, String::len)
            + self.payload.len()
            + self.deps.iter().map(|d| 32 + d.role.len()).sum::<usize>()
            + self
                .props
                .iter()
                .map(|(k, v)| k.len() + v.approx_size())
                .sum::<usize>()
    }
}

/// Builder for a new object, passed to [`crate::Tx::create`].
///
/// ```
/// use pxp_doc::{AppId, NewObject, ObjectKind, Value};
/// let spec = NewObject::new(ObjectKind::from_static("modeling.block"), AppId::from_static("pxp.modeling"))
///     .name("Block(1)")
///     .prop("note", Value::Text("demo".into()));
/// # let _ = spec;
/// ```
#[derive(Debug, Clone)]
pub struct NewObject {
    pub(crate) object: Object,
}

impl NewObject {
    /// Starts a new object of `kind`, owned by `owner_app`, on layer 1,
    /// version 1, state out of date (it has not been computed yet).
    #[must_use]
    pub fn new(kind: ObjectKind, owner_app: AppId) -> Self {
        Self {
            object: Object {
                id: ObjectId::from_u128(0),
                kind,
                kind_ver: 1,
                owner_app,
                parent: None,
                name: None,
                layer: Layer::DEFAULT,
                state: ObjectState::OutOfDate,
                diagnostic: None,
                props: BTreeMap::new(),
                deps: BTreeSet::new(),
                payload: Vec::new(),
            },
        }
    }

    /// Sets the display name.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.object.name = Some(name.into());
        self
    }

    /// Sets the navigator parent.
    #[must_use]
    pub fn parent(mut self, parent: ObjectId) -> Self {
        self.object.parent = Some(parent);
        self
    }

    /// Sets the layer.
    #[must_use]
    pub fn layer(mut self, layer: Layer) -> Self {
        self.object.layer = layer;
        self
    }

    /// Sets the payload schema version.
    #[must_use]
    pub fn kind_ver(mut self, v: u32) -> Self {
        self.object.kind_ver = v;
        self
    }

    /// Adds a property.
    #[must_use]
    pub fn prop(mut self, key: impl Into<String>, value: Value) -> Self {
        self.object.props.insert(key.into(), value);
        self
    }

    /// Adds a dependency.
    #[must_use]
    pub fn dep(mut self, on: ObjectId, role: impl Into<String>) -> Self {
        self.object.deps.insert(Dep::new(on, role));
        self
    }

    /// Sets the opaque payload.
    #[must_use]
    pub fn payload(mut self, bytes: Vec<u8>) -> Self {
        self.object.payload = bytes;
        self
    }

    /// Sets the initial state (default: out of date). Data-only objects
    /// that need no computation can start up to date.
    #[must_use]
    pub fn state(mut self, state: ObjectState) -> Self {
        self.object.state = state;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_names() {
        assert!(AppId::new("pxp.modeling").is_ok());
        assert!(ObjectKind::new("cam.operation.face_mill").is_ok());
        for bad in ["", "nodot", "Upper.case", "a..b", ".a", "a.", "sp ace.x"] {
            assert!(ObjectKind::new(bad).is_err(), "{bad}");
        }
        assert_eq!(ObjectKind::EXPRESSION.namespace(), "core");
        assert!(AppId::CORE.is_valid());
    }

    #[test]
    fn layers_are_1_to_256() {
        assert!(Layer::new(0).is_err());
        assert!(Layer::new(257).is_err());
        assert_eq!(Layer::new(256).unwrap().get(), 256);
    }

    #[test]
    fn content_eq_ignores_state_and_diagnostic() {
        let a = NewObject::new(ObjectKind::EXPRESSION, AppId::CORE).object;
        let mut b = a.clone();
        b.state = ObjectState::Failed;
        b.diagnostic = Some("x".into());
        assert!(a.content_eq(&b));
        b.props.insert("k".into(), Value::Int(1));
        assert!(!a.content_eq(&b));
    }
}
