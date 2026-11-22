// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-kernel-native: SolidCraft's pure-Rust B-rep kernel behind `pxp-kernel`
//!
//! [`NativeKernel`] implements `pxp_kernel::GeometryKernel` on
//! [SolidCraft](https://github.com/tangency-labs/solidcraft)'s
//! `solidcraft-kernel`, which is built on SolidCore (a safe-Rust port of
//! NVIDIA SMLib). It is the in-process candidate of the Stage 0 kernel
//! bake-off (ADR-0004); OCCT, run in a worker process, is the other.
//!
//! ## Responsibility
//!
//! - Translate PXP's primitive specifications and booleans into SolidCraft
//!   calls, and SolidCraft errors into [`pxp_kernel::KernelError`].
//! - Own the shapes it creates: a thread-safe store of SolidCraft bodies
//!   behind opaque [`pxp_kernel::ShapeHandle`]s.
//! - Return a complete [`pxp_kernel::TopoHistory`] for every operation. Face
//!   provenance is SolidCraft's own; edge and vertex records it does not
//!   report are derived from exact adjacency (the `history` module documents
//!   the rules; `docs/design/kernel-contract.md`, "Derived records").
//! - Hash geometry deterministically (the `hash` module) and serialise exact
//!   B-reps in SolidCraft's versioned native body format for `.pxp` storage.
//!
//! ## Data flow
//!
//! ```text
//! KernelHost::op ─► NativeKernel::block/…/boolean ─► solidcraft_kernel ─► Body
//!                                     │                                  │
//!                                     └── history::primitive/operation ◄─┘
//!                                                │
//!                         OpResult { shape (store id), history, warnings }
//! ```
//!
//! ## Primitive construction
//!
//! | PXP primitive | SolidCraft call | Face roles from |
//! | --- | --- | --- |
//! | Block | `box_solid` at the origin, then `transform_matrix` into the block's CSYS | the faces' `FaceSource::Box { axis, high }` |
//! | Cylinder | `cylinder(base, axis, radius, height)` | history: profile segment 0 = side, end cap = top, start cap = bottom |
//! | Cone | `revolve` of the right trapezoid (or triangle) `(0,0) (r₁,0) (r₂,h) (0,h)` about the sketch's y axis | history: profile segment 0 = bottom, 1 = side, 2 = top |
//! | Sphere | `sphere(center, radius)` | history: profile segment 0 = surface |
//!
//! SolidCraft has no cone primitive, so the cone is a revolve. The
//! transform used for blocks keeps the body's face and edge order (SolidCraft
//! guarantees this; the adapter checks it on every call).
//!
//! ## Invariants
//!
//! - Every topological operation returns a history that passes
//!   `TopoHistory::check_complete` (tested on every primitive and boolean).
//! - Identical inputs give identical topology order, history, native bytes
//!   and geometry hash.
//! - [`NativeKernel::live_shapes`] counts exactly the handles not yet
//!   released.
//!
//! ## Must never
//!
//! - Be called directly by application crates (only shell crates wire
//!   backends; `tools/check_repo.py` enforces it).
//! - Return plausible-looking geometry for an operation it cannot perform:
//!   it returns a typed refusal instead.
//!
//! ## Licence note
//!
//! The SolidCraft crates are MIT OR Apache-2.0 (taken under MIT). The
//! SolidCore crates they use are Apache-2.0 only, so binaries that link this
//! crate are conveyed under LGPL-3.0 (ADR-0001). The dependency is pinned to
//! [`KERNEL_REVISION`].
#![forbid(unsafe_code)]

mod hash;
mod history;

use pxp_kernel::{
    BackendId, BlockSpec, BodyKind, BooleanOp, Capabilities, ConeSpec, CylinderSpec, FacetMesh,
    GeometryKernel, ImportedShape, KernelError, MassProperties, MergedCounts, MeshTolerance,
    OpResult, ShapeHandle, SphereSpec, StepImport, TopoHistory, TopoId, TopoKind,
};
use pxp_math::{Aabb, Csys, Point3, UnitVec3};
use pxp_units::{Area, Length, Volume};
use solidcore_record::brep::{Brep, Shell};
use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};
use solidcraft_kernel::history::FaceOrigin;
use solidcraft_kernel::{Body, BoolOp, FaceSource};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status`.
pub const PLANNED_STAGE: u8 = 0;

/// This backend's identity, persisted next to every exact B-rep it writes.
pub const BACKEND: BackendId = BackendId("solidcraft");

/// The SolidCraft commit this adapter is built and validated against (the
/// `rev` of the git dependency in the workspace `Cargo.toml`).
pub const KERNEL_REVISION: &str = "2091b8828c328d5ab0a0f162dbc7a36a558aa99d";

/// Operation names reported in [`Capabilities`].
const OPS: [&str; 17] = [
    "block",
    "cylinder",
    "cone",
    "sphere",
    "boolean",
    "topology",
    "body_kind",
    "mass_properties",
    "geometry_hash",
    "merged_counts",
    "export_native",
    "import_native",
    "bounding_box",
    "import_step",
    "export_step",
    "tessellate",
    "check_validity",
];

/// SolidCraft behind the `GeometryKernel` trait. Thread-safe: the recompute
/// scheduler calls it from several workers at once; the shape store is
/// locked only to insert, look up or remove a body, never during geometry
/// work.
pub struct NativeKernel {
    shapes: Mutex<BTreeMap<u64, Arc<Body>>>,
    next: AtomicU64,
}

impl Default for NativeKernel {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for NativeKernel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeKernel")
            .field("backend", &BACKEND.0)
            .field("live_shapes", &self.live_shapes())
            .finish()
    }
}

/// Converts a SolidCraft error. Invalid input stays invalid input; anything
/// else is a geometry failure of `op` carrying SolidCraft's explanation.
fn kernel_err(op: &'static str) -> impl Fn(solidcraft_kernel::KernelError) -> KernelError {
    move |e| match e {
        solidcraft_kernel::KernelError::Invalid(m) => KernelError::InvalidInput(m),
        other => KernelError::GeometryFailure {
            op,
            reason: other.to_string(),
            entities: Vec::new(),
        },
    }
}

fn point(p: Point3) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

fn direction(u: UnitVec3) -> Vec3 {
    let v = u.get();
    Vec3::new(v.x, v.y, v.z)
}

/// The column-major 4×4 matrix SolidCraft's `transform_matrix` takes, for a
/// CSYS's local-to-absolute transform.
fn column_major(csys: &Csys) -> [[f64; 4]; 4] {
    let t = csys.to_absolute();
    let (cx, cy, cz, d) = (t.rotation.cx, t.rotation.cy, t.rotation.cz, t.translation);
    [
        [cx.x, cx.y, cx.z, 0.0],
        [cy.x, cy.y, cy.z, 0.0],
        [cz.x, cz.y, cz.z, 0.0],
        [d.x, d.y, d.z, 1.0],
    ]
}

/// A unit vector perpendicular to `axis`, chosen by a fixed rule (project
/// the coordinate axis least aligned with `axis`) so the same cone always
/// gets the same seam.
fn perpendicular(axis: UnitVec3) -> Vec3 {
    let along = axis.get();
    let seed = if along.x.abs() < 0.9 {
        pxp_math::Vec3::X
    } else {
        pxp_math::Vec3::Y
    };
    let dot = seed.dot(along);
    let projected = pxp_math::Vec3::new(
        seed.x - dot * along.x,
        seed.y - dot * along.y,
        seed.z - dot * along.z,
    );
    let len = projected.length();
    Vec3::new(projected.x / len, projected.y / len, projected.z / len)
}

fn unknown(shape: ShapeHandle) -> KernelError {
    KernelError::UnknownShape(shape)
}

/// The body's B-rep record, refusing facet (mesh) bodies, which no PXP
/// operation creates yet.
fn record<'a>(op: &'static str, body: &'a Body) -> Result<&'a Brep, KernelError> {
    if body.is_mesh() {
        return Err(KernelError::Unsupported {
            op,
            reason: "facet (mesh) bodies are not supported yet".into(),
        });
    }
    Ok(body.record())
}

impl NativeKernel {
    /// An empty kernel.
    #[must_use]
    pub fn new() -> Self {
        Self {
            shapes: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
        }
    }

    /// Number of shapes created or imported and not yet released.
    #[must_use]
    pub fn live_shapes(&self) -> usize {
        self.lock().len()
    }

    /// The store never holds a half-updated map (each access is a single
    /// insert, lookup or remove), so a lock poisoned by a panicking thread
    /// is still safe to use.
    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<u64, Arc<Body>>> {
        self.shapes.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn store(&self, body: Body) -> ShapeHandle {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.lock().insert(id, Arc::new(body));
        ShapeHandle {
            backend: BACKEND,
            id,
        }
    }

    fn get(&self, shape: ShapeHandle) -> Result<Arc<Body>, KernelError> {
        if shape.backend != BACKEND {
            return Err(unknown(shape));
        }
        self.lock()
            .get(&shape.id)
            .cloned()
            .ok_or_else(|| unknown(shape))
    }

    fn finish(&self, body: Body, history: TopoHistory) -> OpResult {
        OpResult {
            shape: self.store(body),
            history,
            warnings: Vec::new(),
        }
    }
}

/// Block face roles from SolidCraft's face sources (`axis` 0..2 = X..Z,
/// `high` = the max side), in the order of `BlockSpec::FACE_ROLES`.
fn block_roles(body: &Body) -> Result<Vec<&'static str>, KernelError> {
    let missing = || KernelError::GeometryFailure {
        op: "block",
        reason: "SolidCraft did not report which side each box face is".into(),
        entities: Vec::new(),
    };
    body.face_sources()
        .ok_or_else(missing)?
        .iter()
        .map(|s| match s {
            Some(FaceSource::Box { axis, high, .. }) => BlockSpec::FACE_ROLES
                .get(usize::from(*axis) * 2 + usize::from(*high))
                .copied()
                .ok_or_else(missing),
            _ => Err(missing()),
        })
        .collect()
}

/// Checks that a transformed body kept its source's face order (each face
/// `i` modified from face `i` of operand 0).
fn kept_face_order(body: &Body) -> bool {
    body.history().is_some_and(|h| {
        h.faces.iter().enumerate().all(|(i, f)| {
            f.origins.as_slice()
                == [FaceOrigin::Face {
                    operand: 0,
                    face: i,
                }]
        })
    })
}

impl GeometryKernel for NativeKernel {
    fn backend_id(&self) -> BackendId {
        BACKEND
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            ops: OPS.into_iter().collect(),
        }
    }

    fn block(&self, spec: &BlockSpec) -> Result<OpResult, KernelError> {
        spec.validate()?;
        let local = solidcraft_kernel::box_solid(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(spec.x.mm(), spec.y.mm(), spec.z.mm()),
        )
        .map_err(kernel_err("block"))?;
        let roles = block_roles(&local)?;
        let body = if spec.csys == Csys::ABSOLUTE {
            local
        } else {
            let placed = solidcraft_kernel::transform_matrix(&local, column_major(&spec.csys))
                .map_err(kernel_err("block"))?;
            if !kept_face_order(&placed) {
                return Err(KernelError::GeometryFailure {
                    op: "block",
                    reason: "placing the block reordered its faces".into(),
                    entities: Vec::new(),
                });
            }
            placed
        };
        let history = history::primitive(&body, &roles)?;
        Ok(self.finish(body, history))
    }

    fn cylinder(&self, spec: &CylinderSpec) -> Result<OpResult, KernelError> {
        spec.validate()?;
        let body = solidcraft_kernel::cylinder(
            point(spec.base),
            direction(spec.axis),
            spec.diameter.mm() / 2.0,
            spec.height.mm(),
        )
        .map_err(kernel_err("cylinder"))?;
        let [bottom, top, side] = CylinderSpec::FACE_ROLES;
        let roles = history::roles_from_history(&body, |o| match o {
            FaceOrigin::Profile { segment: 0 } => Some(side),
            FaceOrigin::Cap { end: true } => Some(top),
            FaceOrigin::Cap { end: false } => Some(bottom),
            _ => None,
        })?;
        let history = history::primitive(&body, &roles)?;
        Ok(self.finish(body, history))
    }

    fn cone(&self, spec: &ConeSpec) -> Result<OpResult, KernelError> {
        spec.validate()?;
        let (r1, r2, h) = (
            spec.base_diameter.mm() / 2.0,
            spec.top_diameter.mm() / 2.0,
            spec.height.mm(),
        );
        // Sketch plane through the axis: x radial, y along the axis.
        let plane = Plane {
            origin: point(spec.base),
            x: perpendicular(spec.axis),
            y: direction(spec.axis),
        };
        let mut corners = vec![Vec2::new(0.0, 0.0), Vec2::new(r1, 0.0)];
        if r2 > 0.0 {
            corners.push(Vec2::new(r2, h));
        }
        corners.push(Vec2::new(0.0, h));
        let segs: Vec<Seg2> = corners
            .iter()
            .zip(corners.iter().cycle().skip(1))
            .map(|(&a, &b)| Seg2::Line { a, b })
            .collect();
        let region = Region2 {
            outer: Loop2 { segs },
            holes: Vec::new(),
        };
        let mut bodies = solidcraft_kernel::revolve(
            &plane,
            &[region],
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 1.0),
            std::f64::consts::TAU,
        )
        .map_err(kernel_err("cone"))?;
        let (Some(body), true) = (bodies.pop(), bodies.is_empty()) else {
            return Err(KernelError::GeometryFailure {
                op: "cone",
                reason: "revolving the cone profile did not give exactly one body".into(),
                entities: Vec::new(),
            });
        };
        let [bottom, top, side] = ConeSpec::FACE_ROLES;
        let roles = history::roles_from_history(&body, |o| match o {
            FaceOrigin::Profile { segment: 0 } => Some(bottom),
            FaceOrigin::Profile { segment: 1 } => Some(side),
            FaceOrigin::Profile { segment: 2 } if r2 > 0.0 => Some(top),
            _ => None,
        })?;
        let history = history::primitive(&body, &roles)?;
        Ok(self.finish(body, history))
    }

    fn sphere(&self, spec: &SphereSpec) -> Result<OpResult, KernelError> {
        spec.validate()?;
        let body = solidcraft_kernel::sphere(point(spec.center), spec.diameter.mm() / 2.0)
            .map_err(kernel_err("sphere"))?;
        let [surface] = SphereSpec::FACE_ROLES;
        let roles = history::roles_from_history(&body, |o| match o {
            FaceOrigin::Profile { segment: 0 } => Some(surface),
            _ => None,
        })?;
        let history = history::primitive(&body, &roles)?;
        Ok(self.finish(body, history))
    }

    fn boolean(
        &self,
        op: BooleanOp,
        target: ShapeHandle,
        tool: ShapeHandle,
    ) -> Result<OpResult, KernelError> {
        let a = self.get(target)?;
        let b = self.get(tool)?;
        let kop = match op {
            BooleanOp::Unite => BoolOp::Union,
            BooleanOp::Subtract => BoolOp::Cut,
            BooleanOp::Intersect => BoolOp::Intersect,
        };
        let result = solidcraft_kernel::boolean(&a, &b, kop)
            .map_err(kernel_err("boolean"))?
            .ok_or(KernelError::EmptyResult { op: "boolean" })?;
        if result.face_count() == 0 {
            return Err(KernelError::EmptyResult { op: "boolean" });
        }
        let history = history::operation(&result, &[a.as_ref(), b.as_ref()])?;
        Ok(self.finish(result, history))
    }

    fn topology(&self, shape: ShapeHandle) -> Result<Vec<TopoId>, KernelError> {
        let body = self.get(shape)?;
        let rec = record("topology", &body)?;
        let mut out = Vec::with_capacity(
            rec.vertices.len() + rec.edges.len() + rec.faces.len() + rec.shells.len() + 1,
        );
        for (kind, n) in [
            (TopoKind::Vertex, rec.vertices.len()),
            (TopoKind::Edge, rec.edges.len()),
            (TopoKind::Face, rec.faces.len()),
            (TopoKind::Shell, rec.shells.len()),
            (TopoKind::Body, 1),
        ] {
            for i in 0..n {
                out.push(history::topo(kind, i)?);
            }
        }
        Ok(out)
    }

    fn body_kind(&self, shape: ShapeHandle) -> Result<BodyKind, KernelError> {
        let body = self.get(shape)?;
        let rec = record("body_kind", &body)?;
        Ok(if !rec.material_regions().is_empty() {
            BodyKind::Solid
        } else if rec.shells.iter().any(|s| matches!(s, Shell::Faces { .. })) {
            BodyKind::Sheet
        } else {
            BodyKind::Wire
        })
    }

    fn mass_properties(&self, shape: ShapeHandle) -> Result<MassProperties, KernelError> {
        let body = self.get(shape)?;
        let m = solidcraft_kernel::measure_mass(&body).map_err(kernel_err("mass_properties"))?;
        Ok(MassProperties {
            volume: Volume::from_mm3(m.volume),
            area: Area::from_mm2(m.area),
            centroid: Point3::new(m.centroid.x, m.centroid.y, m.centroid.z),
        })
    }

    fn geometry_hash(&self, shape: ShapeHandle, resolution: Length) -> Result<u64, KernelError> {
        let body = self.get(shape)?;
        record("geometry_hash", &body)?;
        hash::geometry_hash(&body, resolution.mm())
    }

    fn merged_counts(&self, shape: ShapeHandle) -> Result<MergedCounts, KernelError> {
        let body = self.get(shape)?;
        record("merged_counts", &body)?;
        let t = solidcraft_kernel::merged_topology(&body).map_err(kernel_err("merged_counts"))?;
        Ok(MergedCounts {
            faces: t.faces,
            edges: t.edges,
            vertices: t.vertices,
        })
    }

    fn export_native(&self, shape: ShapeHandle) -> Result<Vec<u8>, KernelError> {
        let body = self.get(shape)?;
        solidcraft_kernel::encode_native_body(&body).map_err(kernel_err("export_native"))
    }

    fn import_native(&self, bytes: &[u8]) -> Result<ShapeHandle, KernelError> {
        let body = solidcraft_kernel::decode_native_body(bytes).map_err(|e| {
            KernelError::InvalidInput(format!("not a readable SolidCraft native body: {e}"))
        })?;
        Ok(self.store(body))
    }

    fn bounding_box(&self, shape: ShapeHandle) -> Result<Aabb, KernelError> {
        let body = self.get(shape)?;
        // SolidCraft's exact box (SMLib's, on the surfaces) for a B-rep; a
        // mesh body's box is its triangles' own, which is exact for it.
        let m = solidcraft_kernel::measure_mass(&body).map_err(kernel_err("bounding_box"))?;
        let (lo, hi) = (m.bbox.min, m.bbox.max);
        Ok(Aabb::from_points([
            Point3::new(lo.x, lo.y, lo.z),
            Point3::new(hi.x, hi.y, hi.z),
        ]))
    }

    /// SMLib's full check (`solidcraft_kernel::failing_check`, every test
    /// level) on a copy of the body, as an operation on it would run it.
    /// A mesh body is not SMLib's to check and passes.
    fn check_validity(&self, shape: ShapeHandle) -> Result<Vec<String>, KernelError> {
        let body = self.get(shape)?;
        let failure =
            solidcraft_kernel::failing_check(&body).map_err(kernel_err("check_validity"))?;
        Ok(failure.map_or_else(Vec::new, |f| {
            f.tests
                .iter()
                .map(|t| format!("{} (×{})", t.name, t.times))
                .collect()
        }))
    }

    /// SolidCraft's own STEP reader (`solidcraft_kernel::step_import`,
    /// written from the public standards): the uncached path, so reading
    /// the same text twice really reads it twice. It waits for SMLib's
    /// check of every part; the check's findings and the reader's repairs
    /// are in the warnings.
    fn import_step(&self, text: &str) -> Result<StepImport, KernelError> {
        let imp = solidcraft_kernel::step_import(text).map_err(kernel_err("import_step"))?;
        let report = imp.report.current();
        let (parts, closed_parts) = (report.parts, report.solids);
        let schema = imp.schema.clone();
        let warnings = imp.warnings.clone();
        let shapes = imp
            .bodies
            .into_iter()
            .map(|b| {
                let mesh = b.body.is_mesh();
                ImportedShape {
                    name: b.name,
                    closed: b.closed && b.refused.is_none(),
                    mesh,
                    shape: self.store(b.body),
                }
            })
            .collect();
        Ok(StepImport {
            shapes,
            parts,
            closed_parts,
            schema,
            warnings,
        })
    }

    /// SolidCraft's STEP writer, one product per shape, named `body-<n>`,
    /// with an empty time stamp so the text depends on the geometry alone.
    fn export_step(&self, shapes: &[ShapeHandle]) -> Result<String, KernelError> {
        let bodies = shapes
            .iter()
            .map(|s| self.get(*s))
            .collect::<Result<Vec<_>, _>>()?;
        let export: Vec<solidcraft_kernel::ExportBody<'_>> = bodies
            .iter()
            .enumerate()
            .map(|(i, b)| solidcraft_kernel::ExportBody {
                name: format!("body-{}", i + 1),
                body: b.as_ref(),
                color: None,
            })
            .collect();
        let header = solidcraft_kernel::StepHeader {
            file_name: "pxp-export.stp".into(),
            description: "PXP Engineering Suite".into(),
            ..solidcraft_kernel::StepHeader::default()
        };
        solidcraft_kernel::step_export_bodies(&export, &header).map_err(kernel_err("export_step"))
    }

    /// SolidCraft's mesher (`Body::mesh`): triangles carry their face
    /// index in record order, which is `topology()`'s face order, and
    /// neighbouring faces share boundary nodes, so the mesh is watertight
    /// wherever the B-rep is. Seam polylines are left out of the edges.
    fn tessellate(
        &self,
        shape: ShapeHandle,
        tolerance: &MeshTolerance,
    ) -> Result<FacetMesh, KernelError> {
        let body = self.get(shape)?;
        let mesh = body
            .mesh(&solidcraft_kernel::MeshTolerance {
                chord: tolerance.chord.mm(),
                angle: tolerance.angle,
            })
            .map_err(kernel_err("tessellate"))?;
        #[allow(clippy::cast_possible_truncation)] // display normals: f32 is plenty
        let normals = mesh
            .normals
            .iter()
            .map(|n| [n.x as f32, n.y as f32, n.z as f32])
            .collect();
        let edges = mesh
            .edges
            .iter()
            .enumerate()
            .filter(|(i, _)| !mesh.seams.get(*i).copied().unwrap_or(false))
            .map(|(_, line)| line.iter().map(|p| [p.x, p.y, p.z]).collect())
            .collect();
        let out = FacetMesh {
            positions: mesh.positions.iter().map(|p| [p.x, p.y, p.z]).collect(),
            normals,
            triangles: mesh.triangles.clone(),
            tri_face: mesh.tri_face.clone(),
            edges,
        };
        out.validate()?;
        Ok(out)
    }

    fn release(&self, shape: ShapeHandle) {
        if shape.backend == BACKEND {
            self.lock().remove(&shape.id);
        }
    }
}
