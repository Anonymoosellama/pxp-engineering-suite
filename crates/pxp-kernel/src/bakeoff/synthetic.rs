// SPDX-License-Identifier: LGPL-2.1-or-later
//! PXP's synthetic stress cases. Every expected value is derived in the
//! case's `what` text from first principles.

use super::{Case, Expect, Range, block, combine, cone, cylinder, make, solid, sphere};
use crate::BooleanOp::{Intersect, Subtract, Unite};
use std::f64::consts::PI;

/// Relative tolerance for exact analytic answers.
const TIGHT: f64 = 1e-6;
/// SolidCraft's rig default, used where tessellation-free exactness is not
/// the point of the case.
const LOOSE: f64 = 1e-3;

/// "Expect `value` within `rel`", shaped for the `solid(...)` arguments.
#[allow(clippy::unnecessary_wraps)]
fn v(value: f64, rel: f64) -> Option<Range> {
    Some(Range::rel(value, rel))
}

/// The synthetic corpus, in a fixed order.
#[must_use]
#[allow(clippy::too_many_lines)] // a table of cases
pub fn cases() -> Vec<Case> {
    let z = [0.0, 0.0, 1.0];
    let x = [1.0, 0.0, 0.0];
    vec![
        // ---- primitives -------------------------------------------------
        Case {
            slow: false,
            id: "syn/prim/block-unit",
            source: "primitive",
            what: "1 mm cube: V = 1, A = 6, 6 faces.",
            steps: vec![make("A", block([0.0; 3], [1.0, 1.0, 1.0]))],
            expect: solid(v(1.0, TIGHT), v(6.0, TIGHT), Some(6), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/prim/block-large-far-from-origin",
            source: "precision",
            what: "1000 x 500 x 250 block with a corner at (1e5, -2e5, 3e4): V = 1.25e8, A = 2(5e5 + 2.5e5 + 1.25e5) = 1.75e6.",
            steps: vec![make("A", block([1e5, -2e5, 3e4], [1000.0, 500.0, 250.0]))],
            expect: solid(v(1.25e8, TIGHT), v(1.75e6, TIGHT), Some(6), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/prim/block-thin-plate",
            source: "thin",
            what: "100 x 100 x 0.01 plate: V = 100, A = 2*1e4 + 4*100*0.01 = 20004.",
            steps: vec![make("A", block([0.0; 3], [100.0, 100.0, 0.01]))],
            expect: solid(v(100.0, TIGHT), v(20004.0, TIGHT), Some(6), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/prim/cylinder-tilted",
            source: "primitive",
            what: "r 5, h 25 on axis (1,1,1) from (3,-4,5): V = 625 pi, A = 2 pi 5 25 + 2 pi 25 = 300 pi.",
            steps: vec![make(
                "A",
                cylinder([3.0, -4.0, 5.0], [1.0, 1.0, 1.0], 10.0, 25.0),
            )],
            expect: solid(v(625.0 * PI, TIGHT), v(300.0 * PI, TIGHT), Some(3), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/prim/cone-frustum",
            source: "primitive",
            what: "R 10, r 5, h 30: V = pi h/3 (R^2 + Rr + r^2) = 1750 pi; A = pi (R+r) sqrt(h^2 + (R-r)^2) + pi R^2 + pi r^2 = 15 pi sqrt(925) + 125 pi.",
            steps: vec![make("A", cone([0.0; 3], z, 20.0, 10.0, 30.0))],
            expect: solid(
                v(1750.0 * PI, TIGHT),
                v(15.0 * PI * 925_f64.sqrt() + 125.0 * PI, TIGHT),
                Some(3),
                Some(1),
            ),
        },
        Case {
            slow: false,
            id: "syn/prim/cone-pointed-down",
            source: "primitive",
            what: "Pointed cone R 6, h 12 from z = 10 along -Z: V = pi 36 12 / 3 = 144 pi; A = pi 6 sqrt(180) + 36 pi.",
            steps: vec![make(
                "A",
                cone([0.0, 0.0, 10.0], [0.0, 0.0, -1.0], 12.0, 0.0, 12.0),
            )],
            expect: solid(
                v(144.0 * PI, TIGHT),
                v(6.0 * PI * 180_f64.sqrt() + 36.0 * PI, TIGHT),
                Some(2),
                Some(1),
            ),
        },
        Case {
            slow: false,
            id: "syn/prim/sphere-tiny",
            source: "tiny",
            what: "Sphere of radius 1 micrometre: V = 4/3 pi 1e-9.",
            steps: vec![make("A", sphere([0.0; 3], 0.002))],
            expect: solid(v(4.0 / 3.0 * PI * 1e-9, LOOSE), None, Some(1), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/prim/sphere-large",
            source: "large",
            what: "Sphere of radius 1 m: V = 4/3 pi 1e9, A = 4 pi 1e6.",
            steps: vec![make("A", sphere([0.0; 3], 2000.0))],
            expect: solid(
                v(4.0 / 3.0 * PI * 1e9, TIGHT),
                v(4.0 * PI * 1e6, TIGHT),
                Some(1),
                Some(1),
            ),
        },
        // ---- refusals ---------------------------------------------------
        Case {
            slow: false,
            id: "syn/refuse/zero-height-cylinder",
            source: "validation",
            what: "A cylinder of height 0 is invalid input, not an empty body.",
            steps: vec![make("A", cylinder([0.0; 3], z, 10.0, 0.0))],
            expect: Expect::Refused,
        },
        Case {
            slow: false,
            id: "syn/refuse/negative-sphere",
            source: "validation",
            what: "A negative diameter is invalid input.",
            steps: vec![make("A", sphere([0.0; 3], -1.0))],
            expect: Expect::Refused,
        },
        Case {
            slow: false,
            id: "syn/refuse/cone-equal-diameters",
            source: "validation",
            what: "A cone with equal diameters is a cylinder: refused, the user picks Cylinder.",
            steps: vec![make("A", cone([0.0; 3], z, 5.0, 5.0, 5.0))],
            expect: Expect::Refused,
        },
        // ---- booleans ---------------------------------------------------
        Case {
            slow: false,
            id: "syn/bool/through-hole",
            source: "boolean",
            what: "10x20x30 block minus a d6 cylinder through it along Z: V = 6000 - 9 pi 30 = 6000 - 270 pi; 7 faces.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 20.0, 30.0])),
                make("T", cylinder([5.0, 10.0, -5.0], z, 6.0, 40.0)),
                combine("A", &["T"], Subtract, false),
            ],
            expect: solid(v(6000.0 - 270.0 * PI, TIGHT), None, Some(7), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/blind-hole",
            source: "boolean",
            what: "Same block, d6 cylinder entering the top face 10 deep: V = 6000 - 90 pi; 8 faces (wall and floor added).",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 20.0, 30.0])),
                make("T", cylinder([5.0, 10.0, 20.0], z, 6.0, 20.0)),
                combine("A", &["T"], Subtract, false),
            ],
            expect: solid(v(6000.0 - 90.0 * PI, TIGHT), None, Some(8), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/coplanar-face-union",
            source: "coincident faces",
            what: "Two 10x20x30 blocks sharing the whole face x = 10: one 20x20x30 box, V = 12000, A = 3200, 6 faces (coplanar faces merged).",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 20.0, 30.0])),
                make("B", block([10.0, 0.0, 0.0], [10.0, 20.0, 30.0])),
                combine("A", &["B"], Unite, false),
            ],
            expect: solid(v(12000.0, TIGHT), v(3200.0, TIGHT), Some(6), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/coplanar-partial-face-union",
            source: "coincident faces",
            what: "The second block shifted 5 in Y, sharing a 15x30 patch of x = 10: V = 12000, A = 4400 - 2*15*30 = 3500.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 20.0, 30.0])),
                make("B", block([10.0, 5.0, 0.0], [10.0, 20.0, 30.0])),
                combine("A", &["B"], Unite, false),
            ],
            expect: solid(v(12000.0, TIGHT), v(3500.0, TIGHT), None, Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/edge-touching-union",
            source: "non-manifold contact",
            what: "Two 10 mm cubes touching only along the line x = y = 10: V = 2000 (no volume shared).",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("B", block([10.0, 10.0, 0.0], [10.0, 10.0, 10.0])),
                combine("A", &["B"], Unite, false),
            ],
            expect: solid(v(2000.0, TIGHT), v(1200.0, TIGHT), None, None),
        },
        Case {
            slow: false,
            id: "syn/bool/vertex-touching-union",
            source: "non-manifold contact",
            what: "Two 10 mm cubes touching at the single point (10,10,10): V = 2000, A = 1200.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("B", block([10.0, 10.0, 10.0], [10.0, 10.0, 10.0])),
                combine("A", &["B"], Unite, false),
            ],
            expect: solid(v(2000.0, TIGHT), v(1200.0, TIGHT), None, None),
        },
        Case {
            slow: false,
            id: "syn/bool/disjoint-union",
            source: "multi-lump",
            what: "Two 10 mm cubes 5 mm apart: one body of two lumps, V = 2000, 2 shells.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("B", block([15.0, 0.0, 0.0], [10.0, 10.0, 10.0])),
                combine("A", &["B"], Unite, false),
            ],
            expect: solid(v(2000.0, TIGHT), v(1200.0, TIGHT), Some(12), Some(2)),
        },
        Case {
            slow: false,
            id: "syn/bool/internal-void",
            source: "void",
            what: "20 mm cube minus a centred d10 sphere: V = 8000 - 4/3 pi 125; A = 2400 + 100 pi; 7 faces, 2 shells (outer and void).",
            steps: vec![
                make("A", block([0.0; 3], [20.0, 20.0, 20.0])),
                make("T", sphere([10.0, 10.0, 10.0], 10.0)),
                combine("A", &["T"], Subtract, false),
            ],
            expect: solid(
                v(8000.0 - 4.0 / 3.0 * PI * 125.0, TIGHT),
                v(2400.0 + 100.0 * PI, TIGHT),
                Some(7),
                Some(2),
            ),
        },
        Case {
            slow: false,
            id: "syn/bool/tangent-cylinder-on-face",
            source: "tangent",
            what: "10 mm cube united with a d10 cylinder along X lying on its top face (line contact at z = 10): V = 1000 + 250 pi.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("T", cylinder([0.0, 5.0, 15.0], x, 10.0, 10.0)),
                combine("A", &["T"], Unite, false),
            ],
            expect: solid(v(1000.0 + 250.0 * PI, TIGHT), None, None, None),
        },
        Case {
            slow: false,
            id: "syn/bool/inscribed-cylinder-cut",
            source: "tangent",
            what: "10 mm cube minus the d10 cylinder along X inscribed in its square section (tangent to four faces): four corner slivers, V = 1000 - 250 pi.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("T", cylinder([-1.0, 5.0, 5.0], x, 10.0, 12.0)),
                combine("A", &["T"], Subtract, false),
            ],
            expect: solid(v(1000.0 - 250.0 * PI, TIGHT), None, None, None),
        },
        Case {
            slow: false,
            id: "syn/bool/tiny-offset-union",
            source: "tiny edges",
            what: "10 mm cube united with a copy moved 1e-4 mm (0.1 micrometre) in X: a 10.0001 x 10 x 10 box, V = 1000.01, 6 faces (the 1e-4 wide slivers merge).",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("B", block([1e-4, 0.0, 0.0], [10.0, 10.0, 10.0])),
                combine("A", &["B"], Unite, false),
            ],
            expect: solid(v(1000.01, TIGHT), None, Some(6), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/near-coincident-union",
            source: "near tolerance",
            what: "10 mm cube united with a copy moved 1e-8 mm (below modeling tolerance): the same cube, V = 1000 within 1e-6.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("B", block([1e-8, 0.0, 0.0], [10.0, 10.0, 10.0])),
                combine("A", &["B"], Unite, false),
            ],
            expect: solid(v(1000.0, TIGHT), None, Some(6), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/thin-cap-union",
            source: "thin",
            what: "10 mm cube united with a 0.0001 mm plate on its top face: a 10 x 10 x 10.0001 box, V = 1000 + 100 * 0.0001 = 1000.01, 6 faces.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("B", block([0.0, 0.0, 10.0], [10.0, 10.0, 1e-4])),
                combine("A", &["B"], Unite, false),
            ],
            expect: solid(v(1000.01, TIGHT), None, Some(6), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/sphere-lens",
            source: "boolean",
            what: "Intersection of two r 10 spheres 10 apart: lens V = pi (4r + d)(2r - d)^2 / 12 = pi 50 100 / 12.",
            steps: vec![
                make("A", sphere([0.0; 3], 20.0)),
                make("B", sphere([10.0, 0.0, 0.0], 20.0)),
                combine("A", &["B"], Intersect, false),
            ],
            expect: solid(
                v(5000.0 * PI / 12.0, TIGHT),
                v(2.0 * 2.0 * PI * 10.0 * 5.0, TIGHT),
                Some(2),
                Some(1),
            ),
        },
        Case {
            slow: false,
            id: "syn/bool/cone-cut-by-block",
            source: "boolean",
            what: "Pointed cone R 10, h 20 minus everything above z = 10: frustum R 10, r 5, h 10, V = pi 10/3 (100 + 50 + 25) = 1750 pi / 3; 3 faces.",
            steps: vec![
                make("A", cone([0.0; 3], z, 20.0, 0.0, 20.0)),
                make("T", block([-20.0, -20.0, 10.0], [40.0, 40.0, 20.0])),
                combine("A", &["T"], Subtract, false),
            ],
            expect: solid(v(1750.0 * PI / 3.0, TIGHT), None, Some(3), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/coaxial-cylinder-union",
            source: "coincident faces",
            what: "Two d10 h10 cylinders on one axis overlapping by 5: one d10 h15 cylinder, V = 375 pi, 3 faces (sides merge).",
            steps: vec![
                make("A", cylinder([0.0; 3], z, 10.0, 10.0)),
                make("B", cylinder([0.0, 0.0, 5.0], z, 10.0, 10.0)),
                combine("A", &["B"], Unite, false),
            ],
            expect: solid(v(375.0 * PI, TIGHT), None, Some(3), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/tube",
            source: "boolean",
            what: "d20 h10 cylinder minus a coaxial d16 cylinder through it: V = pi 10 (100 - 64) = 360 pi; 4 faces.",
            steps: vec![
                make("A", cylinder([0.0; 3], z, 20.0, 10.0)),
                make("T", cylinder([0.0, 0.0, -1.0], z, 16.0, 12.0)),
                combine("A", &["T"], Subtract, false),
            ],
            expect: solid(v(360.0 * PI, TIGHT), None, Some(4), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/nine-holes",
            source: "repeated booleans",
            what: "100x100x10 plate minus nine d5 through holes on a 25 mm grid: V = 1e5 - 9 pi 6.25 10 = 1e5 - 562.5 pi; 15 faces.",
            steps: {
                let mut s = vec![make("A", block([0.0; 3], [100.0, 100.0, 10.0]))];
                let names = ["H1", "H2", "H3", "H4", "H5", "H6", "H7", "H8", "H9"];
                let grid = [25.0, 50.0, 75.0];
                let centres = grid
                    .iter()
                    .flat_map(|&cy| grid.iter().map(move |&cx| (cx, cy)));
                for (n, (cx, cy)) in names.iter().zip(centres) {
                    s.push(make(n, cylinder([cx, cy, -1.0], z, 5.0, 12.0)));
                }
                s.push(combine("A", &names, Subtract, false));
                s
            },
            expect: solid(v(1e5 - 562.5 * PI, TIGHT), None, Some(15), Some(1)),
        },
        Case {
            slow: false,
            id: "syn/bool/diamond-cut-leaves-four-corners",
            source: "multi-lump",
            what: "10 mm cube minus a 10x10x20 block turned 45 deg about the cube's vertical axis: the octagon both squares share has area 2 a^2 (sqrt 2 - 1), so V = 10 (100 - 200 (sqrt 2 - 1)); four separate corner prisms.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make(
                    "T",
                    super::Prim::Block(crate::BlockSpec {
                        csys: pxp_math::Csys::new(
                            pxp_math::Point3::new(5.0, 5.0 - 50_f64.sqrt(), -5.0),
                            pxp_math::Vec3::new(1.0, 1.0, 0.0),
                            pxp_math::Vec3::new(-1.0, 1.0, 0.0),
                        )
                        .unwrap_or(pxp_math::Csys::ABSOLUTE),
                        x: super::mm(10.0),
                        y: super::mm(10.0),
                        z: super::mm(20.0),
                    }),
                ),
                combine("A", &["T"], Subtract, false),
            ],
            expect: solid(
                v(10.0 * (100.0 - 200.0 * (2_f64.sqrt() - 1.0)), TIGHT),
                None,
                Some(20),
                Some(4),
            ),
        },
        Case {
            slow: false,
            id: "syn/bool/identical-subtract-is-empty",
            source: "empty result",
            what: "A cube minus itself leaves nothing: EmptyResult, not an empty body.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("B", block([0.0; 3], [10.0, 10.0, 10.0])),
                combine("A", &["B"], Subtract, false),
            ],
            expect: Expect::Empty,
        },
        Case {
            slow: false,
            id: "syn/bool/disjoint-intersect-is-empty",
            source: "empty result",
            what: "Intersecting bodies 90 mm apart leaves nothing.",
            steps: vec![
                make("A", block([0.0; 3], [10.0, 10.0, 10.0])),
                make("B", sphere([100.0, 0.0, 0.0], 10.0)),
                combine("A", &["B"], Intersect, false),
            ],
            expect: Expect::Empty,
        },
    ]
}
