// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-units: typed physical quantities
//!
//! Every physical number in PXP Engineering Suite carries its unit in the type
//! system, so millimetres can never be added to inches, or newtons to
//! megapascals, by accident.
//!
//! ## Responsibility
//!
//! - **Static newtypes** ([`Length`], [`Angle`], [`Area`], [`Volume`], [`Mass`],
//!   [`Time`], [`Force`], [`Stress`], [`Density`], [`Energy`]) for code that
//!   knows at compile time what a number means. Arithmetic between them is
//!   only implemented where it is physically meaningful (`Length * Length =
//!   Area`, `Force / Area = Stress`, ...).
//! - **Runtime quantities** ([`Quantity`] = value + [`Dim`]) for data whose
//!   unit is only known at run time, chiefly the NX-style expression engine in
//!   `pxp-doc` (`p12 = width * 2`, `25.4mm`, `7850[kg/m^3]`).
//! - **Unit symbols and parsing** ([`Unit`], [`parse_unit`]) and **display unit
//!   systems** ([`UnitSystem`]: a part shows mm or inch, as NX parts do).
//!
//! ## The internal unit system (read this before adding a unit)
//!
//! All values are stored in one *coherent* system based on the millimetre:
//!
//! | Quantity | Internal unit | Why |
//! | --- | --- | --- |
//! | length | millimetre (mm) | master prompt; CAD convention |
//! | mass | **tonne** (t) | makes the system coherent: 1 N = 1 t·mm/s² |
//! | time | second (s) | SI |
//! | angle | radian (rad) | master prompt |
//! | force | newton (N) | derived, coherent |
//! | stress | megapascal (MPa = N/mm²) | derived, coherent |
//! | energy | millijoule (mJ = N·mm) | derived, coherent |
//! | density | t/mm³ (steel ≈ 7.85e-9) | derived, coherent |
//!
//! *Coherent* means no conversion factors appear when quantities are
//! multiplied or divided: `force / area` of internal values is directly a
//! stress in MPa. This is the same mm-N-MPa-tonne-s system that Stage 5 FEA
//! solvers use, so solver input needs no unit conversion at all. Users never
//! see tonnes unless they ask: display goes through [`UnitSystem`].
//!
//! Angle is treated as its own dimension (unlike SI, where the radian is
//! dimensionless). That catches `length + angle` mistakes in expressions. The
//! price is that an arc length must be written `r * theta / 1rad`; this is
//! deliberate and documented in the expression guide.
//!
//! ## Data flow
//!
//! Foundation crate: depends on nothing. `pxp-math`, `pxp-doc` (properties
//! and expressions) and every later crate depend on it.
//!
//! ## Invariants
//!
//! - Newtype fields are private: a value can only enter through a constructor
//!   that names its unit (`Length::from_mm`, `Length::from_inch`).
//! - Conversion factors are exact where the definition is exact (1 in =
//!   25.4 mm, 1 lb = 0.453 592 37 kg, 1 lbf = 4.448 221 615 260 5 N).
//! - `Quantity` addition of mismatched dimensions is an error, never a
//!   silent coercion.
//!
//! ## Must never
//!
//! - Expose a raw `f64` without naming its unit (accessors are `mm()`,
//!   `inch()`, `rad()`, `deg()`, ...).
//! - Panic on bad input: parsing returns [`UnitError`].
#![forbid(unsafe_code)]

mod dim;
mod quantity;
mod system;
mod typed;
mod unit;

pub use dim::Dim;
pub use quantity::Quantity;
pub use system::UnitSystem;
pub use typed::{Angle, Area, Density, Energy, Force, Length, Mass, Stress, Time, Volume};
pub use unit::{Unit, parse_unit};

use std::fmt;

/// Errors from unit parsing and dimension-checked arithmetic.
#[derive(Debug, Clone, PartialEq)]
pub enum UnitError {
    /// The unit symbol is not in the unit table (for example `"furlong"`).
    UnknownUnit(String),
    /// Two quantities with different dimensions were added, subtracted or
    /// compared, or a quantity was converted to a newtype of another
    /// dimension.
    DimensionMismatch {
        /// The dimension that was required.
        expected: Dim,
        /// The dimension that was supplied.
        found: Dim,
    },
    /// A unit expression such as `kg/m^` could not be parsed.
    Syntax(String),
    /// The exponent would overflow the small integer exponents in [`Dim`]
    /// (|exponent| > 127), or a non-integer power was applied to a
    /// dimensioned quantity.
    BadExponent(String),
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownUnit(u) => write!(f, "unknown unit '{u}'"),
            Self::DimensionMismatch { expected, found } => {
                write!(f, "dimension mismatch: expected {expected}, found {found}")
            }
            Self::Syntax(msg) => write!(f, "unit syntax error: {msg}"),
            Self::BadExponent(msg) => write!(f, "bad exponent: {msg}"),
        }
    }
}

impl std::error::Error for UnitError {}
