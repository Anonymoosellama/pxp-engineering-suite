// SPDX-License-Identifier: LGPL-2.1-or-later
//! Per-part modeling tolerances.

use pxp_units::{Angle, Length};
use std::fmt;

/// Modeling distance and angle tolerances, stored per part (in the `.pxp`
/// `meta` table) and shown in part preferences, as the master prompt
/// requires.
///
/// The defaults (0.01 mm, 0.5°) are PXP's own starting values. A site can
/// change them through layered customer defaults; an existing part keeps the
/// tolerances it was created with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerances {
    distance: Length,
    angle: Angle,
}

/// Errors from [`Tolerances::new`].
#[derive(Debug, Clone, PartialEq)]
pub enum ToleranceError {
    /// The distance tolerance must be finite and strictly positive.
    BadDistance(Length),
    /// The angle tolerance must be finite and in (0, 90°).
    BadAngle(Angle),
}

impl fmt::Display for ToleranceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadDistance(d) => write!(f, "distance tolerance must be > 0 and finite, got {d}"),
            Self::BadAngle(a) => {
                write!(
                    f,
                    "angle tolerance must be in (0, 90) degrees, got {} deg",
                    a.deg()
                )
            }
        }
    }
}

impl std::error::Error for ToleranceError {}

impl Tolerances {
    /// Validated tolerances.
    ///
    /// # Errors
    /// [`ToleranceError`] for non-positive, non-finite or out-of-range
    /// values.
    pub fn new(distance: Length, angle: Angle) -> Result<Self, ToleranceError> {
        if !(distance.is_finite() && distance > Length::ZERO) {
            return Err(ToleranceError::BadDistance(distance));
        }
        if !(angle.is_finite() && angle > Angle::ZERO && angle < Angle::from_deg(90.0)) {
            return Err(ToleranceError::BadAngle(angle));
        }
        Ok(Self { distance, angle })
    }

    /// Distance tolerance: two points closer than this are coincident.
    #[must_use]
    pub const fn distance(&self) -> Length {
        self.distance
    }

    /// Angle tolerance: two directions closer than this are parallel.
    #[must_use]
    pub const fn angle(&self) -> Angle {
        self.angle
    }
}

impl Default for Tolerances {
    fn default() -> Self {
        Self {
            distance: Length::from_mm(0.01),
            angle: Angle::from_rad(0.5_f64.to_radians()),
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
    fn defaults_are_valid() {
        let d = Tolerances::default();
        assert!(Tolerances::new(d.distance(), d.angle()).is_ok());
        assert!((d.angle().deg() - 0.5).abs() < 1e-12);
    }

    #[test]
    fn rejects_bad_values() {
        let a = Angle::from_deg(0.5);
        assert!(Tolerances::new(Length::ZERO, a).is_err());
        assert!(Tolerances::new(Length::from_mm(f64::NAN), a).is_err());
        assert!(Tolerances::new(Length::from_mm(0.01), Angle::from_deg(95.0)).is_err());
        assert!(Tolerances::new(Length::from_mm(0.01), Angle::ZERO).is_err());
    }
}
