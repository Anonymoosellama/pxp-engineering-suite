// SPDX-License-Identifier: LGPL-2.1-or-later
//! Backend conformance tests: every operation goes through `KernelHost::op`,
//! which rejects incomplete histories, and every measured value is compared
//! with a hand calculation (never with another kernel's output).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::panic,
    clippy::many_single_char_names
)]

use pxp_kernel::{
    BlockSpec, BodyKind, BooleanOp, ConeSpec, CylinderSpec, GeometryKernel, HistorySource,
    KernelError, KernelHost, OpResult, Relation, ShapeHandle, SphereSpec, TopoId, TopoKind,
};
use pxp_kernel_native::{BACKEND, NativeKernel};
use pxp_math::{Csys, Point3, UnitVec3, Vec3};
use pxp_units::Length;
use std::f64::consts::PI;
use std::sync::Arc;

const RES: Length = Length::from_mm(1e-6);

fn mm(v: f64) -> Length {
    Length::from_mm(v)
}

fn host() -> KernelHost {
    KernelHost::in_process(Box::new(NativeKernel::new()))
}

fn close(actual: f64, expected: f64, rel: f64) {
    let err = (actual - expected).abs() / expected.abs().max(1.0);
    assert!(
        err <= rel,
        "{actual} vs expected {expected} (rel err {err:e})"
    );
}

fn block_spec(x: f64, y: f64, z: f64) -> BlockSpec {
    BlockSpec {
        csys: Csys::ABSOLUTE,
        x: mm(x),
        y: mm(y),
        z: mm(z),
    }
}

fn counts(h: &KernelHost, s: ShapeHandle) -> [usize; 5] {
    let topo = h.call(|k| k.topology(s)).unwrap();
    let n = |kind| topo.iter().filter(|t: &&TopoId| t.kind == kind).count();
    [
        n(TopoKind::Vertex),
        n(TopoKind::Edge),
        n(TopoKind::Face),
        n(TopoKind::Shell),
        n(TopoKind::Body),
    ]
}

fn faces_with_role(r: &OpResult, role: &'static str) -> Vec<TopoId> {
    r.history
        .outputs_from(&HistorySource::Role(role))
        .filter(|t| t.kind == TopoKind::Face)
        .collect()
}

#[test]
fn block_measures_and_names_every_entity() {
    let h = host();
    let r = h.op(|k| k.block(&block_spec(10.0, 20.0, 30.0))).unwrap();
    assert_eq!(counts(&h, r.shape), [8, 12, 6, 1, 1]);
    assert_eq!(h.call(|k| k.body_kind(r.shape)).unwrap(), BodyKind::Solid);
    let m = h.call(|k| k.mass_properties(r.shape)).unwrap();
    close(m.volume.mm3(), 6000.0, 1e-9);
    close(m.area.mm2(), 2200.0, 1e-9);
    close(m.centroid.x, 5.0, 1e-9);
    close(m.centroid.y, 10.0, 1e-9);
    close(m.centroid.z, 15.0, 1e-9);
    for role in BlockSpec::FACE_ROLES {
        assert_eq!(faces_with_role(&r, role).len(), 1, "{role}");
    }
    // Every edge lies between exactly two named faces, every vertex at three.
    for rec in &r.history.records {
        let roles = rec
            .sources
            .iter()
            .filter(|s| matches!(s, HistorySource::Role(_)))
            .count();
        let expected = match rec.output.kind {
            TopoKind::Face => 1,
            TopoKind::Edge => 2,
            TopoKind::Vertex => 3,
            _ => unreachable!(),
        };
        assert_eq!(roles, expected, "{rec:?}");
        assert_eq!(rec.relation, Relation::Generated);
    }
    // The twelve edges have twelve different names.
    let mut names: Vec<_> = r
        .history
        .records
        .iter()
        .filter(|rec| rec.output.kind == TopoKind::Edge)
        .map(|rec| rec.sources.clone())
        .collect();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 12);
}

#[test]
fn placed_block_follows_its_csys() {
    let h = host();
    let (s, c) = (30_f64.to_radians().sin(), 30_f64.to_radians().cos());
    let csys = Csys::new(
        Point3::new(5.0, 6.0, 7.0),
        Vec3::new(c, s, 0.0),
        Vec3::new(-s, c, 0.0),
    )
    .unwrap();
    let r = h
        .op(|k| {
            k.block(&BlockSpec {
                csys,
                ..block_spec(10.0, 20.0, 30.0)
            })
        })
        .unwrap();
    let m = h.call(|k| k.mass_properties(r.shape)).unwrap();
    close(m.volume.mm3(), 6000.0, 1e-9);
    // Local centroid (5, 10, 15) rotated 30° about Z, then moved to (5, 6, 7).
    close(m.centroid.x, 5.0 + 5.0 * c - 10.0 * s, 1e-9);
    close(m.centroid.y, 6.0 + 5.0 * s + 10.0 * c, 1e-9);
    close(m.centroid.z, 7.0 + 15.0, 1e-9);
    for role in BlockSpec::FACE_ROLES {
        assert_eq!(faces_with_role(&r, role).len(), 1, "{role}");
    }
}

#[test]
fn cylinder_cone_and_sphere_match_hand_calculations() {
    let h = host();
    let tilted = Vec3::new(1.0, 2.0, 2.0).normalized().unwrap();

    let cyl = h
        .op(|k| {
            k.cylinder(&CylinderSpec {
                base: Point3::new(1.0, 2.0, 3.0),
                axis: tilted,
                diameter: mm(6.0),
                height: mm(40.0),
            })
        })
        .unwrap();
    let m = h.call(|k| k.mass_properties(cyl.shape)).unwrap();
    close(m.volume.mm3(), PI * 9.0 * 40.0, 1e-7);
    close(m.area.mm2(), 2.0 * PI * 3.0 * 40.0 + 2.0 * PI * 9.0, 1e-7);
    close(m.centroid.x, 1.0 + 20.0 / 3.0, 1e-7);
    assert_eq!(counts(&h, cyl.shape), [2, 3, 3, 1, 1]);
    for role in CylinderSpec::FACE_ROLES {
        assert_eq!(faces_with_role(&cyl, role).len(), 1, "{role}");
    }

    let frustum = h
        .op(|k| {
            k.cone(&ConeSpec {
                base: Point3::new(0.0, 0.0, 0.0),
                axis: UnitVec3::Z,
                base_diameter: mm(8.0),
                top_diameter: mm(3.0),
                height: mm(10.0),
            })
        })
        .unwrap();
    let m = h.call(|k| k.mass_properties(frustum.shape)).unwrap();
    // V = πh/3 (R² + Rr + r²), R = 4, r = 1.5.
    close(m.volume.mm3(), PI * 10.0 / 3.0 * (16.0 + 6.0 + 2.25), 1e-7);
    for role in ConeSpec::FACE_ROLES {
        assert_eq!(faces_with_role(&frustum, role).len(), 1, "{role}");
    }

    let pointed = h
        .op(|k| {
            k.cone(&ConeSpec {
                base: Point3::new(5.0, 0.0, 0.0),
                axis: UnitVec3::X,
                base_diameter: mm(8.0),
                top_diameter: mm(0.0),
                height: mm(10.0),
            })
        })
        .unwrap();
    let m = h.call(|k| k.mass_properties(pointed.shape)).unwrap();
    close(m.volume.mm3(), PI * 16.0 * 10.0 / 3.0, 1e-7);
    // Centroid of a solid cone is a quarter of the height above the base.
    close(m.centroid.x, 5.0 + 2.5, 1e-7);
    assert!(faces_with_role(&pointed, "cone.top").is_empty());
    assert_eq!(faces_with_role(&pointed, "cone.side").len(), 1);

    let sphere = h
        .op(|k| {
            k.sphere(&SphereSpec {
                center: Point3::new(-3.0, 0.0, 4.0),
                diameter: mm(14.0),
            })
        })
        .unwrap();
    let m = h.call(|k| k.mass_properties(sphere.shape)).unwrap();
    close(m.volume.mm3(), 4.0 / 3.0 * PI * 343.0, 1e-7);
    close(m.area.mm2(), 4.0 * PI * 49.0, 1e-7);
    close(m.centroid.z, 4.0, 1e-7);
    assert_eq!(counts(&h, sphere.shape), [2, 1, 1, 1, 1]);
}

#[test]
fn subtract_reports_kept_modified_and_deleted_entities() {
    let h = host();
    let target = h.op(|k| k.block(&block_spec(10.0, 20.0, 30.0))).unwrap();
    let tool = h
        .op(|k| {
            k.cylinder(&CylinderSpec {
                base: Point3::new(5.0, 10.0, -5.0),
                axis: UnitVec3::Z,
                diameter: mm(6.0),
                height: mm(40.0),
            })
        })
        .unwrap();
    let r = h
        .op(|k| k.boolean(BooleanOp::Subtract, target.shape, tool.shape))
        .unwrap();
    let m = h.call(|k| k.mass_properties(r.shape)).unwrap();
    close(m.volume.mm3(), 6000.0 - PI * 9.0 * 30.0, 1e-7);
    assert_eq!(counts(&h, r.shape)[2], 7);

    let face = |i| TopoId {
        kind: TopoKind::Face,
        index: i,
    };
    // The hole wall comes from the cylinder's side (operand 1, face 0).
    let from_cyl_side: Vec<_> = r
        .history
        .outputs_from(&HistorySource::Input {
            operand: 1,
            entity: face(0),
        })
        .filter(|t| t.kind == TopoKind::Face)
        .collect();
    assert_eq!(from_cyl_side.len(), 1);
    // The hole's two rims are where the block's top and bottom met the
    // cylinder side: generated edges naming both faces.
    let rims = r
        .history
        .records
        .iter()
        .filter(|rec| rec.output.kind == TopoKind::Edge && rec.relation == Relation::Generated)
        .filter(|rec| {
            rec.sources.contains(&HistorySource::Input {
                operand: 1,
                entity: face(0),
            })
        })
        .count();
    assert_eq!(rims, 2);
    // The cylinder's end caps lie outside the block: deleted.
    assert!(r.history.deleted.contains(&(1, face(1))));
    assert!(r.history.deleted.contains(&(1, face(2))));
    // All six block faces survive.
    for f in 0..6 {
        assert!(!r.history.deleted.contains(&(0, face(f))));
    }
    // The block's eight corners are the same vertices, modified (kept).
    let kept_corners = r
        .history
        .records
        .iter()
        .filter(|rec| rec.output.kind == TopoKind::Vertex && rec.relation == Relation::Modified)
        .filter(|rec| {
            matches!(rec.sources.as_slice(), [HistorySource::Input { operand: 0, entity }]
                if entity.kind == TopoKind::Vertex)
        })
        .count();
    assert_eq!(kept_corners, 8);
}

#[test]
fn empty_results_are_errors_not_empty_bodies() {
    let h = host();
    let a = h.op(|k| k.block(&block_spec(10.0, 10.0, 10.0))).unwrap();
    let b = h.op(|k| k.block(&block_spec(10.0, 10.0, 10.0))).unwrap();
    let far = h
        .op(|k| {
            k.sphere(&SphereSpec {
                center: Point3::new(100.0, 0.0, 0.0),
                diameter: mm(2.0),
            })
        })
        .unwrap();
    assert_eq!(
        h.op(|k| k.boolean(BooleanOp::Subtract, a.shape, b.shape)),
        Err(KernelError::EmptyResult { op: "boolean" })
    );
    assert_eq!(
        h.op(|k| k.boolean(BooleanOp::Intersect, a.shape, far.shape)),
        Err(KernelError::EmptyResult { op: "boolean" })
    );
}

#[test]
fn identical_inputs_give_identical_results() {
    let h = host();
    let spec = ConeSpec {
        base: Point3::new(1.0, -2.0, 0.5),
        axis: Vec3::new(0.0, 1.0, 1.0).normalized().unwrap(),
        base_diameter: mm(9.0),
        top_diameter: mm(2.0),
        height: mm(12.0),
    };
    let a = h.op(|k| k.cone(&spec)).unwrap();
    let b = h.op(|k| k.cone(&spec)).unwrap();
    assert_ne!(a.shape, b.shape);
    assert_eq!(a.history, b.history);
    let hash = |s| h.call(|k| k.geometry_hash(s, RES)).unwrap();
    assert_eq!(hash(a.shape), hash(b.shape));
    let bytes = |s| h.call(|k| k.export_native(s)).unwrap();
    assert_eq!(bytes(a.shape), bytes(b.shape));
}

#[test]
fn geometry_hash_sees_small_changes() {
    let h = host();
    let a = h.op(|k| k.block(&block_spec(10.0, 20.0, 30.0))).unwrap();
    let b = h.op(|k| k.block(&block_spec(10.0, 20.0, 30.001))).unwrap();
    let hash = |s| h.call(|k| k.geometry_hash(s, RES)).unwrap();
    assert_ne!(hash(a.shape), hash(b.shape));
}

#[test]
fn native_bytes_round_trip_exactly() {
    let h = host();
    let target = h.op(|k| k.block(&block_spec(30.0, 20.0, 5.0))).unwrap();
    let tool = h
        .op(|k| {
            k.sphere(&SphereSpec {
                center: Point3::new(12.0, 20.0, 0.0),
                diameter: mm(20.0),
            })
        })
        .unwrap();
    let cut = h
        .op(|k| k.boolean(BooleanOp::Subtract, target.shape, tool.shape))
        .unwrap();
    let bytes = h.call(|k| k.export_native(cut.shape)).unwrap();
    let back = h.call(|k| k.import_native(&bytes)).unwrap();
    assert_eq!(
        h.call(|k| k.geometry_hash(back, RES)).unwrap(),
        h.call(|k| k.geometry_hash(cut.shape, RES)).unwrap()
    );
    assert_eq!(
        h.call(|k| k.topology(back)).unwrap(),
        h.call(|k| k.topology(cut.shape)).unwrap()
    );
    assert_eq!(h.call(|k| k.export_native(back)).unwrap(), bytes);
    assert!(matches!(
        h.call(|k| k.import_native(b"not a body")),
        Err(KernelError::InvalidInput(_))
    ));
}

#[test]
fn invalid_specs_are_refused() {
    let h = host();
    assert!(matches!(
        h.op(|k| k.block(&block_spec(0.0, 1.0, 1.0))),
        Err(KernelError::InvalidInput(_))
    ));
    assert!(matches!(
        h.op(|k| {
            k.cone(&ConeSpec {
                base: Point3::new(0.0, 0.0, 0.0),
                axis: UnitVec3::Z,
                base_diameter: mm(4.0),
                top_diameter: mm(4.0),
                height: mm(1.0),
            })
        }),
        Err(KernelError::InvalidInput(_))
    ));
}

#[test]
fn handles_are_checked_and_released() {
    let kernel = Arc::new(NativeKernel::new());
    let r = kernel.block(&block_spec(1.0, 1.0, 1.0)).unwrap();
    assert_eq!(r.shape.backend, BACKEND);
    assert_eq!(kernel.live_shapes(), 1);
    let foreign = ShapeHandle {
        backend: pxp_kernel::BackendId("occt"),
        id: r.shape.id,
    };
    assert_eq!(
        kernel.topology(foreign),
        Err(KernelError::UnknownShape(foreign))
    );
    kernel.release(r.shape);
    kernel.release(r.shape); // releasing twice is a no-op
    assert_eq!(kernel.live_shapes(), 0);
    assert_eq!(
        kernel.body_kind(r.shape),
        Err(KernelError::UnknownShape(r.shape))
    );
}

#[test]
fn concurrent_calls_are_safe() {
    let kernel = NativeKernel::new();
    let hashes: Vec<u64> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..4)
            .map(|i| {
                let k = &kernel;
                s.spawn(move || {
                    let r = k.block(&block_spec(5.0 + f64::from(i), 5.0, 5.0)).unwrap();
                    k.geometry_hash(r.shape, RES).unwrap()
                })
            })
            .collect();
        workers.into_iter().map(|w| w.join().unwrap()).collect()
    });
    assert_eq!(kernel.live_shapes(), 4);
    let mut unique = hashes.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), 4);
}

#[test]
fn merged_counts_join_pieces_of_one_surface() {
    let h = host();
    let block = h.op(|k| k.block(&block_spec(1.0, 2.0, 3.0))).unwrap();
    let m = h.call(|k| k.merged_counts(block.shape)).unwrap();
    assert_eq!((m.faces, m.edges, m.vertices), (6, 12, 8));
    // A lens is two spherical caps, however the kernel splits them.
    let a = h
        .op(|k| {
            k.sphere(&SphereSpec {
                center: Point3::new(0.0, 0.0, 0.0),
                diameter: mm(20.0),
            })
        })
        .unwrap();
    let b = h
        .op(|k| {
            k.sphere(&SphereSpec {
                center: Point3::new(10.0, 0.0, 0.0),
                diameter: mm(20.0),
            })
        })
        .unwrap();
    let lens = h
        .op(|k| k.boolean(BooleanOp::Intersect, a.shape, b.shape))
        .unwrap();
    assert_eq!(h.call(|k| k.merged_counts(lens.shape)).unwrap().faces, 2);
}
