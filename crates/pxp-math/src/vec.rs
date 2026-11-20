// SPDX-License-Identifier: LGPL-2.1-or-later
//! Vectors, unit vectors and points in 3D (millimetres).

use pxp_units::Length;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// A 3D displacement or direction, components in millimetres when it is a
/// displacement (directions are unit-less; see [`UnitVec3`]).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    /// X component.
    pub x: f64,
    /// Y component.
    pub y: f64,
    /// Z component.
    pub z: f64,
}

impl Vec3 {
    /// The zero vector.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    /// +X.
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    /// +Y.
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    /// +Z.
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    /// Builds a vector from components.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Dot product.
    #[must_use]
    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Cross product (right-handed).
    #[must_use]
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    /// Squared Euclidean length (cheaper than [`Vec3::length`]).
    #[must_use]
    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    /// Euclidean length.
    #[must_use]
    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    /// The unit vector in this direction, or `None` if the vector is shorter
    /// than `1e-12` (or not finite), where a direction is meaningless.
    #[must_use]
    pub fn normalized(self) -> Option<UnitVec3> {
        let len = self.length();
        if len.is_finite() && len > 1e-12 {
            Some(UnitVec3(self / len))
        } else {
            None
        }
    }

    /// True if every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    /// Component-wise minimum.
    #[must_use]
    pub fn min(self, o: Self) -> Self {
        Self::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    /// Component-wise maximum.
    #[must_use]
    pub fn max(self, o: Self) -> Self {
        Self::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Self) {
        *self = *self + o;
    }
}
impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Self) {
        *self = *self - o;
    }
}
impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}
impl Mul<f64> for Vec3 {
    type Output = Self;
    fn mul(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Mul<Vec3> for f64 {
    type Output = Vec3;
    fn mul(self, v: Vec3) -> Vec3 {
        v * self
    }
}
impl Div<f64> for Vec3 {
    type Output = Self;
    fn div(self, s: f64) -> Self {
        Self::new(self.x / s, self.y / s, self.z / s)
    }
}

/// A vector of length 1 (within 1e-12). Directions, axes and normals use this
/// type so "is it normalised?" never has to be asked twice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitVec3(Vec3);

impl UnitVec3 {
    /// +X.
    pub const X: Self = Self(Vec3::X);
    /// +Y.
    pub const Y: Self = Self(Vec3::Y);
    /// +Z.
    pub const Z: Self = Self(Vec3::Z);

    /// The underlying vector.
    #[must_use]
    pub const fn get(self) -> Vec3 {
        self.0
    }

    /// The opposite direction.
    #[must_use]
    pub fn reversed(self) -> Self {
        Self(-self.0)
    }

    /// Dot product with another direction (the cosine of the angle between
    /// them).
    #[must_use]
    pub fn dot(self, o: Self) -> f64 {
        self.0.dot(o.0)
    }
}

/// A position in 3D, coordinates in millimetres.
///
/// Points and vectors are different types on purpose: `point + point` is
/// meaningless and does not compile, while `point - point` is a [`Vec3`].
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point3 {
    /// X coordinate (mm).
    pub x: f64,
    /// Y coordinate (mm).
    pub y: f64,
    /// Z coordinate (mm).
    pub z: f64,
}

impl Point3 {
    /// The origin.
    pub const ORIGIN: Self = Self::new(0.0, 0.0, 0.0);

    /// Builds a point from coordinates in mm.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    /// Builds a point from typed lengths (any display unit converts here).
    #[must_use]
    pub fn from_lengths(x: Length, y: Length, z: Length) -> Self {
        Self::new(x.mm(), y.mm(), z.mm())
    }

    /// The vector from the origin to this point.
    #[must_use]
    pub const fn to_vec(self) -> Vec3 {
        Vec3::new(self.x, self.y, self.z)
    }

    /// Distance to another point.
    #[must_use]
    pub fn distance(self, o: Self) -> Length {
        Length::from_mm((self - o).length())
    }

    /// True if every coordinate is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.to_vec().is_finite()
    }
}

impl Sub for Point3 {
    type Output = Vec3;
    fn sub(self, o: Self) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Add<Vec3> for Point3 {
    type Output = Self;
    fn add(self, v: Vec3) -> Self {
        Self::new(self.x + v.x, self.y + v.y, self.z + v.z)
    }
}
impl Sub<Vec3> for Point3 {
    type Output = Self;
    fn sub(self, v: Vec3) -> Self {
        Self::new(self.x - v.x, self.y - v.y, self.z - v.z)
    }
}

// Exact float equality is intended in these tests: the inputs are small
// integers or exact binary fractions, so the results are exact in IEEE-754.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn cross_is_right_handed() {
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(Vec3::Y.cross(Vec3::Z), Vec3::X);
        assert_eq!(Vec3::Z.cross(Vec3::X), Vec3::Y);
    }

    #[test]
    fn normalizing_zero_is_none_not_nan() {
        assert!(Vec3::ZERO.normalized().is_none());
        assert!(Vec3::new(f64::NAN, 0.0, 0.0).normalized().is_none());
        let u = Vec3::new(3.0, 4.0, 0.0).normalized().unwrap();
        assert!((u.get().length() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn point_vector_algebra() {
        let a = Point3::new(1.0, 2.0, 3.0);
        let b = Point3::new(4.0, 6.0, 3.0);
        assert_eq!(b - a, Vec3::new(3.0, 4.0, 0.0));
        assert_eq!(a + (b - a), b);
        assert!((a.distance(b).mm() - 5.0).abs() < 1e-12);
    }

    #[test]
    fn from_lengths_converts_units() {
        let p = Point3::from_lengths(Length::from_inch(1.0), Length::ZERO, Length::from_mm(2.0));
        assert!((p.x - 25.4).abs() < 1e-12);
        assert!((p.z - 2.0).abs() < 1e-12);
    }
}
