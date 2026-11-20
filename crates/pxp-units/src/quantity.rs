// SPDX-License-Identifier: LGPL-2.1-or-later
//! Runtime, dimension-checked quantities.

use crate::{Dim, UnitError};
use std::fmt;

/// A number together with its physical dimension, stored in internal units
/// (mm, t, s, rad; see the crate docs).
///
/// `Quantity` is what the expression engine computes with: the dimension of
/// `width * 2` is only known once `width` is evaluated. Code that knows the
/// dimension at compile time should convert to a typed newtype as early as
/// possible with `Length::try_from(q)` and friends.
///
/// ```
/// use pxp_units::{parse_unit, Dim, Quantity};
/// let inch = parse_unit("in").unwrap().quantity(1.0);
/// let mm = parse_unit("mm").unwrap().quantity(25.4);
/// assert!((inch.value() - mm.value()).abs() < 1e-12);
/// assert_eq!(inch.checked_add(mm).unwrap().dim(), Dim::LENGTH);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantity {
    value: f64,
    dim: Dim,
}

impl Quantity {
    /// Builds a quantity from a value **already in internal units**.
    ///
    /// Prefer [`crate::Unit::quantity`] when the value is in a named unit;
    /// this constructor exists for code that has just computed an internal
    /// value (the expression evaluator, conversions from newtypes).
    #[must_use]
    pub const fn from_internal(value: f64, dim: Dim) -> Self {
        Self { value, dim }
    }

    /// A dimensionless number.
    #[must_use]
    pub const fn number(value: f64) -> Self {
        Self {
            value,
            dim: Dim::DIMENSIONLESS,
        }
    }

    /// The value in internal units (mm, t, s, rad and their coherent
    /// combinations).
    #[must_use]
    pub const fn value(self) -> f64 {
        self.value
    }

    /// The physical dimension.
    #[must_use]
    pub const fn dim(self) -> Dim {
        self.dim
    }

    /// Sum of two quantities of the same dimension.
    ///
    /// # Errors
    /// [`UnitError::DimensionMismatch`] if the dimensions differ
    /// (`10mm + 5deg`).
    pub fn checked_add(self, rhs: Self) -> Result<Self, UnitError> {
        self.require_same(rhs)?;
        Ok(Self {
            value: self.value + rhs.value,
            dim: self.dim,
        })
    }

    /// Difference of two quantities of the same dimension.
    ///
    /// # Errors
    /// [`UnitError::DimensionMismatch`] if the dimensions differ.
    pub fn checked_sub(self, rhs: Self) -> Result<Self, UnitError> {
        self.require_same(rhs)?;
        Ok(Self {
            value: self.value - rhs.value,
            dim: self.dim,
        })
    }

    /// Product; dimensions combine (`length * length = area`).
    ///
    /// # Errors
    /// [`UnitError::BadExponent`] if a dimension exponent overflows.
    pub fn checked_mul(self, rhs: Self) -> Result<Self, UnitError> {
        let dim = self
            .dim
            .checked_mul(rhs.dim)
            .ok_or_else(|| UnitError::BadExponent("dimension exponent overflow".into()))?;
        Ok(Self {
            value: self.value * rhs.value,
            dim,
        })
    }

    /// Quotient; dimensions combine (`force / area = stress`).
    ///
    /// Division by zero follows IEEE-754 (gives ±inf or NaN); callers that
    /// must reject it (the expression evaluator does) check the result with
    /// [`Quantity::is_finite`].
    ///
    /// # Errors
    /// [`UnitError::BadExponent`] if a dimension exponent overflows.
    pub fn checked_div(self, rhs: Self) -> Result<Self, UnitError> {
        let dim = self
            .dim
            .checked_div(rhs.dim)
            .ok_or_else(|| UnitError::BadExponent("dimension exponent overflow".into()))?;
        Ok(Self {
            value: self.value / rhs.value,
            dim,
        })
    }

    /// Raises to a power. A dimensioned base needs a small integer exponent
    /// (`side^2` is an area); a dimensionless base accepts any real exponent.
    ///
    /// # Errors
    /// [`UnitError::BadExponent`] if the base has a dimension and the
    /// exponent is not an integer in −127..=127, or the exponents overflow.
    pub fn checked_pow(self, exponent: Self) -> Result<Self, UnitError> {
        if !exponent.dim.is_dimensionless() {
            return Err(UnitError::BadExponent(
                "an exponent must be dimensionless".into(),
            ));
        }
        let e = exponent.value;
        if self.dim.is_dimensionless() {
            return Ok(Self::number(self.value.powf(e)));
        }
        let rounded = e.round();
        if (e - rounded).abs() > 1e-12 || !(-127.0..=127.0).contains(&rounded) {
            return Err(UnitError::BadExponent(format!(
                "{} raised to non-integer power {e}",
                self.dim
            )));
        }
        // In range and integral (checked just above), so the cast is exact.
        #[allow(clippy::cast_possible_truncation)]
        let n = rounded as i8;
        let dim = self
            .dim
            .checked_powi(n)
            .ok_or_else(|| UnitError::BadExponent("dimension exponent overflow".into()))?;
        Ok(Self {
            value: self.value.powi(i32::from(n)),
            dim,
        })
    }

    /// Square root; the dimension's exponents must all be even.
    ///
    /// # Errors
    /// [`UnitError::BadExponent`] for `sqrt(length)` and similar.
    pub fn checked_sqrt(self) -> Result<Self, UnitError> {
        let dim = self.dim.checked_root(2).ok_or_else(|| {
            UnitError::BadExponent(format!(
                "square root of {} has no physical meaning",
                self.dim
            ))
        })?;
        Ok(Self {
            value: self.value.sqrt(),
            dim,
        })
    }

    /// Absolute value.
    #[must_use]
    pub fn abs(self) -> Self {
        Self {
            value: self.value.abs(),
            dim: self.dim,
        }
    }

    /// True unless the value is NaN or infinite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.value.is_finite()
    }

    /// Ensures `self` has the dimension `expected`.
    ///
    /// # Errors
    /// [`UnitError::DimensionMismatch`] otherwise.
    pub fn expect_dim(self, expected: Dim) -> Result<Self, UnitError> {
        if self.dim == expected {
            Ok(self)
        } else {
            Err(UnitError::DimensionMismatch {
                expected,
                found: self.dim,
            })
        }
    }

    fn require_same(self, rhs: Self) -> Result<(), UnitError> {
        if self.dim == rhs.dim {
            Ok(())
        } else {
            Err(UnitError::DimensionMismatch {
                expected: self.dim,
                found: rhs.dim,
            })
        }
    }
}

impl std::ops::Neg for Quantity {
    type Output = Self;
    /// Negation keeps the dimension.
    fn neg(self) -> Self {
        Self {
            value: -self.value,
            dim: self.dim,
        }
    }
}

impl fmt::Display for Quantity {
    /// Internal units with the dimension name; user-facing display should go
    /// through [`crate::UnitSystem`] instead.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.dim.is_dimensionless() {
            write!(f, "{}", self.value)
        } else {
            write!(f, "{} [internal {}]", self.value, self.dim)
        }
    }
}

// Exact float equality is intended in these tests: the inputs are small
// integers or exact binary fractions, so the results are exact in IEEE-754.
#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn len(mm: f64) -> Quantity {
        Quantity::from_internal(mm, Dim::LENGTH)
    }

    #[test]
    fn add_requires_same_dimension() {
        assert_eq!(len(1.0).checked_add(len(2.0)).unwrap().value(), 3.0);
        let angle = Quantity::from_internal(1.0, Dim::ANGLE);
        assert!(matches!(
            len(1.0).checked_add(angle),
            Err(UnitError::DimensionMismatch { .. })
        ));
    }

    #[test]
    fn mul_div_combine_dimensions() {
        let area = len(2.0).checked_mul(len(3.0)).unwrap();
        assert_eq!(area.dim(), Dim::AREA);
        assert_eq!(area.value(), 6.0);
        let back = area.checked_div(len(3.0)).unwrap();
        assert_eq!(back.dim(), Dim::LENGTH);
    }

    #[test]
    fn pow_rules() {
        let sq = len(3.0).checked_pow(Quantity::number(2.0)).unwrap();
        assert_eq!((sq.value(), sq.dim()), (9.0, Dim::AREA));
        assert!(len(3.0).checked_pow(Quantity::number(0.5)).is_err());
        assert!(len(3.0).checked_pow(len(1.0)).is_err());
        let n = Quantity::number(4.0)
            .checked_pow(Quantity::number(0.5))
            .unwrap();
        assert_eq!(n.value(), 2.0);
    }

    #[test]
    fn sqrt_needs_even_exponents() {
        let area = Quantity::from_internal(16.0, Dim::AREA);
        assert_eq!(area.checked_sqrt().unwrap(), len(4.0));
        assert!(len(16.0).checked_sqrt().is_err());
    }
}
