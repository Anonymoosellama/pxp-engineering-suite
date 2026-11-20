// SPDX-License-Identifier: LGPL-2.1-or-later
//! Axis-aligned bounding boxes.

use crate::{Point3, Vec3};

/// An axis-aligned bounding box in mm. The empty box contains nothing and is
/// the identity for [`Aabb::union`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    /// The empty box (min = +inf, max = −inf).
    pub const EMPTY: Self = Self {
        min: Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY),
        max: Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY),
    };

    /// The smallest box containing all `points` (empty for no points).
    #[must_use]
    pub fn from_points<I: IntoIterator<Item = Point3>>(points: I) -> Self {
        points.into_iter().fold(Self::EMPTY, Self::including)
    }

    /// True if the box contains no points.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    /// This box grown to include `p`.
    #[must_use]
    pub fn including(self, p: Point3) -> Self {
        Self {
            min: self.min.min(p.to_vec()),
            max: self.max.max(p.to_vec()),
        }
    }

    /// The smallest box containing both boxes.
    #[must_use]
    pub fn union(self, o: Self) -> Self {
        Self {
            min: self.min.min(o.min),
            max: self.max.max(o.max),
        }
    }

    /// Minimum corner, or `None` if empty.
    #[must_use]
    pub fn min(&self) -> Option<Point3> {
        (!self.is_empty()).then(|| Point3::ORIGIN + self.min)
    }

    /// Maximum corner, or `None` if empty.
    #[must_use]
    pub fn max(&self) -> Option<Point3> {
        (!self.is_empty()).then(|| Point3::ORIGIN + self.max)
    }

    /// True if `p` lies inside or on the boundary.
    #[must_use]
    pub fn contains(&self, p: Point3) -> bool {
        p.x >= self.min.x
            && p.x <= self.max.x
            && p.y >= self.min.y
            && p.y <= self.max.y
            && p.z >= self.min.z
            && p.z <= self.max.z
    }

    /// Diagonal vector (max − min), or zero for an empty box.
    #[must_use]
    pub fn diagonal(&self) -> Vec3 {
        if self.is_empty() {
            Vec3::ZERO
        } else {
            self.max - self.min
        }
    }
}

// Exact float equality is intended in these tests: the inputs are small
// integers or exact binary fractions, so the results are exact in IEEE-754.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn empty_box_behaviour() {
        assert!(Aabb::EMPTY.is_empty());
        assert_eq!(Aabb::EMPTY.min(), None);
        assert_eq!(Aabb::EMPTY.diagonal(), Vec3::ZERO);
        assert!(!Aabb::EMPTY.contains(Point3::ORIGIN));
        let b = Aabb::from_points([Point3::new(1.0, 1.0, 1.0)]);
        assert_eq!(Aabb::EMPTY.union(b), b);
    }

    #[test]
    fn grows_to_fit_points() {
        let b = Aabb::from_points([Point3::new(-1.0, 2.0, 0.0), Point3::new(3.0, -2.0, 5.0)]);
        assert_eq!(b.diagonal(), Vec3::new(4.0, 4.0, 5.0));
        assert!(b.contains(Point3::new(0.0, 0.0, 2.5)));
        assert!(!b.contains(Point3::new(0.0, 0.0, 6.0)));
    }
}
