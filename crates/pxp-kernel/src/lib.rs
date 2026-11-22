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
//!   bake-off: SolidCraft's kernel, built on SolidCore, in-process via
//!   `pxp-kernel-native`; OCCT in a worker process via `pxp-kernel-occt`;
//!   ADR-0004 records the winner).
//! - [`KernelHost`]: the single entry point applications use. In-process
//!   hosting with panic isolation exists now; out-of-process worker hosting
//!   for native (C++) backends arrives with the OCCT half of sub-stage 0F.
//! - The stored-shape envelope ([`encode_stored_shape`]) documents use to
//!   keep a backend's exact B-rep, and [`DEFAULT_HASH_RESOLUTION`].
//! - The backend-neutral kernel bake-off harness and corpus ([`bakeoff`]).
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
///
/// A backend whose kernel reports provenance for faces but not for every
/// edge and vertex may derive the missing records from adjacency: an edge
/// new to the operation from the faces it separates, a vertex from the faces
/// around it. Such a backend says so in its crate documentation
/// (`docs/design/kernel-contract.md`, "Derived records").
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
    /// The operation succeeded but left no material (a subtract whose tool
    /// swallows the target, an intersect of bodies that do not overlap). NX
    /// reports this as an error rather than creating an empty body.
    EmptyResult {
        /// Operation name.
        op: &'static str,
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
            Self::EmptyResult { op } => write!(f, "{op} left no material"),
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

/// Topology counts with adjacent pieces of one surface (or curve) merged:
/// a full cylinder side counts as one face whether a kernel stores it as one
/// periodic face or two halves. Kernels differ in where they split, so these
/// are the counts the bake-off compares across backends (SolidCraft's
/// kernel-case rig counts the same way).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct MergedCounts {
    /// Faces, adjacent faces on one surface merged.
    pub faces: usize,
    /// Edges, edges between merged faces dropped and split edges joined.
    pub edges: usize,
    /// Vertices that remain after merging.
    pub vertices: usize,
}

/// Resolution at which documents store geometry hashes (one nanometre):
/// far below any modeling tolerance, far above floating-point noise.
pub const DEFAULT_HASH_RESOLUTION: Length = Length::from_mm(1e-6);

/// One body a backend's STEP translator read
/// ([`GeometryKernel::import_step`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedShape {
    /// The body, owned by the backend that read it: release it when done.
    pub shape: ShapeHandle,
    /// Name from the file (the solid's label, else its product's); may be
    /// empty.
    pub name: String,
    /// Every face the file gives for this solid was read and the shell
    /// closes. `false` means faces were left out (or the file holds a
    /// surface model): the body is open and must not be graded as a solid.
    pub closed: bool,
    /// A facet (mesh) body, such as AP242 tessellated geometry: no exact
    /// B-rep, so exact operations refuse it.
    pub mesh: bool,
}

/// What a backend's STEP translator made of one file. Nothing it skipped
/// is hidden: every entity it could not read, and every repair it made, is
/// in [`StepImport::warnings`] (pxp-io-step's "must never report success
/// for entities it skipped").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StepImport {
    /// The bodies read, in file order.
    pub shapes: Vec<ImportedShape>,
    /// Distinct solid items the file places.
    pub parts: usize,
    /// Of [`StepImport::parts`], how many came in as closed solids.
    pub closed_parts: usize,
    /// The `FILE_SCHEMA` the file declares (`AP242_…`, `CONFIG_CONTROL_DESIGN`).
    pub schema: String,
    /// What could not be read or had to be repaired, in the translator's
    /// own words.
    pub warnings: Vec<String>,
}

/// Display tolerances for [`GeometryKernel::tessellate`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeshTolerance {
    /// The farthest a facet or edge segment may stand off the exact
    /// surface or curve (chord height).
    pub chord: Length,
    /// The most a facet may turn across its width, in radians; `None`:
    /// the chord height alone.
    pub angle: Option<f64>,
}

impl MeshTolerance {
    /// The viewport's default: a chord of 1/1000 of the body's size (at
    /// least 1 µm) and 15° between facets, roughly what NX displays at its
    /// standard resolution.
    #[must_use]
    pub fn display(body_size: Length) -> Self {
        Self {
            chord: Length::from_mm((body_size.mm() * 1e-3).max(1e-3)),
            angle: Some(15f64.to_radians()),
        }
    }
}

/// A display mesh of a body ([`GeometryKernel::tessellate`]): never the
/// source of truth for geometry, the exact B-rep always is.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FacetMesh {
    /// Vertex positions (mm, model space).
    pub positions: Vec<[f64; 3]>,
    /// Unit normals, one per position (pointing out of the material).
    pub normals: Vec<[f32; 3]>,
    /// Triangles as vertex indices, counter-clockwise seen from outside.
    pub triangles: Vec<[u32; 3]>,
    /// For each triangle, the index of its face among the body's faces in
    /// [`GeometryKernel::topology`] order (faces only, counted from 0).
    pub tri_face: Vec<u32>,
    /// Edge polylines to display. Seams (where one smooth surface meets
    /// itself) are left out.
    pub edges: Vec<Vec<[f64; 3]>>,
}

impl FacetMesh {
    /// Checks the mesh's internal consistency: indices in range, one face
    /// per triangle, one normal per position.
    ///
    /// # Errors
    /// [`KernelError::GeometryFailure`] naming the first inconsistency.
    pub fn validate(&self) -> Result<(), KernelError> {
        let bad = |reason: String| KernelError::GeometryFailure {
            op: "tessellate",
            reason,
            entities: Vec::new(),
        };
        let n = self.positions.len();
        if self.normals.len() != n {
            return Err(bad(format!(
                "{} normals for {n} positions",
                self.normals.len()
            )));
        }
        if self.tri_face.len() != self.triangles.len() {
            return Err(bad(format!(
                "{} face indices for {} triangles",
                self.tri_face.len(),
                self.triangles.len()
            )));
        }
        if let Some(t) = self
            .triangles
            .iter()
            .find(|t| t.iter().any(|&i| i as usize >= n))
        {
            return Err(bad(format!("triangle {t:?} indexes past {n} positions")));
        }
        Ok(())
    }
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

    /// Face, edge and vertex counts with adjacent pieces of one surface
    /// merged ([`MergedCounts`]).
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`]; the default refuses with
    /// [`KernelError::Unsupported`].
    fn merged_counts(&self, shape: ShapeHandle) -> Result<MergedCounts, KernelError> {
        Err(KernelError::Unsupported {
            op: "merged_counts",
            reason: format!("backend cannot merge topology of shape {}", shape.id),
        })
    }

    /// The shape's exact B-rep in this backend's own versioned format, for
    /// storage in a document (wrap it with [`encode_stored_shape`] so the
    /// file records which backend wrote it). Identical shapes give identical
    /// bytes (determinism rule).
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`], or [`KernelError::Unsupported`] for a
    /// backend that cannot serialise.
    fn export_native(&self, shape: ShapeHandle) -> Result<Vec<u8>, KernelError>;

    /// Restores a shape from bytes made by [`GeometryKernel::export_native`]
    /// of the same backend. The restored shape has the same topology order
    /// and geometry hash as the exported one. No history: it is the same
    /// shape, not the result of an operation.
    ///
    /// # Errors
    /// [`KernelError::InvalidInput`] for bytes this backend cannot read.
    fn import_native(&self, bytes: &[u8]) -> Result<ShapeHandle, KernelError>;

    /// The shape's exact axis-aligned bounding box (mm): computed on the
    /// exact curves and surfaces, not on vertices, so curved faces that
    /// bulge past every vertex are inside it.
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`] or a geometry failure; the default
    /// refuses with [`KernelError::Unsupported`].
    fn bounding_box(&self, shape: ShapeHandle) -> Result<pxp_math::Aabb, KernelError> {
        Err(KernelError::Unsupported {
            op: "bounding_box",
            reason: format!("backend cannot bound shape {}", shape.id),
        })
    }

    /// Runs the backend's own validity check on a shape (NX: Examine
    /// Geometry): each test it fails, in its words; empty when the shape
    /// passes. A shape that fails may be refused by later operations, so
    /// an import whose bodies fail it is not ready for modeling even when
    /// every body is closed.
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`], a check that cannot run, or (the
    /// default) [`KernelError::Unsupported`].
    fn check_validity(&self, shape: ShapeHandle) -> Result<Vec<String>, KernelError> {
        Err(KernelError::Unsupported {
            op: "check_validity",
            reason: format!("backend has no validity check (shape {})", shape.id),
        })
    }

    /// Reads a STEP (ISO 10303-21) file's solids with this backend's own
    /// translator (OCCT and SolidCraft each have one). Bodies come back in
    /// millimetres, placed where the file puts them, with no history: an
    /// import starts a part, it is not the result of an operation on one.
    ///
    /// The text is untrusted: a backend must refuse what it cannot read
    /// rather than panic, hang or return a partial body without saying so.
    ///
    /// # Errors
    /// [`KernelError::InvalidInput`] for text that is not readable STEP, a
    /// geometry failure, or (the default) [`KernelError::Unsupported`].
    fn import_step(&self, text: &str) -> Result<StepImport, KernelError> {
        Err(KernelError::Unsupported {
            op: "import_step",
            reason: format!(
                "backend has no STEP translator ({} bytes offered)",
                text.len()
            ),
        })
    }

    /// Writes `shapes` as one STEP file (AP242 geometry, one product per
    /// shape). The header carries no time stamp, so identical shapes give
    /// identical text (determinism rule).
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`], a geometry failure, or (the default)
    /// [`KernelError::Unsupported`].
    fn export_step(&self, shapes: &[ShapeHandle]) -> Result<String, KernelError> {
        Err(KernelError::Unsupported {
            op: "export_step",
            reason: format!(
                "backend has no STEP writer ({} shapes offered)",
                shapes.len()
            ),
        })
    }

    /// A display mesh of the shape at the given tolerances (see
    /// [`FacetMesh`]). Read-only: the shape is unchanged.
    ///
    /// # Errors
    /// [`KernelError::UnknownShape`], a meshing failure, or (the default)
    /// [`KernelError::Unsupported`].
    fn tessellate(
        &self,
        shape: ShapeHandle,
        tolerance: &MeshTolerance,
    ) -> Result<FacetMesh, KernelError> {
        Err(KernelError::Unsupported {
            op: "tessellate",
            reason: format!(
                "backend has no mesher (shape {shape:?}, chord {:?})",
                tolerance.chord
            ),
        })
    }

    /// Frees a shape. Releasing an unknown handle is a no-op.
    fn release(&self, shape: ShapeHandle);
}

/// Magic bytes that open a stored shape.
const STORED_SHAPE_MAGIC: &[u8; 4] = b"PXPB";
/// Version of the stored-shape envelope (not of the backend's own format,
/// which the backend versions itself).
const STORED_SHAPE_VERSION: u8 = 1;

/// An exact B-rep as a document stores it: which backend wrote it and that
/// backend's native bytes (from [`GeometryKernel::export_native`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredShape<'a> {
    /// The writing backend's [`BackendId`] string.
    pub backend: &'a str,
    /// The backend's native bytes.
    pub native: &'a [u8],
}

/// Wraps native B-rep bytes in the stored-shape envelope:
/// `"PXPB"`, version (u8), backend id length (u8), backend id (UTF-8),
/// native length (u64 little-endian), native bytes.
///
/// # Errors
/// [`KernelError::InvalidInput`] if the backend id is longer than 255 bytes.
pub fn encode_stored_shape(backend: BackendId, native: &[u8]) -> Result<Vec<u8>, KernelError> {
    let id = backend.0.as_bytes();
    let id_len = u8::try_from(id.len())
        .map_err(|_| KernelError::InvalidInput("backend id longer than 255 bytes".into()))?;
    let mut out = Vec::with_capacity(4 + 2 + id.len() + 8 + native.len());
    out.extend_from_slice(STORED_SHAPE_MAGIC);
    out.push(STORED_SHAPE_VERSION);
    out.push(id_len);
    out.extend_from_slice(id);
    out.extend_from_slice(&(native.len() as u64).to_le_bytes());
    out.extend_from_slice(native);
    Ok(out)
}

/// Reads the envelope written by [`encode_stored_shape`].
///
/// # Errors
/// [`KernelError::InvalidInput`] for bytes that are not a stored shape, a
/// newer envelope version, or a truncated or over-long buffer.
pub fn decode_stored_shape(bytes: &[u8]) -> Result<StoredShape<'_>, KernelError> {
    let bad = |why: &str| KernelError::InvalidInput(format!("not a stored shape: {why}"));
    let rest = bytes
        .strip_prefix(STORED_SHAPE_MAGIC.as_slice())
        .ok_or_else(|| bad("missing PXPB header"))?;
    let (&version, rest) = rest.split_first().ok_or_else(|| bad("truncated"))?;
    if version != STORED_SHAPE_VERSION {
        return Err(bad(&format!(
            "envelope version {version}, this build reads {STORED_SHAPE_VERSION}"
        )));
    }
    let (&id_len, rest) = rest.split_first().ok_or_else(|| bad("truncated"))?;
    let (id, rest) = rest
        .split_at_checked(usize::from(id_len))
        .ok_or_else(|| bad("truncated backend id"))?;
    let backend = std::str::from_utf8(id).map_err(|_| bad("backend id is not UTF-8"))?;
    let (len, native) = rest
        .split_first_chunk::<8>()
        .ok_or_else(|| bad("truncated length"))?;
    let len = usize::try_from(u64::from_le_bytes(*len)).map_err(|_| bad("length overflows"))?;
    if native.len() != len {
        return Err(bad("length does not match the data"));
    }
    Ok(StoredShape { backend, native })
}

/// The single entry point applications use for geometry.
///
/// Today it hosts a backend **in-process** and contains backend panics
/// (`catch_unwind`), returning [`KernelError::BackendPanicked`] so the
/// document transaction rolls back. Native backends such as OCCT can crash
/// the process in ways `catch_unwind` cannot stop; for them, sub-stage 0F adds
/// out-of-process hosting with automatic worker restart and transaction
/// replay. The pure-Rust backend (`pxp-kernel-native`, SolidCraft's kernel)
/// is safe Rust and runs in-process.
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

pub mod bakeoff;

#[cfg(test)]
mod tests;
