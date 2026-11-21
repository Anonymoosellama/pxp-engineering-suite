// SPDX-License-Identifier: LGPL-2.1-or-later
//! Property tests for the master prompt's geometric invariants that live in
//! this crate: rigid transforms preserve distances, inverses undo, Csys stays
//! orthonormal, equal values hash equally.

use proptest::prelude::*;
use pxp_math::{Csys, Point3, StableHasher, Transform, Vec3};
use pxp_units::Angle;

fn coord() -> impl Strategy<Value = f64> {
    -1.0e4_f64..1.0e4
}

fn point() -> impl Strategy<Value = Point3> {
    (coord(), coord(), coord()).prop_map(|(x, y, z)| Point3::new(x, y, z))
}

fn rigid() -> impl Strategy<Value = Transform> {
    (point(), point(), -360.0_f64..360.0).prop_filter_map("degenerate axis", |(a, t, deg)| {
        let axis = a.to_vec().normalized()?;
        Some(
            Transform::translation(t.to_vec())
                .then_after(&Transform::rotation(axis, Angle::from_deg(deg))),
        )
    })
}

proptest! {
    #[test]
    fn rigid_transforms_preserve_distance(t in rigid(), a in point(), b in point()) {
        let before = a.distance(b).mm();
        let after = t.apply_point(a).distance(t.apply_point(b)).mm();
        prop_assert!((before - after).abs() <= 1e-9 * before.max(1.0));
    }

    #[test]
    fn inverse_undoes(t in rigid(), p in point()) {
        let back = t.inverse().apply_point(t.apply_point(p));
        prop_assert!((back - p).length() <= 1e-8 * p.to_vec().length().max(1.0));
    }

    #[test]
    fn csys_is_orthonormal_and_right_handed(o in point(), x in point(), y in point()) {
        if let Ok(c) = Csys::new(o, x.to_vec(), y.to_vec()) {
            let (cx, cy, cz) = (c.x_axis().get(), c.y_axis().get(), c.z_axis().get());
            prop_assert!(cx.dot(cy).abs() < 1e-9 && cy.dot(cz).abs() < 1e-9 && cz.dot(cx).abs() < 1e-9);
            prop_assert!((cx.cross(cy) - cz).length() < 1e-9);
        }
    }

    #[test]
    fn equal_values_hash_equally(v in proptest::num::f64::ANY) {
        let h = |x: f64| { let mut s = StableHasher::new(); s.write_f64(x); s.finish() };
        prop_assert_eq!(h(v), h(v));
        if v == 0.0 { prop_assert_eq!(h(0.0), h(-0.0)); }
    }

    #[test]
    fn cross_product_is_perpendicular(a in point(), b in point()) {
        let (a, b): (Vec3, Vec3) = (a.to_vec(), b.to_vec());
        let c = a.cross(b);
        let scale = a.length() * b.length() * c.length();
        prop_assert!(c.dot(a).abs() <= 1e-9 * scale.max(1.0));
    }
}
