// SPDX-License-Identifier: LGPL-2.1-or-later
//! Compile-time typed quantities.
//!
//! Each newtype wraps one `f64` in internal units (see the crate docs). The
//! field is private; values enter only through constructors that name their
//! unit, and leave only through accessors that name theirs.

use crate::{Dim, Quantity, UnitError};
use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

/// Generates a quantity newtype with same-type arithmetic, scaling by `f64`,
/// conversion to and from [`Quantity`], and a `Display` in internal units.
macro_rules! quantity_newtype {
    ($(#[$doc:meta])* $name:ident, $dim:expr, $internal_symbol:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
        pub struct $name(f64);

        impl $name {
            /// Zero of this quantity.
            pub const ZERO: Self = Self(0.0);
            /// The physical dimension of this type.
            pub const DIM: Dim = $dim;

            /// Builds a value from a number already in this type's internal
            #[doc = concat!("unit (`", $internal_symbol, "`).")]
            /// Prefer the named constructors (`from_mm`, `from_deg`, ...).
            #[must_use]
            pub const fn from_internal(value: f64) -> Self {
                Self(value)
            }

            #[doc = concat!("The value in the internal unit (`", $internal_symbol, "`).")]
            #[must_use]
            pub const fn internal(self) -> f64 {
                self.0
            }

            /// Absolute value.
            #[must_use]
            pub fn abs(self) -> Self {
                Self(self.0.abs())
            }

            /// The smaller of two values (NaN-propagating like `f64::min`).
            #[must_use]
            pub fn min(self, other: Self) -> Self {
                Self(self.0.min(other.0))
            }

            /// The larger of two values.
            #[must_use]
            pub fn max(self, other: Self) -> Self {
                Self(self.0.max(other.0))
            }

            /// True unless NaN or infinite.
            #[must_use]
            pub fn is_finite(self) -> bool {
                self.0.is_finite()
            }

            /// True if `|self - other| <= tolerance`.
            #[must_use]
            pub fn approx_eq(self, other: Self, tolerance: Self) -> bool {
                (self.0 - other.0).abs() <= tolerance.0
            }
        }

        impl Add for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self(self.0 + rhs.0)
            }
        }
        impl Sub for $name {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                Self(self.0 - rhs.0)
            }
        }
        impl AddAssign for $name {
            fn add_assign(&mut self, rhs: Self) {
                self.0 += rhs.0;
            }
        }
        impl SubAssign for $name {
            fn sub_assign(&mut self, rhs: Self) {
                self.0 -= rhs.0;
            }
        }
        impl Neg for $name {
            type Output = Self;
            fn neg(self) -> Self {
                Self(-self.0)
            }
        }
        impl Mul<f64> for $name {
            type Output = Self;
            fn mul(self, rhs: f64) -> Self {
                Self(self.0 * rhs)
            }
        }
        impl Mul<$name> for f64 {
            type Output = $name;
            fn mul(self, rhs: $name) -> $name {
                $name(self * rhs.0)
            }
        }
        impl Div<f64> for $name {
            type Output = Self;
            fn div(self, rhs: f64) -> Self {
                Self(self.0 / rhs)
            }
        }
        /// Ratio of two values of the same quantity is a pure number.
        impl Div for $name {
            type Output = f64;
            fn div(self, rhs: Self) -> f64 {
                self.0 / rhs.0
            }
        }
        impl Sum for $name {
            fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
                Self(iter.map(|v| v.0).sum())
            }
        }
        impl From<$name> for Quantity {
            fn from(v: $name) -> Quantity {
                Quantity::from_internal(v.0, $dim)
            }
        }
        impl TryFrom<Quantity> for $name {
            type Error = UnitError;
            /// Fails with [`UnitError::DimensionMismatch`] if `q` has a
            /// different dimension.
            fn try_from(q: Quantity) -> Result<Self, UnitError> {
                Ok(Self(q.expect_dim($dim)?.value()))
            }
        }
        impl fmt::Display for $name {
            /// Internal units; honours `{:.3}`-style precision. User-facing
            /// text should go through [`crate::UnitSystem::format`].
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match f.precision() {
                    Some(p) => write!(f, "{:.*} {}", p, self.0, $internal_symbol),
                    None => write!(f, "{} {}", self.0, $internal_symbol),
                }
            }
        }
    };
}

/// Generates `impl Mul/Div` between two different quantity types.
macro_rules! cross_op {
    ($lhs:ident * $rhs:ident = $out:ident) => {
        impl Mul<$rhs> for $lhs {
            type Output = $out;
            fn mul(self, rhs: $rhs) -> $out {
                $out(self.0 * rhs.0)
            }
        }
    };
    ($lhs:ident / $rhs:ident = $out:ident) => {
        impl Div<$rhs> for $lhs {
            type Output = $out;
            fn div(self, rhs: $rhs) -> $out {
                $out(self.0 / rhs.0)
            }
        }
    };
}

quantity_newtype!(
    /// A length. Internal unit: millimetre (f64).
    ///
    /// ```
    /// use pxp_units::Length;
    /// let a = Length::from_inch(1.0);
    /// let b = Length::from_mm(25.4);
    /// assert!(a.approx_eq(b, Length::from_mm(1e-12)));
    /// ```
    Length,
    Dim::LENGTH,
    "mm"
);
quantity_newtype!(
    /// An area. Internal unit: mm².
    Area,
    Dim::AREA,
    "mm^2"
);
quantity_newtype!(
    /// A volume. Internal unit: mm³.
    Volume,
    Dim::VOLUME,
    "mm^3"
);
quantity_newtype!(
    /// A plane angle. Internal unit: radian.
    Angle,
    Dim::ANGLE,
    "rad"
);
quantity_newtype!(
    /// A mass. Internal unit: tonne (coherent with mm, s and N).
    Mass,
    Dim::MASS,
    "t"
);
quantity_newtype!(
    /// A duration. Internal unit: second.
    Time,
    Dim::TIME,
    "s"
);
quantity_newtype!(
    /// A force. Internal unit: newton (= t·mm/s²).
    Force,
    Dim::FORCE,
    "N"
);
quantity_newtype!(
    /// A stress or pressure. Internal unit: megapascal (= N/mm²).
    Stress,
    Dim::STRESS,
    "MPa"
);
quantity_newtype!(
    /// A mass density. Internal unit: t/mm³ (steel ≈ 7.85e-9).
    Density,
    Dim::DENSITY,
    "t/mm^3"
);
quantity_newtype!(
    /// An energy (or a moment/torque, which has the same dimension).
    /// Internal unit: N·mm (= millijoule).
    Energy,
    Dim::ENERGY,
    "N*mm"
);

cross_op!(Length * Length = Area);
cross_op!(Length * Area = Volume);
cross_op!(Area * Length = Volume);
cross_op!(Area / Length = Length);
cross_op!(Volume / Length = Area);
cross_op!(Volume / Area = Length);
cross_op!(Force / Area = Stress);
cross_op!(Stress * Area = Force);
cross_op!(Area * Stress = Force);
cross_op!(Density * Volume = Mass);
cross_op!(Volume * Density = Mass);
cross_op!(Mass / Volume = Density);
cross_op!(Force * Length = Energy);
cross_op!(Length * Force = Energy);

const INCH_MM: f64 = 25.4;

impl Length {
    /// From millimetres.
    #[must_use]
    pub const fn from_mm(mm: f64) -> Self {
        Self(mm)
    }
    /// From inches (exactly 25.4 mm each).
    #[must_use]
    pub fn from_inch(inch: f64) -> Self {
        Self(inch * INCH_MM)
    }
    /// From metres.
    #[must_use]
    pub fn from_m(m: f64) -> Self {
        Self(m * 1000.0)
    }
    /// In millimetres.
    #[must_use]
    pub const fn mm(self) -> f64 {
        self.0
    }
    /// In inches.
    #[must_use]
    pub fn inch(self) -> f64 {
        self.0 / INCH_MM
    }
    /// In metres.
    #[must_use]
    pub fn m(self) -> f64 {
        self.0 / 1000.0
    }
}

impl Area {
    /// From mm².
    #[must_use]
    pub const fn from_mm2(mm2: f64) -> Self {
        Self(mm2)
    }
    /// In mm².
    #[must_use]
    pub const fn mm2(self) -> f64 {
        self.0
    }
}

impl Volume {
    /// From mm³.
    #[must_use]
    pub const fn from_mm3(mm3: f64) -> Self {
        Self(mm3)
    }
    /// In mm³.
    #[must_use]
    pub const fn mm3(self) -> f64 {
        self.0
    }
}

impl Angle {
    /// From radians.
    #[must_use]
    pub const fn from_rad(rad: f64) -> Self {
        Self(rad)
    }
    /// From degrees.
    #[must_use]
    pub fn from_deg(deg: f64) -> Self {
        Self(deg.to_radians())
    }
    /// In radians.
    #[must_use]
    pub const fn rad(self) -> f64 {
        self.0
    }
    /// In degrees.
    #[must_use]
    pub fn deg(self) -> f64 {
        self.0.to_degrees()
    }
    /// Sine.
    #[must_use]
    pub fn sin(self) -> f64 {
        self.0.sin()
    }
    /// Cosine.
    #[must_use]
    pub fn cos(self) -> f64 {
        self.0.cos()
    }
    /// Tangent.
    #[must_use]
    pub fn tan(self) -> f64 {
        self.0.tan()
    }
    /// The equivalent angle in `[0, 2π)`.
    #[must_use]
    pub fn normalized(self) -> Self {
        Self(self.0.rem_euclid(std::f64::consts::TAU))
    }
}

impl Mass {
    /// From kilograms.
    #[must_use]
    pub fn from_kg(kg: f64) -> Self {
        Self(kg * 1.0e-3)
    }
    /// From tonnes (the internal unit).
    #[must_use]
    pub const fn from_tonne(t: f64) -> Self {
        Self(t)
    }
    /// From avoirdupois pounds (exactly 0.453 592 37 kg).
    #[must_use]
    pub fn from_lb(lb: f64) -> Self {
        Self(lb * 0.453_592_37e-3)
    }
    /// In kilograms.
    #[must_use]
    pub fn kg(self) -> f64 {
        self.0 * 1.0e3
    }
    /// In tonnes.
    #[must_use]
    pub const fn tonne(self) -> f64 {
        self.0
    }
    /// In pounds.
    #[must_use]
    pub fn lb(self) -> f64 {
        self.0 / 0.453_592_37e-3
    }
}

impl Time {
    /// From seconds.
    #[must_use]
    pub const fn from_s(s: f64) -> Self {
        Self(s)
    }
    /// In seconds.
    #[must_use]
    pub const fn s(self) -> f64 {
        self.0
    }
}

impl Force {
    /// From newtons.
    #[must_use]
    pub const fn from_n(n: f64) -> Self {
        Self(n)
    }
    /// From pounds-force (exactly 4.448 221 615 260 5 N).
    #[must_use]
    pub fn from_lbf(lbf: f64) -> Self {
        Self(lbf * 4.448_221_615_260_5)
    }
    /// In newtons.
    #[must_use]
    pub const fn n(self) -> f64 {
        self.0
    }
    /// In pounds-force.
    #[must_use]
    pub fn lbf(self) -> f64 {
        self.0 / 4.448_221_615_260_5
    }
}

impl Stress {
    /// From megapascals (= N/mm²).
    #[must_use]
    pub const fn from_mpa(mpa: f64) -> Self {
        Self(mpa)
    }
    /// From pascals.
    #[must_use]
    pub fn from_pa(pa: f64) -> Self {
        Self(pa * 1.0e-6)
    }
    /// From pounds per square inch.
    #[must_use]
    pub fn from_psi(psi: f64) -> Self {
        Self(psi * 4.448_221_615_260_5 / (INCH_MM * INCH_MM))
    }
    /// In megapascals.
    #[must_use]
    pub const fn mpa(self) -> f64 {
        self.0
    }
    /// In pascals.
    #[must_use]
    pub fn pa(self) -> f64 {
        self.0 * 1.0e6
    }
    /// In pounds per square inch.
    #[must_use]
    pub fn psi(self) -> f64 {
        self.0 * (INCH_MM * INCH_MM) / 4.448_221_615_260_5
    }
}

impl Density {
    /// From kg/m³ (the usual material-datasheet unit).
    #[must_use]
    pub fn from_kg_per_m3(v: f64) -> Self {
        // 1 kg/m³ = 1e-3 t / 1e9 mm³ = 1e-12 t/mm³
        Self(v * 1.0e-12)
    }
    /// In kg/m³.
    #[must_use]
    pub fn kg_per_m3(self) -> f64 {
        self.0 * 1.0e12
    }
}

impl Energy {
    /// From newton-millimetres (= millijoules, the internal unit).
    #[must_use]
    pub const fn from_n_mm(v: f64) -> Self {
        Self(v)
    }
    /// From joules (= N·m).
    #[must_use]
    pub fn from_joule(j: f64) -> Self {
        Self(j * 1.0e3)
    }
    /// In newton-millimetres.
    #[must_use]
    pub const fn n_mm(self) -> f64 {
        self.0
    }
    /// In joules.
    #[must_use]
    pub fn joule(self) -> f64 {
        self.0 * 1.0e-3
    }
}

// Exact float equality is intended in these tests: the inputs are small
// integers or exact binary fractions, so the results are exact in IEEE-754.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0)
    }

    #[test]
    fn length_conversions_are_exact() {
        assert!(close(Length::from_inch(1.0).mm(), 25.4));
        assert!(close(Length::from_mm(50.8).inch(), 2.0));
        assert!(close(Length::from_m(1.5).mm(), 1500.0));
    }

    #[test]
    fn physically_meaningful_products() {
        let a: Area = Length::from_mm(2.0) * Length::from_mm(3.0);
        let v: Volume = a * Length::from_mm(4.0);
        assert!(close(v.mm3(), 24.0));
        let s: Stress = Force::from_n(1000.0) / Area::from_mm2(10.0);
        assert!(close(s.mpa(), 100.0));
        let ratio: f64 = Length::from_mm(10.0) / Length::from_mm(4.0);
        assert!(close(ratio, 2.5));
    }

    #[test]
    fn steel_block_mass_in_coherent_units() {
        // A 100 mm cube of steel (7850 kg/m^3) weighs 7.85 kg.
        let v = Volume::from_mm3(100.0 * 100.0 * 100.0);
        let m: Mass = Density::from_kg_per_m3(7850.0) * v;
        assert!(close(m.kg(), 7.85));
    }

    #[test]
    fn force_from_mass_and_acceleration_is_coherent() {
        // 1 kg at 9.80665 m/s^2 is 9.80665 N. Internally: 1e-3 t * 9806.65 mm/s^2.
        let m = Mass::from_kg(1.0);
        let a = Quantity::from_internal(9806.65, Dim::ACCELERATION);
        let f = Quantity::from(m).checked_mul(a).unwrap();
        let f = Force::try_from(f).unwrap();
        assert!(close(f.n(), 9.806_65));
    }

    #[test]
    fn imperial_round_trips() {
        assert!(close(Stress::from_psi(1000.0).psi(), 1000.0));
        assert!(close(Stress::from_psi(1.0).pa(), 6_894.757_293_168_361));
        assert!(close(Force::from_lbf(1.0).n(), 4.448_221_615_260_5));
        assert!(close(Mass::from_lb(1.0).kg(), 0.453_592_37));
    }

    #[test]
    fn angle_helpers() {
        let a = Angle::from_deg(450.0).normalized();
        assert!(close(a.deg(), 90.0));
        assert!(close(Angle::from_deg(30.0).sin(), 0.5));
    }

    #[test]
    fn quantity_round_trip_checks_dimension() {
        let q: Quantity = Length::from_mm(5.0).into();
        assert_eq!(Length::try_from(q).unwrap(), Length::from_mm(5.0));
        assert!(Angle::try_from(q).is_err());
    }

    #[test]
    fn display_honours_precision() {
        assert_eq!(format!("{:.2}", Length::from_mm(1.0 / 3.0)), "0.33 mm");
        assert_eq!(format!("{}", Stress::from_mpa(250.0)), "250 MPa");
    }

    #[test]
    fn sum_of_lengths() {
        let total: Length = [1.0, 2.0, 3.5].iter().map(|&v| Length::from_mm(v)).sum();
        assert!(close(total.mm(), 6.5));
    }
}
