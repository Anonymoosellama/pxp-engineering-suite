// SPDX-License-Identifier: LGPL-2.1-or-later
//! OCCT backend conformance: every operation through `KernelHost::op`
//! (complete histories), measured values against hand calculations, and the
//! worker's failure behaviour (a crash costs one call; stale handles are
//! refused).
//!
//! These tests need the OCCT worker (`PXP_OCCT_PYTHON` pointing at a Python
//! with `cadquery-ocp` installed). Without it each test says so and passes
//! without checking anything, so per-push CI, which does not install OCCT,
//! stays quick; the nightly workflow installs it and runs them for real.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::panic,
    clippy::many_single_char_names
)]

use pxp_kernel::{
    BlockSpec, BodyKind, BooleanOp, CylinderSpec, GeometryKernel, HistorySource, KernelError,
    KernelHost, Relation, SphereSpec, TopoKind,
};
use pxp_kernel_occt::{OcctConfig, OcctKernel};
use pxp_math::{Csys, Point3, UnitVec3};
use pxp_units::Length;
use std::f64::consts::PI;
use std::sync::Arc;

const RES: Length = Length::from_mm(1e-6);

fn mm(v: f64) -> Length {
    Length::from_mm(v)
}

/// The kernel, if its worker starts; `None` (with a note) otherwise.
fn kernel() -> Option<Arc<OcctKernel>> {
    let k = OcctKernel::new(OcctConfig::from_env());
    match k.connect() {
        Ok(_) => Some(Arc::new(k)),
        Err(e) => {
            eprintln!("skipping OCCT conformance test: {e}");
            None
        }
    }
}

struct Shared(Arc<OcctKernel>);

impl GeometryKernel for Shared {
    fn backend_id(&self) -> pxp_kernel::BackendId {
        self.0.backend_id()
    }
    fn capabilities(&self) -> pxp_kernel::Capabilities {
        self.0.capabilities()
    }
    fn block(&self, s: &BlockSpec) -> Result<pxp_kernel::OpResult, KernelError> {
        self.0.block(s)
    }
    fn cylinder(&self, s: &CylinderSpec) -> Result<pxp_kernel::OpResult, KernelError> {
        self.0.cylinder(s)
    }
    fn cone(&self, s: &pxp_kernel::ConeSpec) -> Result<pxp_kernel::OpResult, KernelError> {
        self.0.cone(s)
    }
    fn sphere(&self, s: &SphereSpec) -> Result<pxp_kernel::OpResult, KernelError> {
        self.0.sphere(s)
    }
    fn boolean(
        &self,
        op: BooleanOp,
        a: pxp_kernel::ShapeHandle,
        b: pxp_kernel::ShapeHandle,
    ) -> Result<pxp_kernel::OpResult, KernelError> {
        self.0.boolean(op, a, b)
    }
    fn topology(&self, s: pxp_kernel::ShapeHandle) -> Result<Vec<pxp_kernel::TopoId>, KernelError> {
        self.0.topology(s)
    }
    fn body_kind(&self, s: pxp_kernel::ShapeHandle) -> Result<BodyKind, KernelError> {
        self.0.body_kind(s)
    }
    fn mass_properties(
        &self,
        s: pxp_kernel::ShapeHandle,
    ) -> Result<pxp_kernel::MassProperties, KernelError> {
        self.0.mass_properties(s)
    }
    fn geometry_hash(&self, s: pxp_kernel::ShapeHandle, r: Length) -> Result<u64, KernelError> {
        self.0.geometry_hash(s, r)
    }
    fn export_native(&self, s: pxp_kernel::ShapeHandle) -> Result<Vec<u8>, KernelError> {
        self.0.export_native(s)
    }
    fn import_native(&self, b: &[u8]) -> Result<pxp_kernel::ShapeHandle, KernelError> {
        self.0.import_native(b)
    }
    fn tessellate(
        &self,
        s: pxp_kernel::ShapeHandle,
        t: &pxp_kernel::MeshTolerance,
    ) -> Result<pxp_kernel::FacetMesh, KernelError> {
        self.0.tessellate(s, t)
    }
    fn release(&self, s: pxp_kernel::ShapeHandle) {
        self.0.release(s);
    }
}

fn host(k: &Arc<OcctKernel>) -> KernelHost {
    KernelHost::in_process(Box::new(Shared(Arc::clone(k))))
}

fn close(actual: f64, expected: f64, rel: f64) {
    let err = (actual - expected).abs() / expected.abs().max(1.0);
    assert!(err <= rel, "{actual} vs {expected} (rel {err:e})");
}

fn block(x: f64, y: f64, z: f64) -> BlockSpec {
    BlockSpec {
        csys: Csys::ABSOLUTE,
        x: mm(x),
        y: mm(y),
        z: mm(z),
    }
}

fn hole() -> CylinderSpec {
    CylinderSpec {
        base: Point3::new(10.0, 10.0, -1.0),
        axis: UnitVec3::Z,
        diameter: mm(8.0),
        height: mm(12.0),
    }
}

#[test]
fn primitives_have_roles_and_hand_calculated_measures() {
    let Some(k) = kernel() else { return };
    let h = host(&k);
    let b = h.op(|k| k.block(&block(1.0, 2.0, 3.0))).unwrap();
    let m = h.call(|k| k.mass_properties(b.shape)).unwrap();
    close(m.volume.mm3(), 6.0, 1e-12);
    close(m.area.mm2(), 22.0, 1e-12);
    let roles: Vec<_> = b
        .history
        .records
        .iter()
        .filter(|r| r.output.kind == TopoKind::Face)
        .flat_map(|r| r.sources.clone())
        .collect();
    for role in BlockSpec::FACE_ROLES {
        assert!(roles.contains(&HistorySource::Role(role)), "missing {role}");
    }
    let s = h
        .op(|k| {
            k.sphere(&SphereSpec {
                center: Point3::new(1.0, 2.0, 3.0),
                diameter: mm(10.0),
            })
        })
        .unwrap();
    let ms = h.call(|k| k.mass_properties(s.shape)).unwrap();
    close(ms.volume.mm3(), 4.0 / 3.0 * PI * 125.0, 1e-9);
    close(ms.area.mm2(), 100.0 * PI, 1e-9);
}

#[test]
fn subtract_reports_modified_faces_and_exact_volume() {
    let Some(k) = kernel() else { return };
    let h = host(&k);
    let b = h.op(|k| k.block(&block(20.0, 20.0, 10.0))).unwrap();
    let c = h.op(|k| k.cylinder(&hole())).unwrap();
    let r = h
        .op(|k| k.boolean(BooleanOp::Subtract, b.shape, c.shape))
        .unwrap();
    let m = h.call(|k| k.mass_properties(r.shape)).unwrap();
    close(m.volume.mm3(), 4000.0 - 160.0 * PI, 1e-12);
    close(m.area.mm2(), 1600.0 + 48.0 * PI, 1e-12);
    assert_eq!(k.merged_counts(r.shape).unwrap().faces, 7);
    // The top face is the block's, modified (it gained a hole).
    assert!(r.history.records.iter().any(|rec| {
        rec.output.kind == TopoKind::Face
            && rec.relation == Relation::Modified
            && rec
                .sources
                .iter()
                .any(|s| matches!(s, HistorySource::Input { operand: 0, .. }))
    }));
    // The tool's end caps are gone.
    assert!(
        r.history
            .deleted
            .iter()
            .any(|(op, t)| *op == 1 && t.kind == TopoKind::Face)
    );
    assert!(k.check_validity(r.shape).unwrap().is_empty());
}

#[test]
fn empty_and_invalid_are_refusals() {
    let Some(k) = kernel() else { return };
    let h = host(&k);
    let a = h.op(|k| k.block(&block(1.0, 1.0, 1.0))).unwrap();
    let far = BlockSpec {
        csys: Csys::new(
            Point3::new(10.0, 0.0, 0.0),
            pxp_math::Vec3::X,
            pxp_math::Vec3::Y,
        )
        .unwrap(),
        ..block(1.0, 1.0, 1.0)
    };
    let b = h.op(|k| k.block(&far)).unwrap();
    assert!(matches!(
        h.op(|k| k.boolean(BooleanOp::Intersect, a.shape, b.shape)),
        Err(KernelError::EmptyResult { .. })
    ));
    assert!(matches!(
        h.op(|k| k.block(&block(0.0, 1.0, 1.0))),
        Err(KernelError::InvalidInput(_))
    ));
}

#[test]
fn hashes_round_trips_and_step_are_stable() {
    let Some(k) = kernel() else { return };
    let h = host(&k);
    let b = h.op(|k| k.block(&block(20.0, 20.0, 10.0))).unwrap();
    let c = h.op(|k| k.cylinder(&hole())).unwrap();
    let r = h
        .op(|k| k.boolean(BooleanOp::Subtract, b.shape, c.shape))
        .unwrap();
    let h1 = k.geometry_hash(r.shape, RES).unwrap();
    assert_eq!(h1, k.geometry_hash(r.shape, RES).unwrap());
    let bytes = k.export_native(r.shape).unwrap();
    let back = k.import_native(&bytes).unwrap();
    assert_eq!(h1, k.geometry_hash(back, RES).unwrap());
    let text = k.export_step(&[r.shape]).unwrap();
    assert_eq!(
        text,
        k.export_step(&[r.shape]).unwrap(),
        "STEP text is deterministic"
    );
    let imp = k.import_step(&text).unwrap();
    assert_eq!(imp.shapes.len(), 1);
    assert!(imp.shapes[0].closed);
    let m = k.mass_properties(imp.shapes[0].shape).unwrap();
    close(m.volume.mm3(), 4000.0 - 160.0 * PI, 1e-9);
}

#[test]
fn a_crash_costs_one_call_and_stale_handles_are_refused() {
    let Some(k) = kernel() else { return };
    let before = k.block(&block(1.0, 1.0, 1.0)).unwrap().shape;
    let generation = k.generation();
    assert!(matches!(
        k.abort_worker_for_test(),
        KernelError::BackendPanicked(_)
    ));
    // The next call starts a new worker and succeeds.
    let after = k.block(&block(2.0, 2.0, 2.0)).unwrap().shape;
    assert_eq!(k.generation(), generation + 1);
    close(k.mass_properties(after).unwrap().volume.mm3(), 8.0, 1e-12);
    // The shape made by the dead worker is unknown, not some other shape.
    assert!(matches!(
        k.mass_properties(before),
        Err(KernelError::UnknownShape(_))
    ));
}

/// Signed volume enclosed by a closed triangle mesh (divergence theorem).
fn mesh_volume(m: &pxp_kernel::FacetMesh) -> f64 {
    m.triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| m.positions[i as usize]);
            (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]))
                / 6.0
        })
        .sum()
}

#[test]
fn display_meshes_are_closed_oriented_and_tag_every_face() {
    let Some(k) = kernel() else { return };
    let h = host(&k);
    let b = h.op(|k| k.block(&block(20.0, 30.0, 10.0))).unwrap().shape;
    let tol = pxp_kernel::MeshTolerance::display(mm(40.0));
    let m = h.call(|k| k.tessellate(b, &tol)).unwrap();
    m.validate().unwrap();
    // Outward-facing triangles enclose +6000 mm³ (a reversed face would
    // subtract its share).
    close(mesh_volume(&m), 6000.0, 1e-12);
    let faces: std::collections::BTreeSet<u32> = m.tri_face.iter().copied().collect();
    assert_eq!(faces, (0..6).collect());
    assert_eq!(m.edges.len(), 12);
    let c = h.op(|k| k.cylinder(&hole())).unwrap().shape;
    let m = h.call(|k| k.tessellate(c, &tol)).unwrap();
    assert_eq!(m.edges.len(), 2, "seam hidden");
    let v = h.call(|k| k.mass_properties(c)).unwrap().volume.mm3();
    let mv = mesh_volume(&m);
    assert!(mv < v && mv > 0.99 * v, "faceted {mv} vs exact {v}");
}
