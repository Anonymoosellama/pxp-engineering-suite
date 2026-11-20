// SPDX-License-Identifier: LGPL-2.1-or-later
//! Display unit systems: what a part shows the user (mm or inch), as NX parts
//! do. Internal storage is always the coherent mm-t-s system.

use crate::{Dim, Quantity, Unit, UnitError, parse_unit};

/// A part's display unit system.
///
/// The choice affects only what the user sees and how a bare number typed
/// into a dialog is interpreted; stored values are always internal units.
/// Changing it never changes geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UnitSystem {
    /// Millimetre parts: mm, kg, N, MPa; angles in degrees.
    #[default]
    Millimetre,
    /// Inch parts: in, lbm, lbf, psi; angles in degrees.
    Inch,
}

impl UnitSystem {
    /// The unit used to show values of dimension `dim`, or `None` for
    /// dimensions without a conventional display unit (those fall back to
    /// internal units in [`UnitSystem::format`]).
    ///
    /// Angles display in degrees in both systems, matching NX.
    ///
    /// # Errors
    /// Never in practice: every symbol below is in the unit table (a unit
    /// test checks this); the `Result` keeps parsing failures typed rather
    /// than panicking.
    pub fn display_unit(self, dim: Dim) -> Result<Option<Unit>, UnitError> {
        let symbol = match (self, dim) {
            (_, d) if d == Dim::DIMENSIONLESS => return Ok(None),
            (_, d) if d == Dim::ANGLE => "deg",
            (_, d) if d == Dim::TIME => "s",
            (_, d) if d == Dim::FREQUENCY => "Hz",
            (Self::Millimetre, d) if d == Dim::LENGTH => "mm",
            (Self::Millimetre, d) if d == Dim::AREA => "mm^2",
            (Self::Millimetre, d) if d == Dim::VOLUME => "mm^3",
            (Self::Millimetre, d) if d == Dim::MASS => "kg",
            (Self::Millimetre, d) if d == Dim::FORCE => "N",
            (Self::Millimetre, d) if d == Dim::STRESS => "MPa",
            (Self::Millimetre, d) if d == Dim::ENERGY => "N*mm",
            (Self::Millimetre, d) if d == Dim::DENSITY => "kg/m^3",
            (Self::Millimetre, d) if d == Dim::VELOCITY => "mm/s",
            (Self::Millimetre, d) if d == Dim::ACCELERATION => "mm/s^2",
            (Self::Inch, d) if d == Dim::LENGTH => "in",
            (Self::Inch, d) if d == Dim::AREA => "in^2",
            (Self::Inch, d) if d == Dim::VOLUME => "in^3",
            (Self::Inch, d) if d == Dim::MASS => "lbm",
            (Self::Inch, d) if d == Dim::FORCE => "lbf",
            (Self::Inch, d) if d == Dim::STRESS => "psi",
            (Self::Inch, d) if d == Dim::ENERGY => "lbf*in",
            (Self::Inch, d) if d == Dim::DENSITY => "lbm/in^3",
            (Self::Inch, d) if d == Dim::VELOCITY => "in/s",
            (Self::Inch, d) if d == Dim::ACCELERATION => "in/s^2",
            _ => return Ok(None),
        };
        parse_unit(symbol).map(Some)
    }

    /// The unit a bare number typed into a length field means in this part
    /// (`"25"` is 25 mm in a millimetre part, 25 in in an inch part).
    ///
    /// # Errors
    /// Never in practice (see [`UnitSystem::display_unit`]).
    pub fn length_unit(self) -> Result<Unit, UnitError> {
        parse_unit(match self {
            Self::Millimetre => "mm",
            Self::Inch => "in",
        })
    }

    /// Formats a quantity for the user, e.g. `"25.400 mm"` or `"1.000 in"`.
    /// Dimensions without a display unit are shown in internal units with the
    /// dimension spelled out, so nothing is ever shown unit-less by mistake.
    ///
    /// # Errors
    /// Never in practice (see [`UnitSystem::display_unit`]).
    pub fn format(self, q: Quantity, decimals: usize) -> Result<String, UnitError> {
        match self.display_unit(q.dim())? {
            Some(unit) => Ok(format!(
                "{:.*} {}",
                decimals,
                unit.value_of(q)?,
                unit.symbol()
            )),
            None if q.dim().is_dimensionless() => Ok(format!("{:.*}", decimals, q.value())),
            None => Ok(format!("{:.*} [internal {}]", decimals, q.value(), q.dim())),
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
    fn every_display_symbol_parses() {
        let dims = [
            Dim::DIMENSIONLESS,
            Dim::LENGTH,
            Dim::AREA,
            Dim::VOLUME,
            Dim::MASS,
            Dim::TIME,
            Dim::ANGLE,
            Dim::FREQUENCY,
            Dim::VELOCITY,
            Dim::ACCELERATION,
            Dim::FORCE,
            Dim::STRESS,
            Dim::ENERGY,
            Dim::DENSITY,
        ];
        for system in [UnitSystem::Millimetre, UnitSystem::Inch] {
            for d in dims {
                let unit = system.display_unit(d).unwrap();
                if let Some(u) = unit {
                    assert_eq!(u.dim(), d, "{system:?} display unit for {d}");
                }
            }
        }
    }

    #[test]
    fn same_quantity_two_systems() {
        let q = Quantity::from_internal(25.4, Dim::LENGTH);
        assert_eq!(UnitSystem::Millimetre.format(q, 3).unwrap(), "25.400 mm");
        assert_eq!(UnitSystem::Inch.format(q, 3).unwrap(), "1.000 in");
    }

    #[test]
    fn angles_display_in_degrees() {
        let q = Quantity::from_internal(std::f64::consts::FRAC_PI_2, Dim::ANGLE);
        assert_eq!(UnitSystem::Inch.format(q, 1).unwrap(), "90.0 deg");
    }

    #[test]
    fn unusual_dimension_never_loses_its_unit() {
        let q = Quantity::from_internal(2.0, Dim::new(4, 0, 0, 0));
        assert_eq!(
            UnitSystem::Millimetre.format(q, 1).unwrap(),
            "2.0 [internal L^4]"
        );
    }
}
