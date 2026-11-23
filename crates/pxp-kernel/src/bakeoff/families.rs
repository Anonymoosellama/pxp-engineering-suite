// SPDX-License-Identifier: LGPL-2.1-or-later
//! Generated families of stress cases with closed-form answers.
//!
//! Each family sweeps one parameter (a count, an offset, an angle, a scale)
//! through a construction whose volume and area are known exactly, so the
//! corpus covers many more geometric situations than hand-written cases
//! without any answer coming from a kernel. Every formula is derived in the
//! case's `what` text, with the parameter values filled in.
//!
//! | Family | Stresses |
//! | --- | --- |
//! | `disjoint-array` | unions of separate bodies (several shells in one result) |
//! | `hole-grid` | many sequential subtracts on one body |
//! | `coplanar-stack` | exactly coincident faces merged by a union |
//! | `shifted-stack` | near-coincident faces leaving strips 1 mm down to 0.1 µm wide |
//! | `steinmetz` | cylinder-cylinder intersections at oblique angles, and three cylinders |
//! | `sphere-cap` | a plane cutting a sphere at every height |
//! | `cone-bore` | a cone bored by a coaxial cylinder |
//! | `tangent-hole` | a hole whose wall thins from 0.5 mm to exactly zero (tangency) |
//! | `voids` | internal cavities (one shell per void) |
//! | `scale` | the same part from 0.2 mm to 200 m |
//! | `far` | the same part up to 1 km from the origin |
//! | `orientation` | the same part rotated about a skew axis |
//! | `lens` | two spheres intersected and united at every separation |
//! | `stepped-shaft` | coaxial cylinders whose end faces meet exactly |
//! | `repeat` | subtracting a tool that coincides with an existing hole |
//! | `octant`, `quarter` | spheres and cylinders cut by three and two coordinate planes |
//! | `cone-apex` | a cone apex approaching, then touching, a face |
//!
//! Identifiers and descriptions are built from the parameters at run time
//! and kept for the life of the program (the corpus is built once per run).

use super::{Case, Expect, Prim, Range, combine, cone, cylinder, make, solid, sphere};
use crate::BlockSpec;
use crate::BooleanOp::{Intersect, Subtract, Unite};
use pxp_math::{Csys, Point3, Vec3};
use pxp_units::Length;
use std::f64::consts::PI;

/// Relative tolerance for exact answers.
const TIGHT: f64 = 1e-6;
/// For parts placed so far from the origin that double precision itself
/// limits what any kernel can resolve.
const LOOSE: f64 = 1e-3;

/// Body names for families that need many bodies.
const T: [&str; 40] = [
    "T00", "T01", "T02", "T03", "T04", "T05", "T06", "T07", "T08", "T09", "T10", "T11", "T12",
    "T13", "T14", "T15", "T16", "T17", "T18", "T19", "T20", "T21", "T22", "T23", "T24", "T25",
    "T26", "T27", "T28", "T29", "T30", "T31", "T32", "T33", "T34", "T35", "T36", "T37", "T38",
    "T39",
];

/// A string kept for the rest of the program (generated ids and texts).
fn keep(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// "Expect `value` within `rel`", shaped for the `solid(...)` arguments.
#[allow(clippy::unnecessary_wraps)]
fn v(value: f64, rel: f64) -> Option<Range> {
    Some(Range::rel(value, rel))
}

/// A block with edges along the axes and one corner at `corner`.
fn block(corner: [f64; 3], size: [f64; 3]) -> Prim {
    super::block(corner, size)
}

/// A block of `size` whose local axes are `x` and `y` (orthonormalised),
/// with one corner at `origin`.
fn placed_block(origin: [f64; 3], x: [f64; 3], y: [f64; 3], size: [f64; 3]) -> Prim {
    let p = |a: [f64; 3]| Vec3::new(a[0], a[1], a[2]);
    let csys = Csys::new(Point3::new(origin[0], origin[1], origin[2]), p(x), p(y))
        .unwrap_or(Csys::ABSOLUTE);
    Prim::Block(BlockSpec {
        csys,
        x: Length::from_mm(size[0]),
        y: Length::from_mm(size[1]),
        z: Length::from_mm(size[2]),
    })
}

/// Rotates `p` by `angle` (radians) about the unit axis `k` (Rodrigues).
fn rotate(p: [f64; 3], k: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    let dot = k[0] * p[0] + k[1] * p[1] + k[2] * p[2];
    let cross = [
        k[1] * p[2] - k[2] * p[1],
        k[2] * p[0] - k[0] * p[2],
        k[0] * p[1] - k[1] * p[0],
    ];
    [
        p[0] * c + cross[0] * s + k[0] * dot * (1.0 - c),
        p[1] * c + cross[1] * s + k[1] * dot * (1.0 - c),
        p[2] * c + cross[2] * s + k[2] * dot * (1.0 - c),
    ]
}

fn case(
    id: String,
    source: &'static str,
    what: String,
    steps: Vec<super::Step>,
    expect: Expect,
) -> Case {
    Case {
        id: keep(id),
        source,
        what: keep(what),
        steps,
        expect,
        slow: false,
    }
}

/// The 20 x 20 x 10 block with a 4 mm-radius through hole at its centre,
/// scaled by `s`, translated by `offset` and turned by `turn` (axis, angle):
/// V = s³(4000 − 160π), A = s²(1600 + 48π) (block area 2·400 + 4·200 = 1600,
/// less two hole openings 2·16π, plus the bore 2π·4·10 = 80π), 7 faces.
fn holed_block(s: f64, offset: [f64; 3], turn: Option<([f64; 3], f64)>) -> Vec<super::Step> {
    let r = |p: [f64; 3]| turn.map_or(p, |(k, a)| rotate(p, k, a));
    let add = |p: [f64; 3]| [p[0] + offset[0], p[1] + offset[1], p[2] + offset[2]];
    let origin = add([0.0; 3]);
    let base = add(r([10.0 * s, 10.0 * s, -s]));
    vec![
        make(
            "A",
            placed_block(
                origin,
                r([1.0, 0.0, 0.0]),
                r([0.0, 1.0, 0.0]),
                [20.0 * s, 20.0 * s, 10.0 * s],
            ),
        ),
        make("H", cylinder(base, r([0.0, 0.0, 1.0]), 8.0 * s, 12.0 * s)),
        combine("A", &["H"], Subtract, false),
    ]
}

fn disjoint_array(out: &mut Vec<Case>) {
    for n in [2_usize, 4, 8, 16] {
        let mut steps = Vec::new();
        for (i, name) in T.iter().take(n).enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let x = 20.0 * i as f64;
            steps.push(make(name, block([x, 0.0, 0.0], [10.0; 3])));
        }
        steps.push(combine("T00", T.get(1..n).unwrap_or(&[]), Unite, false));
        #[allow(clippy::cast_precision_loss)]
        let nf = n as f64;
        out.push(case(
            format!("gen/disjoint-array/{n}"),
            "union of separate bodies",
            format!("{n} disjoint 10 mm cubes 10 mm apart united: V = {n}·1000, A = {n}·600, {} faces, {n} shells.", 6 * n),
            steps,
            solid(v(1000.0 * nf, TIGHT), v(600.0 * nf, TIGHT), Some(6 * n), Some(n)),
        ));
    }
}

fn hole_grid(out: &mut Vec<Case>) {
    let (len_x, len_y, thick, radius) = (60.0, 40.0, 10.0, 2.0);
    for (n, m) in [
        (1_usize, 1_usize),
        (2, 1),
        (2, 2),
        (3, 2),
        (3, 3),
        (4, 3),
        (5, 4),
        (6, 4),
        (6, 6),
    ] {
        let mut steps = vec![make("A", block([0.0; 3], [len_x, len_y, thick]))];
        let mut tools = Vec::new();
        for i in 0..n {
            for j in 0..m {
                let name = T.get(i * m + j).copied().unwrap_or("T39");
                #[allow(clippy::cast_precision_loss)]
                let (cx, cy) = (
                    len_x * (i as f64 + 0.5) / n as f64,
                    len_y * (j as f64 + 0.5) / m as f64,
                );
                steps.push(make(
                    name,
                    cylinder([cx, cy, -1.0], [0.0, 0.0, 1.0], 2.0 * radius, thick + 2.0),
                ));
                tools.push(name);
            }
        }
        steps.push(combine("A", &tools, Subtract, false));
        let holes = n * m;
        #[allow(clippy::cast_precision_loss)]
        let kf = holes as f64;
        let vol = len_x * len_y * thick - kf * PI * radius * radius * thick;
        let area = 2.0 * (len_x * len_y - kf * PI * radius * radius)
            + 2.0 * (len_x + len_y) * thick
            + kf * 2.0 * PI * radius * thick;
        out.push(case(
            format!("gen/hole-grid/{n}x{m}"),
            "sequential subtracts",
            format!("60x40x10 block less a {n}x{m} grid of r 2 through holes: V = 24000 − {holes}·π·4·10, A = 2(2400 − {holes}·4π) + 2·100·10 + {holes}·2π·2·10, {} faces.", 6 + holes),
            steps,
            solid(v(vol, TIGHT), v(area, TIGHT), Some(6 + holes), Some(1)),
        ));
    }
}

fn coplanar_stack(out: &mut Vec<Case>) {
    for k in [2_usize, 3, 5, 8, 12] {
        let mut steps = Vec::new();
        for (i, name) in T.iter().take(k).enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let z = 5.0 * i as f64;
            steps.push(make(name, block([0.0, 0.0, z], [10.0, 10.0, 5.0])));
        }
        steps.push(combine("T00", T.get(1..k).unwrap_or(&[]), Unite, false));
        #[allow(clippy::cast_precision_loss)]
        let kf = k as f64;
        out.push(case(
            format!("gen/coplanar-stack/{k}"),
            "coincident faces",
            format!("{k} blocks 10x10x5 stacked face to face and united into one 10x10x{} box: V = 500·{k}, A = 200 + 200·{k}, 6 merged faces.", 5 * k),
            steps,
            solid(v(500.0 * kf, TIGHT), v(200.0 + 200.0 * kf, TIGHT), Some(6), Some(1)),
        ));
    }
}

fn shifted_stack(out: &mut Vec<Case>) {
    for (label, e) in [
        ("1", 1.0),
        ("0.1", 0.1),
        ("0.01", 0.01),
        ("1e-3", 1e-3),
        ("1e-4", 1e-4),
    ] {
        out.push(case(
            format!("gen/shifted-stack/{label}"),
            "near-coincident faces, tiny faces",
            format!("10 mm cube on a 10 mm cube shifted {label} mm along X: V = 2000; A = 2·600 − 2·(10 − {label})·10 = 1000 + 20·{label}; 10 faces (bottom, top, merged y = 0 and y = 10 faces, four X faces, and two {label} mm strips at z = 10)."),
            vec![
                make("A", block([0.0; 3], [10.0; 3])),
                make("B", block([e, 0.0, 10.0], [10.0; 3])),
                combine("A", &["B"], Unite, false),
            ],
            solid(v(2000.0, TIGHT), v(1000.0 + 20.0 * e, TIGHT), Some(10), Some(1)),
        ));
    }
}

fn steinmetz(out: &mut Vec<Case>) {
    let r: f64 = 5.0;
    for deg in [90.0_f64, 75.0, 60.0, 45.0, 30.0] {
        let t = deg.to_radians();
        let (s, c) = t.sin_cos();
        out.push(case(
            format!("gen/steinmetz/{deg}deg"),
            "cylinder-cylinder intersection",
            format!("Two r 5 cylinders whose axes cross at {deg}° through the origin, intersected. A slice at height z is a rhombus of strips 2√(r²−z²) wide, area 4(r²−z²)/sin θ, so V = 16r³/(3 sin θ); each cylinder keeps ∫2r²|cos φ|/sin θ dφ = 8r²/sin θ, so A = 16r²/sin θ."),
            vec![
                make("A", cylinder([-100.0, 0.0, 0.0], [1.0, 0.0, 0.0], 2.0 * r, 200.0)),
                make("B", cylinder([-100.0 * c, -100.0 * s, 0.0], [c, s, 0.0], 2.0 * r, 200.0)),
                combine("A", &["B"], Intersect, false),
            ],
            solid(v(16.0 * r.powi(3) / (3.0 * s), TIGHT), v(16.0 * r * r / s, TIGHT), None, Some(1)),
        ));
    }
    let k = 2.0 - 2_f64.sqrt();
    out.push(case(
        "gen/steinmetz/tricylinder".into(),
        "three-cylinder intersection",
        "Three mutually perpendicular r 5 cylinders through the origin, intersected: V = 8(2 − √2)r³, A = 24(2 − √2)r² (Moore, 1974).".into(),
        vec![
            make("A", cylinder([-100.0, 0.0, 0.0], [1.0, 0.0, 0.0], 2.0 * r, 200.0)),
            make("B", cylinder([0.0, -100.0, 0.0], [0.0, 1.0, 0.0], 2.0 * r, 200.0)),
            make("C", cylinder([0.0, 0.0, -100.0], [0.0, 0.0, 1.0], 2.0 * r, 200.0)),
            combine("A", &["B", "C"], Intersect, false),
        ],
        solid(v(8.0 * k * r.powi(3), TIGHT), v(24.0 * k * r * r, TIGHT), None, Some(1)),
    ));
}

fn sphere_cap(out: &mut Vec<Case>) {
    let r: f64 = 10.0;
    for h in [0.5_f64, 1.0, 5.0, 10.0, 15.0, 19.0] {
        let a2 = h * (2.0 * r - h);
        out.push(case(
            format!("gen/sphere-cap/h{h}"),
            "plane-sphere",
            format!("R 10 sphere intersected with the half-space z ≥ {}: a cap of height h = {h}, V = πh²(3R − h)/3, A = 2πRh + πh(2R − h), 2 faces.", r - h),
            vec![
                make("S", sphere([0.0; 3], 2.0 * r)),
                make("B", block([-20.0, -20.0, r - h], [40.0, 40.0, 20.0])),
                combine("S", &["B"], Intersect, false),
            ],
            solid(v(PI * h * h * (3.0 * r - h) / 3.0, TIGHT), v(2.0 * PI * r * h + PI * a2, TIGHT), Some(2), Some(1)),
        ));
    }
}

fn cone_bore(out: &mut Vec<Case>) {
    let (r1, r2, h) = (10.0_f64, 5.0_f64, 20.0_f64);
    let slant = ((r1 - r2).powi(2) + h * h).sqrt();
    for b in [1.0_f64, 2.0, 3.0, 4.0] {
        let vol = PI * h * (r1 * r1 + r1 * r2 + r2 * r2) / 3.0 - PI * b * b * h;
        let area = PI * (r1 + r2) * slant
            + PI * (r1 * r1 - b * b)
            + PI * (r2 * r2 - b * b)
            + 2.0 * PI * b * h;
        out.push(case(
            format!("gen/cone-bore/r{b}"),
            "cone-cylinder",
            format!("Frustum R 10, r 5, h 20 bored by a coaxial r {b} cylinder: V = πh(R² + Rr + r²)/3 − π{b}²h, A = π(R + r)√((R − r)² + h²) + π(R² − {b}²) + π(r² − {b}²) + 2π{b}h, 4 faces."),
            vec![
                make("A", cone([0.0; 3], [0.0, 0.0, 1.0], 2.0 * r1, 2.0 * r2, h)),
                make("H", cylinder([0.0, 0.0, -1.0], [0.0, 0.0, 1.0], 2.0 * b, h + 2.0)),
                combine("A", &["H"], Subtract, false),
            ],
            solid(v(vol, TIGHT), v(area, TIGHT), Some(4), Some(1)),
        ));
    }
}

fn tangent_hole(out: &mut Vec<Case>) {
    let r: f64 = 5.0;
    let vol = 40.0 * 20.0 * 20.0 - PI * r * r * 40.0;
    let area = 4000.0 - 2.0 * PI * r * r + 2.0 * PI * r * 40.0;
    for (label, d) in [
        ("0.1", 0.1),
        ("0.01", 0.01),
        ("1e-3", 1e-3),
        ("1e-4", 1e-4),
        ("0", 0.0),
    ] {
        out.push(case(
            format!("gen/tangent-hole/{label}"),
            "near-tangent and tangent faces",
            format!("40x20x20 block less an r 5 hole along X whose axis is r(1 + {label}) below the top face (wall {} mm; 0 is tangent): V = 16000 − π25·40, A = 4000 − 2π25 + 2π5·40.", r * d),
            vec![
                make("A", block([0.0; 3], [40.0, 20.0, 20.0])),
                make("H", cylinder([-1.0, 10.0, 20.0 - r * (1.0 + d)], [1.0, 0.0, 0.0], 2.0 * r, 42.0)),
                combine("A", &["H"], Subtract, false),
            ],
            solid(v(vol, TIGHT), v(area, TIGHT), None, Some(1)),
        ));
    }
}

fn voids(out: &mut Vec<Case>) {
    let r: f64 = 4.0;
    for n in [1_usize, 2, 4, 8] {
        let mut steps = vec![make("A", block([0.0; 3], [100.0, 20.0, 20.0]))];
        let mut tools = Vec::new();
        for (i, name) in T.iter().take(n).enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let x = 6.0 + 12.0 * i as f64;
            steps.push(make(name, sphere([x, 10.0, 10.0], 2.0 * r)));
            tools.push(*name);
        }
        steps.push(combine("A", &tools, Subtract, false));
        #[allow(clippy::cast_precision_loss)]
        let nf = n as f64;
        out.push(case(
            format!("gen/voids/{n}"),
            "internal cavities",
            format!("100x20x20 block with {n} internal r 4 spherical voids: V = 40000 − {n}·(4/3)π64, A = 8800 + {n}·4π16, {} faces, {} shells.", 6 + n, 1 + n),
            steps,
            solid(
                v(40000.0 - nf * 4.0 / 3.0 * PI * r.powi(3), TIGHT),
                v(8800.0 + nf * 4.0 * PI * r * r, TIGHT),
                Some(6 + n),
                Some(1 + n),
            ),
        ));
    }
}

fn transformed(out: &mut Vec<Case>) {
    let base_v = 4000.0 - 160.0 * PI;
    let base_a = 1600.0 + 48.0 * PI;
    for (label, s) in [
        ("0.01", 0.01),
        ("0.1", 0.1),
        ("1", 1.0),
        ("10", 10.0),
        ("100", 100.0),
        ("1e3", 1e3),
        ("1e4", 1e4),
    ] {
        out.push(case(
            format!("gen/scale/{label}"),
            "scale",
            format!("The holed block (20x20x10, r 4 through hole) scaled by {label}: V = s³(4000 − 160π), A = s²(1600 + 48π), 7 faces."),
            holed_block(s, [0.0; 3], None),
            solid(v(base_v * s.powi(3), TIGHT), v(base_a * s * s, TIGHT), Some(7), Some(1)),
        ));
    }
    for (label, d) in [
        ("1e2", 1e2),
        ("1e3", 1e3),
        ("1e4", 1e4),
        ("1e5", 1e5),
        ("1e6", 1e6),
    ] {
        let tol = if d > 1e4 { LOOSE } else { TIGHT };
        out.push(case(
            format!("gen/far/{label}"),
            "precision far from the origin",
            format!("The holed block with its corner at ({label}, −{label}, {label}) mm: V = 4000 − 160π, A = 1600 + 48π, 7 faces (graded at {tol:e}: double precision limits any kernel so far out)."),
            holed_block(1.0, [d, -d, d], None),
            solid(v(base_v, tol), v(base_a, tol), Some(7), Some(1)),
        ));
    }
    let n = 14_f64.sqrt();
    let axis = [1.0 / n, 2.0 / n, 3.0 / n];
    for deg in [15.0_f64, 30.0, 45.0, 60.0, 90.0, 137.0, 211.0] {
        out.push(case(
            format!("gen/orientation/{deg}deg"),
            "orientation",
            format!("The holed block turned {deg}° about the axis (1, 2, 3)/√14: V = 4000 − 160π, A = 1600 + 48π, 7 faces."),
            holed_block(1.0, [0.0; 3], Some((axis, deg.to_radians()))),
            solid(v(base_v, TIGHT), v(base_a, TIGHT), Some(7), Some(1)),
        ));
    }
}

fn lens(out: &mut Vec<Case>) {
    let r: f64 = 10.0;
    for d in [1.0_f64, 5.0, 10.0, 15.0, 19.0, 19.9] {
        let lv = PI * (4.0 * r + d) * (2.0 * r - d).powi(2) / 12.0;
        let la = 4.0 * PI * r * (r - d / 2.0);
        let steps = |op| {
            vec![
                make("A", sphere([0.0; 3], 2.0 * r)),
                make("B", sphere([d, 0.0, 0.0], 2.0 * r)),
                combine("A", &["B"], op, false),
            ]
        };
        out.push(case(
            format!("gen/lens/intersect-{d}"),
            "sphere-sphere",
            format!("Two R 10 spheres {d} apart, intersected: a lens of two caps of height h = R − d/2, V = π(4R + d)(2R − d)²/12, A = 2·2πRh, 2 faces."),
            steps(Intersect),
            solid(v(lv, TIGHT), v(la, TIGHT), Some(2), Some(1)),
        ));
        out.push(case(
            format!("gen/lens/unite-{d}"),
            "sphere-sphere",
            format!("Two R 10 spheres {d} apart, united: V = 2·(4/3)πR³ − lens, A = 8πR² − lens area, 2 faces."),
            steps(Unite),
            solid(v(8.0 / 3.0 * PI * r.powi(3) - lv, TIGHT), v(8.0 * PI * r * r - la, TIGHT), Some(2), Some(1)),
        ));
    }
}

fn stepped_shaft(out: &mut Vec<Case>) {
    let shafts: [&[(f64, f64)]; 4] = [
        &[(10.0, 10.0), (6.0, 8.0)],
        &[(10.0, 5.0), (7.0, 5.0), (4.0, 5.0)],
        &[(10.0, 3.0), (9.0, 3.0), (8.0, 3.0), (7.0, 3.0), (6.0, 3.0)],
        &[
            (12.0, 2.0),
            (11.0, 2.0),
            (10.0, 2.0),
            (9.0, 2.0),
            (8.0, 2.0),
            (7.0, 2.0),
            (6.0, 2.0),
            (5.0, 2.0),
        ],
    ];
    for steps_spec in shafts {
        let k = steps_spec.len();
        let mut steps = Vec::new();
        let mut z = 0.0;
        let (mut vol, mut area) = (0.0, 0.0);
        for (i, ((r, h), name)) in steps_spec.iter().zip(T.iter()).enumerate() {
            steps.push(make(
                name,
                cylinder([0.0, 0.0, z], [0.0, 0.0, 1.0], 2.0 * r, *h),
            ));
            vol += PI * r * r * h;
            area += 2.0 * PI * r * h;
            if let Some((next, _)) = steps_spec.get(i + 1) {
                area += PI * (r * r - next * next);
            }
            z += h;
        }
        let (first, last) = (
            steps_spec.first().map_or(0.0, |s| s.0),
            steps_spec.last().map_or(0.0, |s| s.0),
        );
        area += PI * first * first + PI * last * last;
        steps.push(combine("T00", T.get(1..k).unwrap_or(&[]), Unite, false));
        out.push(case(
            format!("gen/stepped-shaft/{k}"),
            "coaxial coincident end faces",
            format!("{k} coaxial cylinders of falling radius stacked end to end and united: V = Σπr²h, A = πr₁² + πr_k² + Σ2πrh + Σπ(r_i² − r_(i+1)²), {} faces.", 2 * k + 1),
            steps,
            solid(v(vol, TIGHT), v(area, TIGHT), Some(2 * k + 1), Some(1)),
        ));
    }
}

fn repeat(out: &mut Vec<Case>) {
    for n in [2_usize, 5] {
        let mut steps = vec![make("A", block([0.0; 3], [20.0, 20.0, 10.0]))];
        let mut tools = Vec::new();
        for name in T.iter().take(n) {
            steps.push(make(
                name,
                cylinder([10.0, 10.0, -1.0], [0.0, 0.0, 1.0], 8.0, 12.0),
            ));
            tools.push(*name);
        }
        steps.push(combine("A", &tools, Subtract, false));
        out.push(case(
            format!("gen/repeat/{n}"),
            "coincident cylindrical faces",
            format!("The holed block's hole drilled {n} times with identical tools: every subtract after the first meets an existing coincident bore and changes nothing: V = 4000 − 160π, A = 1600 + 48π, 7 faces."),
            steps,
            solid(v(4000.0 - 160.0 * PI, TIGHT), v(1600.0 + 48.0 * PI, TIGHT), Some(7), Some(1)),
        ));
    }
}

fn octant_quarter(out: &mut Vec<Case>) {
    for r in [1.0_f64, 10.0, 100.0] {
        out.push(case(
            format!("gen/octant/r{r}"),
            "three coordinate planes through a sphere's centre",
            format!("R {r} sphere at the corner of a 2R cube, intersected: one octant, V = πR³/6, A = πR²/2 + 3πR²/4, 4 faces."),
            vec![
                make("S", sphere([0.0; 3], 2.0 * r)),
                make("B", block([0.0; 3], [2.0 * r; 3])),
                combine("S", &["B"], Intersect, false),
            ],
            solid(v(PI * r.powi(3) / 6.0, TIGHT), v(1.25 * PI * r * r, TIGHT), Some(4), Some(1)),
        ));
    }
    for (r, h) in [(5.0_f64, 10.0_f64), (1.0, 50.0), (50.0, 1.0)] {
        out.push(case(
            format!("gen/quarter/r{r}-h{h}"),
            "two planes through a cylinder's axis",
            format!("r {r}, h {h} cylinder cut by the planes x = 0 and y = 0: a quarter, V = πr²h/4, A = πr²/2 + 2rh + πrh/2, 5 faces."),
            vec![
                make("C", cylinder([0.0; 3], [0.0, 0.0, 1.0], 2.0 * r, h)),
                make("B", block([0.0, 0.0, -1.0], [2.0 * r, 2.0 * r, h + 2.0])),
                combine("C", &["B"], Intersect, false),
            ],
            solid(v(PI * r * r * h / 4.0, TIGHT), v(PI * r * r / 2.0 + 2.0 * r * h + PI * r * h / 2.0, TIGHT), Some(5), Some(1)),
        ));
    }
}

fn cone_apex(out: &mut Vec<Case>) {
    let (big_r, big_h) = (5.0_f64, 11.0_f64);
    for (label, d) in [("1", 1.0), ("0.01", 0.01), ("1e-4", 1e-4), ("0", 0.0)] {
        // The cone rises from z = −1 with base radius 5; its apex is at
        // z = 10 − d. Inside the block (z ≥ 0) it is a smaller cone of
        // height h' = 10 − d and base radius r' = 5 h' / (11 − d).
        let h_cone = big_h - d;
        let hp = 10.0 - d;
        let rp = big_r * hp / h_cone;
        let vol = 4000.0 - PI * rp * rp * hp / 3.0;
        let area = 1600.0 - PI * rp * rp + PI * rp * (rp * rp + hp * hp).sqrt();
        out.push(case(
            format!("gen/cone-apex/{label}"),
            "apex approaching and touching a face",
            format!("20x20x10 block less a pointed cone rising from z = −1 (base r 5) whose apex is {label} mm below the top face (0: touching it): inside the block a cone of h' = 10 − {label}, r' = 5h'/(11 − {label}); V = 4000 − πr'²h'/3, A = 1600 − πr'² + πr'√(r'² + h'²)."),
            vec![
                make("A", block([0.0; 3], [20.0, 20.0, 10.0])),
                make("K", cone([10.0, 10.0, -1.0], [0.0, 0.0, 1.0], 2.0 * big_r, 0.0, h_cone)),
                combine("A", &["K"], Subtract, false),
            ],
            solid(v(vol, TIGHT), v(area, TIGHT), None, Some(1)),
        ));
    }
}

/// Every generated case, in a fixed order.
#[must_use]
pub fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    disjoint_array(&mut out);
    hole_grid(&mut out);
    coplanar_stack(&mut out);
    shifted_stack(&mut out);
    steinmetz(&mut out);
    sphere_cap(&mut out);
    cone_bore(&mut out);
    tangent_hole(&mut out);
    voids(&mut out);
    transformed(&mut out);
    lens(&mut out);
    stepped_shaft(&mut out);
    repeat(&mut out);
    octant_quarter(&mut out);
    cone_apex(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corpus is a fixed list: unique ids, every case expects a solid
    /// with a volume, and the family count together with the hand-written
    /// cases and the NIST files reaches the master prompt's 200 parts.
    #[test]
    fn families_are_unique_and_graded() {
        let cases = cases();
        let mut ids: Vec<&str> = cases.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), cases.len(), "duplicate ids");
        assert!(cases.iter().all(|c| matches!(
            c.expect,
            Expect::Solid {
                volume: Some(_),
                ..
            }
        )));
        let total = cases.len()
            + super::super::synthetic_cases().len()
            + super::super::solidcraft_cases().len()
            + 39; // NIST STEP files (tools/fetch_nist_step.py)
        assert!(total >= 200, "{total} parts");
    }

    #[test]
    fn rotation_keeps_lengths_and_turns_the_right_way() {
        let p = rotate([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], PI / 2.0);
        assert!((p[0]).abs() < 1e-15 && (p[1] - 1.0).abs() < 1e-15 && p[2].abs() < 1e-15);
        let k = [1.0 / 3_f64.sqrt(); 3];
        let q = rotate([3.0, -4.0, 12.0], k, 2.0);
        let len = |a: [f64; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        assert!((len(q) - 13.0).abs() < 1e-12);
    }
}
