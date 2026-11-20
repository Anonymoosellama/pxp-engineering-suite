// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-kernel: geometry kernel contracts
//!
//! Every piece of geometry in PXP Engineering Suite is computed through the
//! [`GeometryKernel`] trait defined here, and every topological operation
//! returns an [`OpResult`] whose [`TopoHistory`] says where each output face,
//! edge and vertex came from. Application code never calls a backend
//! directly; the shell picks a backend and wraps it in a [`KernelHost`].
//!
//! ## Responsibility
//!
//! - The backend-neutral vocabulary: [`ShapeHandle`], [`BodyKind`],
//!   [`TopoId`], [`TopoHistory`], [`KernelWarning`], [`KernelError`],
//!   [`OpResult`], [`MassProperties`], primitive specifications.
//! - The [`GeometryKernel`] trait that backends implement (Stage 0
//!   bake-off: SolidCore in-process via `pxp-kernel-native`, OCCT in a worker
//!   process via `pxp-kernel-occt`; ADR-0004 records the winner).
//! - [`KernelHost`]: the single entry point applications use. In-process
//!   hosting with panic isolation exists now; out-of-process worker hosting
//!   for native backends arrives with sub-stage 0F.
//!
//! ## Why the history map matters
//!
//! Persistent naming (how "the top edge" survives an edit to the extrude
//! that made it) is built entirely from `TopoHistory`. Drafting dimensions,
//! CAM face selections and CAE loads all reference topology through those
//! names. A backend that returns an incomplete history silently breaks
//! associativity in three applications, so [`TopoHistory::check_complete`]
//! exists and backend conformance tests must call it on every operation.
//!
//! ## Data flow
//!
//! ```text
//! application recompute ─► KernelHost::call ─► GeometryKernel op ─► OpResult
//!                                                             ├─ shape   ─► stored by pxp-format (exact B-rep)
//!                                                             ├─ history ─► persistent names (pxp-doc)
//!                                                             └─ warnings ─► navigator badges
//! ```
//!
//! ## Invariants
//!
//! - Every output entity of an operation appears in its history (generated
//!   from, or modified from, named sources).
//! - Identical inputs produce identical topology order, history and
//!   [`GeometryKernel::geometry_hash`] on every OS (determinism rule).
//! - Unsupported operations return [`KernelError::Unsupported`]; a backend
//!   never returns plausible-looking geometry it did not compute.
//!
//! ## Must never
//!
//! - Depend on `pxp-doc`, UI or application crates (the kernel layer knows
//!   nothing about documents).
//! - Let a backend panic escape: [`KernelHost`] converts it into
//!   [`KernelError::BackendPanicked`].
#![forbid(unsafe_code)]

use pxp_math::{Csys, Point3, UnitVec3};
use pxp_units::{Area, Length, Volume};
use std::collections::BTreeSet;
use std::fmt;

/// Identifies a backend implementation (`"solidcore"`, `"occt"`).
/// Persisted next to every exact B-rep so a file records which kernel
/// produced its geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BackendId(pub &'static str);

/// Opaque handle to a shape owned by a backend. Valid only with the backend
/// that issued it; the exact B-rep is reached through the backend (or, once
/// saved, from the `.pxp` geometry table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ShapeHandle {
    /// Issuing backend.
    pub backend: BackendId,
    /// Backend-local shape number.
    pub id: u64,
}

/// What kind of body a shape is. Sheet and wire bodies are first-class, as
/// in NX (master prompt, Stage 1 interpretation to confirm).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BodyKind {
    /// Closed volume.
    Solid,
    /// Surface body (open faces).
    Sheet,
    /// Curves only.
    Wire,
}

/// Kinds of topological entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TopoKind {
    /// A point of topology.
    Vertex,
    /// A bounded curve between vertices.
    Edge,
    /// A bounded surface region.
    Face,
    /// A connected set of faces.
    Shell,
    /// The whole body.
    Body,
}

/// A backend-local reference to one entity **within one shape**.
///
/// Not persistent: indices change whenever the shape is rebuilt. Anything
/// stored in a document references topology through persistent names built
/// from [`TopoHistory`], never through a `TopoId` (master prompt: "Never store
/// a transient face index").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TopoId {
    /// Entity kind.
    pub kind: TopoKind,
    /// Backend-local index, stable only within one shape.
    pub index: u32,
}

/// Where an output entity came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum HistorySource {
    /// An entity of operand `operand` of this operation (0 = target,
    /// 1 = first tool, ...).
    Input {
        /// Operand index.
        operand: u8,
        /// The input entity.
        entity: TopoId,
    },
    /// A curve of a profile (sketch section) the operation consumed,
    /// identified by the curve's index in the section as given.
    Profile {
        /// Curve index in the input section.
        curve: u32,
    },
    /// A semantic role defined by the operation itself, for entities with no
    /// input (primitive faces: `"block.top"`, `"cylinder.side"`). Roles are
    /// part of the naming contract: never rename one after release.
    Role(&'static str),
}

/// How an output relates to its sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Relation {
    /// A new entity created from the sources (the side face swept from a
    /// profile edge).
    Generated,
    /// The same entity in a modified form (a face trimmed by a boolean).
    Modified,
}

/// One row of a history map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRecord {
    /// The output entity.
    pub output: TopoId,
    /// Generated or modified.
    pub relation: Relation,
    /// Its sources (at least one).
    pub sources: Vec<HistorySource>,
}

/// The history map an operation returns: for every output entity, what it
/// was generated or modified from; and which input entities disappeared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TopoHistory {
    /// One record per output entity (vertices, edges and faces).
    pub records: Vec<HistoryRecord>,
    /// Input entities that no longer exist in the output.
    pub deleted: Vec<(u8, TopoId)>,
}

impl TopoHistory {
    /// Records for `output`.
    pub fn sources_of(&self, output: TopoId) -> impl Iterator<Item = &HistoryRecord> {
        self.records.iter().filter(move |r| r.output == output)
    }

    /// Outputs that came from `source`.
    pub fn outputs_from<'a>(
        &'a self,
        source: &'a HistorySource,
    ) -> impl Iterator<Item = TopoId> + 'a {
        self.records
            .iter()
            .filter(move |r| r.sources.contains(source))
            .map(|r| r.output)
    }

    /// Checks the contract: every vertex, edge and face in `outputs` has at
    /// least one record, and every record names at least one source.
    ///
    /// # Errors
    /// [`KernelError::IncompleteHistory`] listing the entities without
    /// history.
    pub fn check_complete(
        &self,
        outputs: impl IntoIterator<Item = TopoId>,
    ) -> Result<(), KernelError> {
        let covered: BTreeSet<TopoId> = self
            .records
            .iter()
            .filter(|r| !r.sources.is_empty())
            .map(|r| r.output)
            .collect();
        let missing: Vec<TopoId> = outputs
            .into_iter()
            .filter(|t| matches!(t.kind, TopoKind::Vertex | TopoKind::Edge | TopoKind::Face))
            .filter(|t| !covered.contains(t))
            .collect();
        if missing.is_empty() {
            Ok(())
        } else {
            Err(KernelError::IncompleteHistory(missing))
        }
    }
}

/// Categories of non-fatal kernel warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WarningCode {
    /// A tolerance had to be relaxed to complete the operation.
    ToleranceRelaxed,
    /// A tiny edge or sliver face was merged away.
    TinyEntityMerged,
    /// Geometry was healed (gaps closed, edges re-sewn).
    Healed,
    /// Anything else; see the message.
    Other,
}

/// A non-fatal issue. Shown as a warning badge in the Part Navigator; never
/// dropped silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelWarning {
    /// Category.
    pub code: WarningCode,
    /// Human-readable detail.
    pub message: String,
    /// Output entities concerned (may be empty).
    pub entities: Vec<TopoId>,
}

/// Returned by **every** topological operation of **every** backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpResult {
    /// Resulting body: solid, sheet or wire.
    pub shape: ShapeHandle,
    /// Which output faces, edges and vertices were generated from, modified
    /// from, or replaced which inputs. Persistent naming is built on this
    /// map.
    pub history: TopoHistory,
    /// Non-fatal issues.
    pub warnings: Vec<KernelWarning>,
}

/// Errors from kernel operations.
#[derive(Debug, Clone, PartialEq)]
pub enum KernelError {
    /// This backend does not implement the operation (the UI greys out the
    /// command rather than calling it).
    Unsupported {
        /// Operation name.
        op: &'static str,
        /// Why, if known.
        reason: String,
    },
    /// The request is invalid (non-positive size, degenerate axis...).
    InvalidInput(String),
    /// The backend tried and failed (no intersection found, self-
    /// intersecting result...). `entities` points at the trouble.
    GeometryFailure {
        /// Operation name.
        op: &'static str,
        /// Backend's explanation.
        reason: String,
        /// Input entities involved, if known.
        entities: Vec<TopoId>,
    },
    /// The handle does not belong to this backend or was released.
    UnknownShape(ShapeHandle),
    /// A backend violated the history contract.
    IncompleteHistory(Vec<TopoId>),
    /// The backend panicked; the host contained it.
    BackendPanicked(String),
    /// The operation was cancelled.
    Cancelled,
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { op, reason } => {
                write!(f, "{op} is not supported by this kernel: {reason}")
            }
            Self::InvalidInput(m) => write!(f, "invalid input: {m}"),
            Self::GeometryFailure { op, reason, .. } => write!(f, "{op} failed: {reason}"),
            Self::UnknownShape(h) => {
                write!(f, "unknown shape {} from backend {}", h.id, h.backend.0)
            }
            Self::IncompleteHistory(m) => write!(
                f,
                "kernel history contract violated for {} entities",
                m.len()
            ),
            Self::BackendPanicked(m) => write!(f, "geometry kernel crashed and was contained: {m}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for KernelError {}

/// Boolean operation types (NX: Unite, Subtract, Intersect).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BooleanOp {
    /// Union.
    Unite,
    /// Target minus tool.
    Subtract,
    /// Common volume.
    Intersect,
}

/// Mass properties at unit density. Multiply by a material density
/// ([`pxp_units::Density`]) for mass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassProperties {
    /// Enclosed volume (zero for sheets and wires).
    pub volume: Volume,
    /// Total face area.
    pub area: Area,
    /// Centroid (of volume for solids, of area for sheets), mm.
    pub centroid: Point3,
}

fn positive(name: &str, v: Length) -> Result<(), KernelError> {
    if v.is_finite() && v > Length::ZERO {
        Ok(())
    } else {
        Err(KernelError::InvalidInput(format!(
            "{name} must be positive and finite, got {v}"
        )))
    }
}

/// A block (NX: Block, origin and edge lengths), aligned to `csys` with one
/// corner at its origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockSpec {
    /// Placement: corner at the origin, edges along X, Y, Z.
    pub csys: Csys,
    /// Edge length along X.
    pub x: Length,
    /// Edge length along Y.
    pub y: Length,
    /// Edge length along Z.
    pub z: Length,
}

impl BlockSpec {
    /// Face roles, for persistent naming of primitive faces.
    pub const FACE_ROLES: [&'static str; 6] = [
        "block.x_min",
        "block.x_max",
        "block.y_min",
        "block.y_max",
        "block.z_min",
        "block.z_max",
    ];

    /// Checks sizes.
    ///
    /// # Errors
    /// [`KernelError::InvalidInput`] for non-positive or non-finite sizes.
    pub fn validate(&self) -> Result<(), KernelError> {
        positive("block X length", self.x)?;
        positive("block Y length", self.y)?;
        positive("block Z length", self.z)
    }
}

/// A cylinder (NX: Cylinder, axis, diameter and height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CylinderSpec {
    /// Centre of the base circle.
    pub base: Point3,
    /// Axis direction (from base towards top).
    pub axis: UnitVec3,
    /// Diameter.
    pub diameter: Length,
    /// Height along the axis.
    pub height: Length,
}

impl CylinderSpec {
    /// Face roles.
    pub const FACE_ROLES: [&'static str; 3] = ["cylinder.bottom", "cylinder.top", "cylinder.side"];

    /// Checks sizes.
    ///
    /// # Errors
    /// [`KernelError::InvalidInput`] for non-positive sizes or a non-finite
    /// base.
    pub fn validate(&self) -> Result<(), KernelError> {
        if !self.base.is_finite() {
            return Err(KernelError::InvalidInput(
                "cylinder base is not finite".into(),
            ));
        }
        positive("cylinder diameter", self.diameter)?;
        positive("cylinder height", self.height)
    }
}

/// A cone or truncated cone (NX: Cone, base and top diameters and height).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConeSpec {
    /// Centre of the base circle.
    pub base: Point3,
    /// Axis direction.
    pub axis: UnitVec3,
    /// Base diameter.
    pub base_diameter: Length,
    /// Top diameter (zero for a pointed cone).
    pub top_diameter: Length,
    /// Height along the axis.
    pub height: Length,
}

impl ConeSpec {
    /// Face roles (`cone.top` is absent for a pointed cone).
    pub const FACE_ROLES: [&'static str; 3] = ["cone.bottom", "cone.top", "cone.side"];

    /// Checks sizes: positive base and height, non-negative top, and the two
    /// diameters not equal (that would be a cylinder).
    ///
    /// # Errors
    /// [`KernelError::InvalidInput`].
    pub fn validate(&self) -> Result<(), KernelError> {
        if !self.base.is_finite() {
            return Err(KernelError::InvalidInput("cone base is not finite".into()));
        }
        positive("cone base diameter", self.base_diameter)?;
        positive("cone height", self.height)?;
        if !(self.top_diameter.is_finite() && self.top_diameter >= Length::ZERO) {
            return Err(KernelError::InvalidInput(
                "cone top diameter must be >= 0".into(),
            ));
        }
        if self.top_diameter == self.base_diameter {
            return Err(KernelError::InvalidInput(
                "equal diameters: use a cylinder".into(),
            ));
        }
        Ok(())
    }
}

/// A sphere (NX: Sphere, centre and diameter).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SphereSpec {
    /// Centre.
    pub center: Point3,
    /// Diameter.
    pub diameter: Length,
}

impl SphereSpec {
    /// Face roles.
    pub const FACE_ROLES: [&'static str; 1] = ["sphere.surface"];

    /// Checks sizes.
    ///
    /// # Errors
    /// [`KernelError::InvalidInput`].
    pub fn validate(&self) -> Result<(), KernelError> {
        if !self.center.is_finite() {
            return Err(KernelError::InvalidInput(
                "sphere centre is not finite".into(),
            ));
        }
        positive("sphere diameter", self.diameter)
    }
}

/// Which operations a backend implements, so the UI can grey out the rest
/// ("never fake a feature").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Operation names this backend supports (`"block"`, `"boolean"`, ...).
    pub ops: BTreeSet<&'static str>,
}

impl Capabilities {
    /// True if `op` is supported.
    #[must_use]
    pub fn supports(&self, op: &str) -> bool {
        self.ops.contains(op)
    }
}

/// The contract every geometry backend implements.
///
/// Methods take `&self` and the trait requires `Send + Sync` because the
/// recompute scheduler calls the kernel from several worker threads at once
/// (independent features in one level). Backends synchronise their own
/// shape stores.
pub trait GeometryKernel: Send + Sync {
    /// The backend's identity.
    fn backend_id(&self) -> BackendId;

    /// The operations this backend implements.
    fn capabilities(&self) -> Capabilities;

    /// Creates a block.
    ///
    /// # Errors
    /// [`KernelError`] (invalid input, unsupported, geometry failure).
    fn block(&self, spec: &BlockSpec) -> Result<OpResult, KernelError>;

    /// Creates a cylinder.
    ///
    /// # Errors
    /// [`KernelError`].
    fn cylinder(&self, spec: &CylinderSpec) -> Result<OpResult, KernelError>;

    /// Creates a cone.
    ///
    /// # Errors
    /// [`KernelError`].
    fn cone(&self, spec: &ConeSpec) -> Result<OpResult, KernelError>;

    /// Creates a sphere.
    ///
    /// # Errors
    /// [`KernelError`].
    fn sphere(&self, spec: &SphereSpec) -> Result<OpResult, KernelError>;

    /// Boolean of `target` with `tool` (operand 0 and 1 in the history).
    ///
    /// # Errors
    /// [`KernelError`].
    fn boolean(
        &self,
        op: BooleanOp,
        target: ShapeHandle,
        tool: ShapeHandle,
    ) -> Result<OpResult, KernelError>;

    /// Lists a shape's entities (vertices, edges, faces, shells, body).
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`].
    fn topology(&self, shape: ShapeHandle) -> Result<Vec<TopoId>, KernelError>;

    /// Body kind of a shape.
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`].
    fn body_kind(&self, shape: ShapeHandle) -> Result<BodyKind, KernelError>;

    /// Volume, area and centroid.
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`] or a geometry failure.
    fn mass_properties(&self, shape: ShapeHandle) -> Result<MassProperties, KernelError>;

    /// A platform-independent hash of the shape's exact geometry and
    /// topology, with coordinates quantised to `resolution` (see
    /// `pxp_math::StableHasher`). Equal across save/reopen and across OSes.
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`].
    fn geometry_hash(&self, shape: ShapeHandle, resolution: Length) -> Result<u64, KernelError>;

    /// Frees a shape. Releasing an unknown handle is a no-op.
    fn release(&self, shape: ShapeHandle);
}

/// The single entry point applications use for geometry.
///
/// Today it hosts a backend **in-process** and contains backend panics
/// (`catch_unwind`), returning [`KernelError::BackendPanicked`] so the
/// document transaction rolls back. Native backends such as OCCT can crash
/// the process in ways `catch_unwind` cannot stop; for them, sub-stage 0F adds
/// out-of-process hosting with automatic worker restart and transaction
/// replay. Until then no native backend is wired in.
pub struct KernelHost {
    backend: Box<dyn GeometryKernel>,
}

impl fmt::Debug for KernelHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KernelHost")
            .field("backend", &self.backend.backend_id())
            .finish()
    }
}

impl KernelHost {
    /// Hosts a pure-Rust backend in this process.
    #[must_use]
    pub fn in_process(backend: Box<dyn GeometryKernel>) -> Self {
        Self { backend }
    }

    /// The hosted backend's identity.
    #[must_use]
    pub fn backend_id(&self) -> BackendId {
        self.backend.backend_id()
    }

    /// The hosted backend's capabilities.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        self.backend.capabilities()
    }

    /// Runs one kernel call with panic containment.
    ///
    /// # Errors
    /// Whatever the call returns, or [`KernelError::BackendPanicked`].
    pub fn call<T>(
        &self,
        f: impl FnOnce(&dyn GeometryKernel) -> Result<T, KernelError>,
    ) -> Result<T, KernelError> {
        let backend = self.backend.as_ref();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(backend))).unwrap_or_else(|p| {
            Err(KernelError::BackendPanicked(pxp_diag::panic_message(
                p.as_ref(),
            )))
        })
    }

    /// Runs a topological operation and enforces the history contract on
    /// its result before anyone can store it.
    ///
    /// # Errors
    /// As [`KernelHost::call`], plus [`KernelError::IncompleteHistory`] if
    /// the backend's history does not cover every output entity.
    pub fn op(
        &self,
        f: impl FnOnce(&dyn GeometryKernel) -> Result<OpResult, KernelError>,
    ) -> Result<OpResult, KernelError> {
        let result = self.call(f)?;
        let entities = self.call(|k| k.topology(result.shape))?;
        if let Err(e) = result.history.check_complete(entities) {
            self.backend.release(result.shape);
            return Err(e);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
