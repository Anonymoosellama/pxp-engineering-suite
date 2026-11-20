// SPDX-License-Identifier: LGPL-2.1-or-later
//! Unit symbols, the unit table, and parsing of compound unit expressions.
//!
//! ## Algorithm: compound unit parsing
//!
//! Grammar (recursive descent, one token of look-ahead):
//!
//! ```text
//! unit  := term (('*' | '/' | '·') term)*
//! term  := atom ('^' integer)?
//! atom  := SYMBOL | '(' unit ')'
//! ```
//!
//! Each SYMBOL is looked up in [`BASE_UNITS`]; `*` multiplies factors and
//! adds dimension exponents, `/` divides and subtracts, `^n` raises the factor
//! to `n` and multiplies the exponents. Complexity is linear in the length of
//! the text. Symbols are case-sensitive, so `MPa` (megapascal) and `mPa`
//! (millipascal, not supported) cannot be confused.

use crate::{Dim, Quantity, UnitError};
use std::f64::consts::PI;

/// Exact conversion constants (by definition, not measurement).
const INCH_MM: f64 = 25.4;
const LB_TONNE: f64 = 0.453_592_37e-3;
const LBF_N: f64 = 4.448_221_615_260_5;

/// One row of the unit table: symbol, factor to internal units, dimension.
///
/// `factor` converts a value expressed in this unit to the internal coherent
/// unit (mm, t, s, rad): `internal = value * factor`.
struct BaseUnit {
    symbol: &'static str,
    factor: f64,
    dim: Dim,
}

const fn bu(symbol: &'static str, factor: f64, dim: Dim) -> BaseUnit {
    BaseUnit {
        symbol,
        factor,
        dim,
    }
}

/// The unit table. Order does not matter; symbols are unique (checked by a
/// unit test).
const BASE_UNITS: &[BaseUnit] = &[
    // length (internal: mm)
    bu("mm", 1.0, Dim::LENGTH),
    bu("cm", 10.0, Dim::LENGTH),
    bu("m", 1000.0, Dim::LENGTH),
    bu("km", 1.0e6, Dim::LENGTH),
    bu("um", 1.0e-3, Dim::LENGTH),
    bu("µm", 1.0e-3, Dim::LENGTH),
    bu("in", INCH_MM, Dim::LENGTH),
    bu("ft", 12.0 * INCH_MM, Dim::LENGTH),
    bu("thou", 1.0e-3 * INCH_MM, Dim::LENGTH),
    // mass (internal: tonne)
    bu("t", 1.0, Dim::MASS),
    bu("kg", 1.0e-3, Dim::MASS),
    bu("g", 1.0e-6, Dim::MASS),
    bu("lb", LB_TONNE, Dim::MASS),
    bu("lbm", LB_TONNE, Dim::MASS),
    // time (internal: s)
    bu("s", 1.0, Dim::TIME),
    bu("ms", 1.0e-3, Dim::TIME),
    bu("min", 60.0, Dim::TIME),
    bu("h", 3600.0, Dim::TIME),
    // angle (internal: rad)
    bu("rad", 1.0, Dim::ANGLE),
    bu("deg", PI / 180.0, Dim::ANGLE),
    bu("°", PI / 180.0, Dim::ANGLE),
    bu("rev", 2.0 * PI, Dim::ANGLE),
    // frequency
    bu("Hz", 1.0, Dim::FREQUENCY),
    // force (internal: N = t·mm/s²)
    bu("N", 1.0, Dim::FORCE),
    bu("kN", 1.0e3, Dim::FORCE),
    bu("lbf", LBF_N, Dim::FORCE),
    // stress (internal: MPa = N/mm²)
    bu("Pa", 1.0e-6, Dim::STRESS),
    bu("kPa", 1.0e-3, Dim::STRESS),
    bu("MPa", 1.0, Dim::STRESS),
    bu("GPa", 1.0e3, Dim::STRESS),
    bu("psi", LBF_N / (INCH_MM * INCH_MM), Dim::STRESS),
    bu("ksi", 1.0e3 * LBF_N / (INCH_MM * INCH_MM), Dim::STRESS),
    // energy (internal: mJ = N·mm)
    bu("mJ", 1.0, Dim::ENERGY),
    bu("J", 1.0e3, Dim::ENERGY),
];

/// A unit: a scale factor to internal units plus a dimension, and the text it
/// was written as.
///
/// ```
/// use pxp_units::{parse_unit, Dim};
/// let density = parse_unit("kg/m^3").unwrap();
/// assert_eq!(density.dim(), Dim::DENSITY);
/// // 7850 kg/m^3 is 7.85e-9 t/mm^3 internally.
/// assert!((density.quantity(7850.0).value() - 7.85e-9).abs() < 1e-20);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    symbol: String,
    factor: f64,
    dim: Dim,
}

impl Unit {
    /// The text this unit was parsed from (for display), e.g. `"kg/m^3"`.
    #[must_use]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    /// Multiply a value in this unit by `factor` to get internal units.
    #[must_use]
    pub fn factor(&self) -> f64 {
        self.factor
    }

    /// The unit's physical dimension.
    #[must_use]
    pub fn dim(&self) -> Dim {
        self.dim
    }

    /// Interprets `value` as being in this unit and returns the internal
    /// quantity (`25.4` in `mm` → 25.4 mm; `1` in `in` → 25.4 mm).
    #[must_use]
    pub fn quantity(&self, value: f64) -> Quantity {
        Quantity::from_internal(value * self.factor, self.dim)
    }

    /// Expresses `q` in this unit (25.4 mm → `1.0` for `in`).
    ///
    /// # Errors
    /// [`UnitError::DimensionMismatch`] if `q` does not have this unit's
    /// dimension.
    pub fn value_of(&self, q: Quantity) -> Result<f64, UnitError> {
        let q = q.expect_dim(self.dim)?;
        Ok(q.value() / self.factor)
    }
}

/// Parses a unit expression: a single symbol (`mm`, `deg`, `MPa`) or a
/// compound such as `kg/m^3`, `N*m`, `mm/s^2`, `(lbf*in)`.
///
/// # Errors
/// [`UnitError::UnknownUnit`] for unknown symbols, [`UnitError::Syntax`] for
/// malformed text, [`UnitError::BadExponent`] for exponent overflow.
pub fn parse_unit(text: &str) -> Result<Unit, UnitError> {
    let mut p = Parser {
        chars: text.trim().chars().collect(),
        pos: 0,
    };
    if p.chars.is_empty() {
        return Err(UnitError::Syntax("empty unit".into()));
    }
    let (factor, dim) = p.unit()?;
    if p.pos != p.chars.len() {
        return Err(UnitError::Syntax(format!(
            "unexpected text after unit in '{text}'"
        )));
    }
    Ok(Unit {
        symbol: text.trim().to_owned(),
        factor,
        dim,
    })
}

/// Recursive-descent parser over the unit grammar in the module docs.
struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn unit(&mut self) -> Result<(f64, Dim), UnitError> {
        let (mut factor, mut dim) = self.term()?;
        loop {
            self.skip_ws();
            match self.peek() {
                Some('*' | '·') => {
                    self.pos += 1;
                    let (f, d) = self.term()?;
                    factor *= f;
                    dim = dim.checked_mul(d).ok_or_else(overflow)?;
                }
                Some('/') => {
                    self.pos += 1;
                    let (f, d) = self.term()?;
                    factor /= f;
                    dim = dim.checked_div(d).ok_or_else(overflow)?;
                }
                _ => return Ok((factor, dim)),
            }
        }
    }

    fn term(&mut self) -> Result<(f64, Dim), UnitError> {
        let (factor, dim) = self.atom()?;
        self.skip_ws();
        if self.peek() == Some('^') {
            self.pos += 1;
            let n = self.integer()?;
            let dim = dim.checked_powi(n).ok_or_else(overflow)?;
            return Ok((factor.powi(i32::from(n)), dim));
        }
        Ok((factor, dim))
    }

    fn atom(&mut self) -> Result<(f64, Dim), UnitError> {
        self.skip_ws();
        if self.peek() == Some('(') {
            self.pos += 1;
            let inner = self.unit()?;
            self.skip_ws();
            if self.peek() != Some(')') {
                return Err(UnitError::Syntax("missing ')'".into()));
            }
            self.pos += 1;
            return Ok(inner);
        }
        let start = self.pos;
        while self
            .peek()
            .is_some_and(|c| c.is_alphabetic() || c == '°' || c == 'µ')
        {
            self.pos += 1;
        }
        let symbol: String = self
            .chars
            .get(start..self.pos)
            .unwrap_or_default()
            .iter()
            .collect();
        if symbol.is_empty() {
            return Err(UnitError::Syntax(format!(
                "expected a unit symbol at position {}",
                start + 1
            )));
        }
        BASE_UNITS
            .iter()
            .find(|u| u.symbol == symbol)
            .map(|u| (u.factor, u.dim))
            .ok_or(UnitError::UnknownUnit(symbol))
    }

    fn integer(&mut self) -> Result<i8, UnitError> {
        self.skip_ws();
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.pos += 1;
        }
        let text: String = self
            .chars
            .get(start..self.pos)
            .unwrap_or_default()
            .iter()
            .collect();
        text.parse::<i8>()
            .map_err(|_| UnitError::BadExponent(format!("'{text}' after '^'")))
    }
}

fn overflow() -> UnitError {
    UnitError::BadExponent("dimension exponent overflow".into())
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
    fn symbols_are_unique() {
        for (i, a) in BASE_UNITS.iter().enumerate() {
            for b in &BASE_UNITS[i + 1..] {
                assert_ne!(a.symbol, b.symbol, "duplicate unit symbol");
            }
        }
    }

    #[test]
    fn exact_definitions() {
        assert!(close(parse_unit("in").unwrap().factor(), 25.4));
        assert!(close(parse_unit("ft").unwrap().factor(), 304.8));
        // 1 psi = 6894.757... Pa = 0.006894757... MPa
        assert!(close(
            parse_unit("psi").unwrap().factor(),
            0.006_894_757_293_168_361
        ));
        // 1 lbf = 4.4482216152605 N exactly
        assert!(close(
            parse_unit("lbf").unwrap().factor(),
            4.448_221_615_260_5
        ));
    }

    #[test]
    fn coherent_derived_units() {
        // N/mm^2 is exactly MPa in the internal system.
        let n_per_mm2 = parse_unit("N/mm^2").unwrap();
        assert_eq!(n_per_mm2.dim(), Dim::STRESS);
        assert!(close(n_per_mm2.factor(), 1.0));
        // kg*m/s^2 is exactly 1 N.
        let newton = parse_unit("kg*m/s^2").unwrap();
        assert_eq!(newton.dim(), Dim::FORCE);
        assert!(close(newton.factor(), 1.0));
        // J = N*m = 1000 N*mm
        assert!(close(
            parse_unit("N*m").unwrap().factor(),
            parse_unit("J").unwrap().factor()
        ));
    }

    #[test]
    fn parentheses_and_whitespace() {
        let u = parse_unit(" ( lbf * in ) ").unwrap();
        assert_eq!(u.dim(), Dim::ENERGY);
        assert!(close(u.factor(), 4.448_221_615_260_5 * 25.4));
    }

    #[test]
    fn errors_are_typed() {
        assert!(matches!(
            parse_unit("furlong"),
            Err(UnitError::UnknownUnit(_))
        ));
        assert!(matches!(parse_unit(""), Err(UnitError::Syntax(_))));
        assert!(matches!(parse_unit("kg/"), Err(UnitError::Syntax(_))));
        assert!(matches!(parse_unit("m^x"), Err(UnitError::BadExponent(_))));
        assert!(matches!(parse_unit("(m"), Err(UnitError::Syntax(_))));
        assert!(matches!(
            parse_unit("m^100*m^100"),
            Err(UnitError::BadExponent(_))
        ));
    }

    #[test]
    fn value_of_round_trips() {
        let inch = parse_unit("in").unwrap();
        let q = inch.quantity(2.0);
        assert!(close(q.value(), 50.8));
        assert!(close(inch.value_of(q).unwrap(), 2.0));
        assert!(parse_unit("deg").unwrap().value_of(q).is_err());
    }
}
