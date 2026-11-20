// SPDX-License-Identifier: LGPL-2.1-or-later
//! Rotations, rigid transforms and coordinate systems.

use crate::{Point3, UnitVec3, Vec3};
use pxp_units::Angle;
use std::fmt;

/// A 3×3 matrix stored as three **column** vectors.
///
/// For a rotation, the columns are where the X, Y and Z axes go, which makes
/// `apply` a weighted sum of columns and needs no indexing at all (clippy's
/// `indexing_slicing` is denied in library code).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat3 {
    /// First column (image of +X).
    pub cx: Vec3,
    /// Second column (image of +Y).
    pub cy: Vec3,
    /// Third column (image of +Z).
    pub cz: Vec3,
}

impl Mat3 {
    /// The identity matrix.
    pub const IDENTITY: Self = Self {
        cx: Vec3::X,
        cy: Vec3::Y,
        cz: Vec3::Z,
    };

    /// Matrix × vector.
    #[must_use]
    pub fn apply(&self, v: Vec3) -> Vec3 {
        self.cx * v.x + self.cy * v.y + self.cz * v.z
    }

    /// Matrix × matrix (`self` applied after `rhs`).
    #[must_use]
    pub fn mul(&self, rhs: &Self) -> Self {
        Self {
            cx: self.apply(rhs.cx),
            cy: self.apply(rhs.cy),
            cz: self.apply(rhs.cz),
        }
    }

    /// Transpose. For a rotation matrix this is also the inverse.
    #[must_use]
    pub fn transpose(&self) -> Self {
        Self {
            cx: Vec3::new(self.cx.x, self.cy.x, self.cz.x),
            cy: Vec3::new(self.cx.y, self.cy.y, self.cz.y),
            cz: Vec3::new(self.cx.z, self.cy.z, self.cz.z),
        }
    }

    /// Determinant (+1 for a proper rotation).
    #[must_use]
    pub fn determinant(&self) -> f64 {
        self.cx.dot(self.cy.cross(self.cz))
    }

    /// Rotation by `angle` about `axis` (right-hand rule).
    ///
    /// Method: Rodrigues' rotation formula,
    /// `R = I cosθ + (1 − cosθ) k kᵀ + sinθ [k]×`, evaluated per column.
    /// Exact up to floating-point rounding; O(1).
    #[must_use]
    pub fn rotation(axis: UnitVec3, angle: Angle) -> Self {
        let k = axis.get();
        let (s, c) = (angle.sin(), angle.cos());
        let rot = |v: Vec3| v * c + k.cross(v) * s + k * (k.dot(v) * (1.0 - c));
        Self {
            cx: rot(Vec3::X),
            cy: rot(Vec3::Y),
            cz: rot(Vec3::Z),
        }
    }
}

/// A rigid transform: rotate, then translate. Lengths are preserved, so
/// volumes and areas are too (a property test target once proptest is
/// available).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Rotation part (orthonormal, determinant +1).
    pub rotation: Mat3,
    /// Translation part, in mm.
    pub translation: Vec3,
}

impl Transform {
    /// The identity transform.
    pub const IDENTITY: Self = Self {
        rotation: Mat3::IDENTITY,
        translation: Vec3::ZERO,
    };

    /// A pure translation (mm).
    #[must_use]
    pub const fn translation(v: Vec3) -> Self {
        Self {
            rotation: Mat3::IDENTITY,
            translation: v,
        }
    }

    /// A pure rotation about an axis through the origin.
    #[must_use]
    pub fn rotation(axis: UnitVec3, angle: Angle) -> Self {
        Self {
            rotation: Mat3::rotation(axis, angle),
            translation: Vec3::ZERO,
        }
    }

    /// Transforms a point (rotation and translation).
    #[must_use]
    pub fn apply_point(&self, p: Point3) -> Point3 {
        Point3::ORIGIN + (self.rotation.apply(p.to_vec()) + self.translation)
    }

    /// Transforms a vector (rotation only: vectors have no position).
    #[must_use]
    pub fn apply_vector(&self, v: Vec3) -> Vec3 {
        self.rotation.apply(v)
    }

    /// `self ∘ rhs`: apply `rhs` first, then `self`.
    #[must_use]
    pub fn then_after(&self, rhs: &Self) -> Self {
        Self {
            rotation: self.rotation.mul(&rhs.rotation),
            translation: self.rotation.apply(rhs.translation) + self.translation,
        }
    }

    /// The inverse transform. For a rigid transform `x' = R x + t`, the
    /// inverse is `x = Rᵀ x' − Rᵀ t`; exact for orthonormal `R`.
    #[must_use]
    pub fn inverse(&self) -> Self {
        let rt = self.rotation.transpose();
        Self {
            rotation: rt,
            translation: -rt.apply(self.translation),
        }
    }
}

/// Errors building a coordinate system.
#[derive(Debug, Clone, PartialEq)]
pub enum CsysError {
    /// The X direction is zero-length or not finite.
    DegenerateX,
    /// The Y direction is zero-length, not finite, or parallel to X, so no
    /// plane is defined.
    DegenerateY,
    /// The origin is not finite.
    NonFiniteOrigin,
}

impl fmt::Display for CsysError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DegenerateX => "the X direction is zero-length or not finite",
            Self::DegenerateY => "the Y direction is zero-length or parallel to X",
            Self::NonFiniteOrigin => "the origin is not finite",
        })
    }
}

impl std::error::Error for CsysError {}

/// A right-handed orthonormal coordinate system: the basis of the WCS, of
/// datum coordinate systems, of sketch planes and of CAM MCS objects.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Csys {
    origin: Point3,
    x: UnitVec3,
    y: UnitVec3,
    z: UnitVec3,
}

impl Csys {
    /// The absolute coordinate system (origin, +X, +Y, +Z).
    pub const ABSOLUTE: Self = Self {
        origin: Point3::ORIGIN,
        x: UnitVec3::X,
        y: UnitVec3::Y,
        z: UnitVec3::Z,
    };

    /// Builds a coordinate system from an origin, an X direction and a vector
    /// that, together with X, defines the XY plane.
    ///
    /// Method: Gram-Schmidt. `z = normalise(x × y_hint)`, then
    /// `y = z × x`, so the result is exactly orthonormal and right-handed
    /// even when `y_hint` is not perpendicular to `x`.
    ///
    /// # Errors
    /// [`CsysError`] when the inputs do not define a plane.
    pub fn new(origin: Point3, x_dir: Vec3, y_hint: Vec3) -> Result<Self, CsysError> {
        if !origin.is_finite() {
            return Err(CsysError::NonFiniteOrigin);
        }
        let x = x_dir.normalized().ok_or(CsysError::DegenerateX)?;
        if !y_hint.is_finite() {
            return Err(CsysError::DegenerateY);
        }
        // Reject a y_hint (nearly) parallel to x: |sin θ| below 1e-9.
        let z_raw = x.get().cross(y_hint);
        if z_raw.length() <= 1e-9 * y_hint.length() {
            return Err(CsysError::DegenerateY);
        }
        let z = z_raw.normalized().ok_or(CsysError::DegenerateY)?;
        let y = z
            .get()
            .cross(x.get())
            .normalized()
            .ok_or(CsysError::DegenerateY)?;
        Ok(Self { origin, x, y, z })
    }

    /// Origin (mm).
    #[must_use]
    pub const fn origin(&self) -> Point3 {
        self.origin
    }
    /// X axis.
    #[must_use]
    pub const fn x_axis(&self) -> UnitVec3 {
        self.x
    }
    /// Y axis.
    #[must_use]
    pub const fn y_axis(&self) -> UnitVec3 {
        self.y
    }
    /// Z axis (normal of the XY plane).
    #[must_use]
    pub const fn z_axis(&self) -> UnitVec3 {
        self.z
    }

    /// The transform from this system's local coordinates to absolute
    /// coordinates.
    #[must_use]
    pub fn to_absolute(&self) -> Transform {
        Transform {
            rotation: Mat3 {
                cx: self.x.get(),
                cy: self.y.get(),
                cz: self.z.get(),
            },
            translation: self.origin.to_vec(),
        }
    }

    /// The transform from absolute coordinates to this system's local ones.
    #[must_use]
    pub fn from_absolute(&self) -> Transform {
        self.to_absolute().inverse()
    }
}

// Exact float equality is intended in these tests: the inputs are small
// integers or exact binary fractions, so the results are exact in IEEE-754.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn near(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-12
    }

    #[test]
    fn rotation_about_z_by_90() {
        let r = Mat3::rotation(UnitVec3::Z, Angle::from_deg(90.0));
        assert!(near(r.apply(Vec3::X), Vec3::Y));
        assert!((r.determinant() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn inverse_undoes_transform() {
        let axis = Vec3::new(1.0, 1.0, 1.0).normalized().unwrap();
        let t = Transform::translation(Vec3::new(10.0, -5.0, 2.0))
            .then_after(&Transform::rotation(axis, Angle::from_deg(37.0)));
        let p = Point3::new(3.0, 4.0, 5.0);
        let back = t.inverse().apply_point(t.apply_point(p));
        assert!(near(back - Point3::ORIGIN, p - Point3::ORIGIN));
    }

    #[test]
    fn rigid_transform_preserves_distance() {
        let axis = Vec3::new(0.2, -1.0, 0.4).normalized().unwrap();
        let t = Transform {
            rotation: Mat3::rotation(axis, Angle::from_deg(123.0)),
            translation: Vec3::new(7.0, 8.0, 9.0),
        };
        let (a, b) = (Point3::new(1.0, 2.0, 3.0), Point3::new(-4.0, 0.5, 9.0));
        let d0 = a.distance(b).mm();
        let d1 = t.apply_point(a).distance(t.apply_point(b)).mm();
        assert!((d0 - d1).abs() < 1e-12);
    }

    #[test]
    fn csys_orthonormalizes_hint() {
        let c = Csys::new(
            Point3::new(1.0, 2.0, 3.0),
            Vec3::X,
            Vec3::new(1.0, 1.0, 0.0),
        )
        .unwrap();
        assert!(near(c.y_axis().get(), Vec3::Y));
        assert!(near(c.z_axis().get(), Vec3::Z));
        let local = c.from_absolute().apply_point(Point3::new(1.0, 3.0, 3.0));
        assert!(near(local.to_vec(), Vec3::Y));
    }

    #[test]
    fn csys_rejects_degenerate_input() {
        assert_eq!(
            Csys::new(Point3::ORIGIN, Vec3::ZERO, Vec3::Y),
            Err(CsysError::DegenerateX)
        );
        assert_eq!(
            Csys::new(Point3::ORIGIN, Vec3::X, Vec3::X * 2.0),
            Err(CsysError::DegenerateY)
        );
        let nan = Point3::new(f64::NAN, 0.0, 0.0);
        assert_eq!(
            Csys::new(nan, Vec3::X, Vec3::Y),
            Err(CsysError::NonFiniteOrigin)
        );
    }
}
