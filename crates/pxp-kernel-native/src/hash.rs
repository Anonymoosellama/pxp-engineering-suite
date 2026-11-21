// SPDX-License-Identifier: LGPL-2.1-or-later
//! The platform-independent geometry hash of a SolidCraft body.
//!
//! ## What is hashed, in order
//!
//! 1. A format tag (`pxp.solidcraft.brep-hash.v1`), so a change to this
//!    recipe changes every hash rather than colliding with old ones.
//! 2. Entity counts: vertices, edges, faces, shells.
//! 3. Every vertex position, quantised to `resolution`.
//! 4. Every edge: its curve kind and its start and end vertex indices.
//! 5. Every face: its surface kind and its loops (edge indices with their
//!    direction, or the vertex of a vertex loop).
//! 6. Every shell: its oriented faces, wire edges or vertex.
//! 7. Volume and area from SolidCraft's mass measure, rounded to 9
//!    significant digits (`StableHasher::write_f64_significant`), and the
//!    centroid quantised to `resolution`. This catches shape changes that
//!    keep every vertex in place (a bulged spline face). Volume and area use
//!    significant digits rather than a resolution derived from the body's
//!    size, because such a resolution would itself carry last-bit noise
//!    (a block typed as 3 in and as 76.2 mm must hash alike).
//!
//! Topology order is part of the hash on purpose: the determinism rule says
//! identical inputs give identical topology order, and persistent naming
//! relies on it.
//!
//! ## Why quantise
//!
//! Values that pass through `sin`, `cos` or `sqrt` may differ in the last
//! bit between platform maths libraries. Rounding to a multiple of
//! `resolution` (callers pass a micro-millimetre-scale value) makes the hash
//! equal across operating systems except for values that sit almost exactly
//! on a rounding boundary, a residual risk the cross-OS CI comparison
//! exists to catch.

use pxp_kernel::KernelError;
use pxp_math::StableHasher;
use solidcore_record::brep::{Curve, Loop, Shell, Surface};
use solidcraft_kernel::Body;

const FORMAT_TAG: &str = "pxp.solidcraft.brep-hash.v1";

fn curve_tag(c: &Curve) -> &'static str {
    match c {
        Curve::Line { .. } => "line",
        Curve::Circle { .. } => "circle",
        Curve::Ellipse { .. } => "ellipse",
        Curve::Polyline { .. } => "polyline",
        Curve::Spline { .. } => "spline",
        Curve::RationalSpline { .. } => "rational_spline",
        Curve::SurfaceIntersection { .. } => "surface_intersection",
    }
}

fn surface_tag(s: &Surface) -> &'static str {
    match s {
        Surface::Plane { .. } | Surface::PlaneThrough { .. } => "plane",
        Surface::Cylinder { .. } => "cylinder",
        Surface::Cone { .. } => "cone",
        Surface::Sphere { .. } => "sphere",
        Surface::Torus { .. } => "torus",
        Surface::BallPatch { .. } => "ball_patch",
        Surface::Spline { .. } => "spline",
        Surface::RationalSpline { .. } => "rational_spline",
        Surface::PlacedRevolution { .. } => "revolution",
        Surface::Blend { .. } => "blend",
    }
}

fn count(h: &mut StableHasher, n: usize) {
    h.write_u64(n as u64);
}

/// Computes the hash described in the module documentation.
///
/// # Errors
/// [`KernelError::GeometryFailure`] if SolidCraft cannot measure the body.
pub(crate) fn geometry_hash(body: &Body, resolution: f64) -> Result<u64, KernelError> {
    let rec = body.record();
    let mut h = StableHasher::new();
    h.write_str(FORMAT_TAG);
    count(&mut h, rec.vertices.len());
    count(&mut h, rec.edges.len());
    count(&mut h, rec.faces.len());
    count(&mut h, rec.shells.len());

    for v in &rec.vertices {
        for &c in &v.point {
            h.write_f64_quantized(c, resolution);
        }
    }

    for e in &rec.edges {
        h.write_str(curve_tag(&e.curve));
        h.write_u64(u64::from(e.vertices[0]));
        h.write_u64(u64::from(e.vertices[1]));
    }
    for f in &rec.faces {
        h.write_str(surface_tag(&f.surface));
        count(&mut h, f.loops.len());
        for l in &f.loops {
            match l {
                Loop::Edges { edges } => {
                    h.write_u64(1);
                    count(&mut h, edges.len());
                    for oe in edges {
                        h.write_u64(u64::from(oe.edge));
                        h.write_u64(u64::from(oe.forward));
                    }
                }
                Loop::Vertex { vertex } => {
                    h.write_u64(2);
                    h.write_u64(u64::from(*vertex));
                }
            }
        }
    }
    for s in &rec.shells {
        match s {
            Shell::Faces { faces, wires } => {
                h.write_u64(1);
                count(&mut h, faces.len());
                for of in faces {
                    h.write_u64(u64::from(of.face));
                    h.write_u64(u64::from(of.forward));
                }
                count(&mut h, wires.len());
                for w in wires {
                    h.write_u64(u64::from(*w));
                }
            }
            Shell::Wire { edges } => {
                h.write_u64(2);
                count(&mut h, edges.len());
                for e in edges {
                    h.write_u64(u64::from(*e));
                }
            }
            Shell::Vertex { vertex } => {
                h.write_u64(3);
                h.write_u64(u64::from(*vertex));
            }
        }
    }

    let m = solidcraft_kernel::measure_mass(body).map_err(|e| KernelError::GeometryFailure {
        op: "geometry_hash",
        reason: format!("mass measure failed: {e}"),
        entities: Vec::new(),
    })?;
    h.write_f64_significant(m.volume, 9);
    h.write_f64_significant(m.area, 9);
    for c in [m.centroid.x, m.centroid.y, m.centroid.z] {
        h.write_f64_quantized(c, resolution);
    }
    Ok(h.finish())
}
