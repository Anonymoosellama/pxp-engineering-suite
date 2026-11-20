// SPDX-License-Identifier: LGPL-2.1-or-later
//! Physical dimensions as integer exponents of the base dimensions.

use std::fmt;

/// A physical dimension, written as exponents of the four base dimensions
/// used by PXP: length (L), mass (M), time (T) and angle (A).
///
/// For example force is `L¹ M¹ T⁻²` and stress is `L⁻¹ M¹ T⁻²`. Multiplying
/// two quantities adds their exponents; dividing subtracts them. Exponents
/// are `i8`: no physical quantity in this program needs more than ±127, and
/// overflow is reported as an error by [`Dim::checked_mul`] and friends.
///
/// Angle is a base dimension here on purpose (see the crate docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Dim {
    /// Exponent of length.
    pub length: i8,
    /// Exponent of mass.
    pub mass: i8,
    /// Exponent of time.
    pub time: i8,
    /// Exponent of angle.
    pub angle: i8,
}

impl Dim {
    /// A pure number (all exponents zero).
    pub const DIMENSIONLESS: Self = Self::new(0, 0, 0, 0);
    /// Length (mm internally).
    pub const LENGTH: Self = Self::new(1, 0, 0, 0);
    /// Area (mm²).
    pub const AREA: Self = Self::new(2, 0, 0, 0);
    /// Volume (mm³).
    pub const VOLUME: Self = Self::new(3, 0, 0, 0);
    /// Mass (tonne).
    pub const MASS: Self = Self::new(0, 1, 0, 0);
    /// Time (s).
    pub const TIME: Self = Self::new(0, 0, 1, 0);
    /// Plane angle (rad).
    pub const ANGLE: Self = Self::new(0, 0, 0, 1);
    /// Frequency (1/s, Hz).
    pub const FREQUENCY: Self = Self::new(0, 0, -1, 0);
    /// Velocity (mm/s).
    pub const VELOCITY: Self = Self::new(1, 0, -1, 0);
    /// Acceleration (mm/s²).
    pub const ACCELERATION: Self = Self::new(1, 0, -2, 0);
    /// Force (N = t·mm/s²).
    pub const FORCE: Self = Self::new(1, 1, -2, 0);
    /// Stress and pressure (MPa = N/mm²).
    pub const STRESS: Self = Self::new(-1, 1, -2, 0);
    /// Energy (mJ = N·mm).
    pub const ENERGY: Self = Self::new(2, 1, -2, 0);
    /// Density (t/mm³).
    pub const DENSITY: Self = Self::new(-3, 1, 0, 0);

    /// Builds a dimension from its four exponents (length, mass, time, angle).
    #[must_use]
    pub const fn new(length: i8, mass: i8, time: i8, angle: i8) -> Self {
        Self {
            length,
            mass,
            time,
            angle,
        }
    }

    /// True if every exponent is zero.
    #[must_use]
    pub const fn is_dimensionless(self) -> bool {
        self.length == 0 && self.mass == 0 && self.time == 0 && self.angle == 0
    }

    /// Dimension of a product: exponents add. `None` on `i8` overflow.
    #[must_use]
    pub fn checked_mul(self, rhs: Self) -> Option<Self> {
        Some(Self {
            length: self.length.checked_add(rhs.length)?,
            mass: self.mass.checked_add(rhs.mass)?,
            time: self.time.checked_add(rhs.time)?,
            angle: self.angle.checked_add(rhs.angle)?,
        })
    }

    /// Dimension of a quotient: exponents subtract. `None` on `i8` overflow.
    #[must_use]
    pub fn checked_div(self, rhs: Self) -> Option<Self> {
        Some(Self {
            length: self.length.checked_sub(rhs.length)?,
            mass: self.mass.checked_sub(rhs.mass)?,
            time: self.time.checked_sub(rhs.time)?,
            angle: self.angle.checked_sub(rhs.angle)?,
        })
    }

    /// Dimension of an integer power: exponents multiply. `None` on overflow.
    #[must_use]
    pub fn checked_powi(self, n: i8) -> Option<Self> {
        Some(Self {
            length: self.length.checked_mul(n)?,
            mass: self.mass.checked_mul(n)?,
            time: self.time.checked_mul(n)?,
            angle: self.angle.checked_mul(n)?,
        })
    }

    /// Dimension of an n-th root, or `None` if any exponent is not divisible
    /// by `n` (the square root of a length has no physical meaning) or `n` is
    /// zero.
    #[must_use]
    pub fn checked_root(self, n: i8) -> Option<Self> {
        if n == 0 {
            return None;
        }
        let div = |e: i8| if e % n == 0 { Some(e / n) } else { None };
        Some(Self {
            length: div(self.length)?,
            mass: div(self.mass)?,
            time: div(self.time)?,
            angle: div(self.angle)?,
        })
    }

    /// A human-readable name for well-known dimensions, used in error
    /// messages ("expected length, found angle").
    #[must_use]
    pub fn name(self) -> Option<&'static str> {
        let known: [(Self, &'static str); 14] = [
            (Self::DIMENSIONLESS, "dimensionless"),
            (Self::LENGTH, "length"),
            (Self::AREA, "area"),
            (Self::VOLUME, "volume"),
            (Self::MASS, "mass"),
            (Self::TIME, "time"),
            (Self::ANGLE, "angle"),
            (Self::FREQUENCY, "frequency"),
            (Self::VELOCITY, "velocity"),
            (Self::ACCELERATION, "acceleration"),
            (Self::FORCE, "force"),
            (Self::STRESS, "stress"),
            (Self::ENERGY, "energy"),
            (Self::DENSITY, "density"),
        ];
        known.iter().find(|(d, _)| *d == self).map(|(_, n)| *n)
    }
}

impl fmt::Display for Dim {
    /// Prints the well-known name when there is one, otherwise the exponent
    /// form, e.g. `L^2 M^1 T^-3`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(name) = self.name() {
            return f.write_str(name);
        }
        let mut parts = Vec::new();
        for (sym, e) in [
            ("L", self.length),
            ("M", self.mass),
            ("T", self.time),
            ("A", self.angle),
        ] {
            if e != 0 {
                parts.push(format!("{sym}^{e}"));
            }
        }
        f.write_str(&parts.join(" "))
    }
}

// Exact float equality is intended in these tests: the inputs are small
// integers or exact binary fractions, so the results are exact in IEEE-754.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn derived_dimensions_are_consistent() {
        // Force / Area = Stress
        assert_eq!(Dim::FORCE.checked_div(Dim::AREA), Some(Dim::STRESS));
        // Force * Length = Energy
        assert_eq!(Dim::FORCE.checked_mul(Dim::LENGTH), Some(Dim::ENERGY));
        // Mass / Volume = Density
        assert_eq!(Dim::MASS.checked_div(Dim::VOLUME), Some(Dim::DENSITY));
        // Mass * Acceleration = Force
        assert_eq!(Dim::MASS.checked_mul(Dim::ACCELERATION), Some(Dim::FORCE));
    }

    #[test]
    fn roots_require_divisible_exponents() {
        assert_eq!(Dim::AREA.checked_root(2), Some(Dim::LENGTH));
        assert_eq!(Dim::LENGTH.checked_root(2), None);
        assert_eq!(Dim::VOLUME.checked_root(3), Some(Dim::LENGTH));
        assert_eq!(Dim::LENGTH.checked_root(0), None);
    }

    #[test]
    fn overflow_is_reported_not_wrapped() {
        let big = Dim::new(100, 0, 0, 0);
        assert_eq!(big.checked_mul(big), None);
        assert_eq!(big.checked_powi(2), None);
    }

    #[test]
    fn display_uses_names_then_exponents() {
        assert_eq!(Dim::STRESS.to_string(), "stress");
        assert_eq!(Dim::new(2, 1, -3, 0).to_string(), "L^2 M^1 T^-3");
    }
}
