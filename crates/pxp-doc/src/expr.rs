// SPDX-License-Identifier: LGPL-2.1-or-later
//! NX-style named expressions: parsing and unit-aware evaluation.
//!
//! An expression is a named formula such as `p12 = width * 2` or
//! `hole_d = 0.5in + 0.1mm`. Every feature parameter is an expression, so
//! this module is on the path of every model edit.
//!
//! ## Syntax
//!
//! ```text
//! expr    := ['='] sum                      (a leading '=' is accepted, as typed in dialogs)
//! sum     := product (('+' | '-') product)*
//! product := unary (('*' | '/') unary)*
//! unary   := ('-' | '+') unary | power
//! power   := primary ('^' unary)?           (right-associative; -2^2 = -4)
//! primary := NUMBER [UNIT] | NAME | NAME '(' [expr (',' expr)*] ')' | '(' expr ')'
//! NUMBER  := digits ['.' digits] [('e'|'E') ['+'|'-'] digits]   or   '.' digits ...
//! UNIT    := letters directly after the number (25.4mm, 90deg, 1in)
//!          | '[' unit-expression ']'          (7850[kg/m^3], 9.81 [m/s^2])
//! NAME    := (letter | '_') (letter | digit | '_')*      ASCII, at most 64 characters
//! ```
//!
//! A unit must touch its number (`25.4mm`) or be bracketed, so a unit symbol
//! can never be confused with an expression name: `in` alone is a name, while
//! `1in` is one inch.
//!
//! ## Method
//!
//! Recursive descent with one token of look-ahead, building an AST, then a
//! recursive evaluator over [`Quantity`] values, so dimensions are checked at
//! every operator. Linear time in the formula length. Input is capped at
//! 10 000 characters and nesting at 128 levels, so hostile input (this parser
//! is fuzzed) cannot exhaust the stack.
//!
//! ## Functions and the NX trigonometry convention
//!
//! `sin cos tan` accept an angle quantity (`sin(0.5rad)`, `sin(30deg)`) or a
//! **bare number, which is read as degrees** (`sin(30)` = 0.5), matching NX.
//! `asin acos atan atan2` return angles. `sqrt` requires even dimension
//! exponents (√area is a length; √length is an error). `min max abs hypot`
//! require equal dimensions. `floor ceil round exp ln log10` accept only
//! dimensionless numbers, because rounding "a length" would silently depend
//! on the display unit. `pi()` is π.
//!
//! ## Failure modes
//!
//! Syntax errors report a 1-based character position. Division by zero and
//! domain errors (`asin(2)`) report [`ExprError::NotFinite`]; no NaN or
//! infinity ever reaches a model parameter.

use pxp_units::{Dim, Quantity, UnitError, parse_unit};
use std::collections::BTreeSet;
use std::fmt;

/// Property holding an expression object's formula text.
pub const FORMULA_PROP: &str = "formula";
/// Property holding an expression object's last evaluated value.
pub const VALUE_PROP: &str = "value";

const MAX_LEN: usize = 10_000;
const MAX_DEPTH: usize = 128;
const MAX_NAME_LEN: usize = 64;

/// The built-in function names (reserved: not usable as expression names).
pub const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sqrt", "abs", "min", "max", "hypot",
    "floor", "ceil", "round", "exp", "ln", "log10", "pi",
];

/// Errors from parsing or evaluating an expression.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprError {
    /// Malformed text; `pos` is the 1-based character position.
    Syntax {
        /// 1-based character position of the problem.
        pos: usize,
        /// What was expected or found.
        msg: String,
    },
    /// Longer than 10 000 characters.
    TooLong,
    /// Nested more than 128 levels deep.
    TooDeep,
    /// A referenced expression name does not exist.
    UnknownName(String),
    /// Not a built-in function.
    UnknownFunction(String),
    /// Wrong number of function arguments.
    Arity {
        /// Function name.
        function: String,
        /// Human-readable expected count ("1", "2", "at least 1").
        expected: &'static str,
        /// Arguments supplied.
        found: usize,
    },
    /// A unit or dimension error (`10mm + 5deg`, `sqrt(10mm)`, `furlong`).
    Unit(UnitError),
    /// Division by zero or a domain error would produce NaN or infinity.
    NotFinite(String),
    /// Not a valid expression name (syntax, length, or a reserved function
    /// name).
    InvalidName(String),
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { pos, msg } => write!(f, "syntax error at character {pos}: {msg}"),
            Self::TooLong => write!(f, "expression longer than {MAX_LEN} characters"),
            Self::TooDeep => write!(f, "expression nested deeper than {MAX_DEPTH} levels"),
            Self::UnknownName(n) => write!(f, "unknown expression '{n}'"),
            Self::UnknownFunction(n) => write!(f, "unknown function '{n}'"),
            Self::Arity {
                function,
                expected,
                found,
            } => {
                write!(f, "{function}() takes {expected} argument(s), got {found}")
            }
            Self::Unit(e) => write!(f, "{e}"),
            Self::NotFinite(what) => write!(f, "{what} is not a finite number"),
            Self::InvalidName(n) => write!(f, "'{n}' is not a valid expression name"),
        }
    }
}

impl std::error::Error for ExprError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unit(e) => Some(e),
            _ => None,
        }
    }
}

impl From<UnitError> for ExprError {
    fn from(e: UnitError) -> Self {
        Self::Unit(e)
    }
}

/// True if `name` can name an expression: ASCII letter or `_`, then letters,
/// digits or `_`; at most 64 characters; not a built-in function name.
#[must_use]
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first_ok
        && name.len() <= MAX_NAME_LEN
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !FUNCTIONS.contains(&name)
}

/// Parsed expression syntax tree.
#[derive(Debug, Clone, PartialEq)]
enum Ast {
    Num(Quantity),
    Name(String),
    Neg(Box<Ast>),
    Bin(BinOp, Box<Ast>, Box<Ast>),
    Call(String, Vec<Ast>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

/// A parsed expression formula.
///
/// ```
/// use pxp_doc::expr::Expression;
/// use pxp_units::{Dim, Quantity};
/// let e = Expression::parse("=width/2 + 1in").unwrap();
/// assert_eq!(e.references().into_iter().collect::<Vec<_>>(), vec!["width".to_string()]);
/// let v = e.evaluate(&mut |_name| Ok(Quantity::from_internal(100.0, Dim::LENGTH))).unwrap();
/// assert!((v.value() - (50.0 + 25.4)).abs() < 1e-12);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Expression {
    source: String,
    ast: Ast,
}

impl Expression {
    /// Parses a formula (see the module docs for the grammar).
    ///
    /// # Errors
    /// [`ExprError::Syntax`], [`ExprError::TooLong`], [`ExprError::TooDeep`]
    /// or a unit error for an unknown unit suffix.
    pub fn parse(text: &str) -> Result<Self, ExprError> {
        if text.chars().count() > MAX_LEN {
            return Err(ExprError::TooLong);
        }
        let tokens = lex(text)?;
        let mut p = Parser {
            tokens,
            pos: 0,
            depth: 0,
        };
        if p.peek() == &Tok::Eq {
            p.pos += 1;
        }
        let ast = p.sum()?;
        match p.peek() {
            Tok::End => Ok(Self {
                source: text.trim().to_owned(),
                ast,
            }),
            _ => Err(ExprError::Syntax {
                pos: p.char_pos(),
                msg: "unexpected text".into(),
            }),
        }
    }

    /// The formula as written (trimmed).
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Every expression name the formula refers to (dependency tracking).
    #[must_use]
    pub fn references(&self) -> BTreeSet<String> {
        fn walk(ast: &Ast, out: &mut BTreeSet<String>) {
            match ast {
                Ast::Num(_) => {}
                Ast::Name(n) => {
                    out.insert(n.clone());
                }
                Ast::Neg(a) => walk(a, out),
                Ast::Bin(_, a, b) => {
                    walk(a, out);
                    walk(b, out);
                }
                Ast::Call(_, args) => args.iter().for_each(|a| walk(a, out)),
            }
        }
        let mut out = BTreeSet::new();
        walk(&self.ast, &mut out);
        out
    }

    /// Evaluates the formula. `lookup` resolves each referenced name to its
    /// current value (the document supplies it).
    ///
    /// # Errors
    /// Any [`ExprError`] from lookup, dimension checks, function arity, or a
    /// non-finite result.
    pub fn evaluate(
        &self,
        lookup: &mut dyn FnMut(&str) -> Result<Quantity, ExprError>,
    ) -> Result<Quantity, ExprError> {
        let v = eval(&self.ast, lookup)?;
        finite(v, "the result")
    }
}

fn finite(q: Quantity, what: &str) -> Result<Quantity, ExprError> {
    if q.is_finite() {
        Ok(q)
    } else {
        Err(ExprError::NotFinite(what.to_owned()))
    }
}

fn eval(
    ast: &Ast,
    lookup: &mut dyn FnMut(&str) -> Result<Quantity, ExprError>,
) -> Result<Quantity, ExprError> {
    Ok(match ast {
        Ast::Num(q) => *q,
        Ast::Name(n) => lookup(n)?,
        Ast::Neg(a) => -eval(a, lookup)?,
        Ast::Bin(op, a, b) => {
            let (a, b) = (eval(a, lookup)?, eval(b, lookup)?);
            match op {
                BinOp::Add => a.checked_add(b)?,
                BinOp::Sub => a.checked_sub(b)?,
                BinOp::Mul => a.checked_mul(b)?,
                BinOp::Div => {
                    if b.value() == 0.0 {
                        return Err(ExprError::NotFinite("division by zero".into()));
                    }
                    a.checked_div(b)?
                }
                BinOp::Pow => finite(a.checked_pow(b)?, "the power")?,
            }
        }
        Ast::Call(name, args) => {
            let vals = args
                .iter()
                .map(|a| eval(a, lookup))
                .collect::<Result<Vec<_>, _>>()?;
            call(name, &vals)?
        }
    })
}

/// Applies a built-in function (see the module docs for the rules).
fn call(name: &str, args: &[Quantity]) -> Result<Quantity, ExprError> {
    let arity = |expected: &'static str| ExprError::Arity {
        function: name.to_owned(),
        expected,
        found: args.len(),
    };
    let number = |q: Quantity| q.expect_dim(Dim::DIMENSIONLESS).map(Quantity::value);
    let angle = |v: f64| Quantity::from_internal(v, Dim::ANGLE);
    let out = match (name, args) {
        ("pi", []) => Quantity::number(std::f64::consts::PI),
        ("sin" | "cos" | "tan", [x]) => {
            // NX convention: a bare number is degrees; an angle is honoured.
            let rad = if x.dim() == Dim::ANGLE {
                x.value()
            } else {
                number(*x)?.to_radians()
            };
            Quantity::number(match name {
                "sin" => rad.sin(),
                "cos" => rad.cos(),
                _ => rad.tan(),
            })
        }
        ("asin", [x]) => angle(number(*x)?.asin()),
        ("acos", [x]) => angle(number(*x)?.acos()),
        ("atan", [x]) => angle(number(*x)?.atan()),
        ("atan2", [y, x]) => {
            let x = x.expect_dim(y.dim())?;
            angle(y.value().atan2(x.value()))
        }
        ("sqrt", [x]) => x.checked_sqrt()?,
        ("abs", [x]) => x.abs(),
        ("hypot", [a, b]) => {
            let b = b.expect_dim(a.dim())?;
            Quantity::from_internal(a.value().hypot(b.value()), a.dim())
        }
        ("min" | "max", [first, rest @ ..]) => {
            let mut best = *first;
            for q in rest {
                let q = q.expect_dim(first.dim())?;
                let better = if name == "min" {
                    q.value() < best.value()
                } else {
                    q.value() > best.value()
                };
                if better {
                    best = q;
                }
            }
            best
        }
        ("floor", [x]) => Quantity::number(number(*x)?.floor()),
        ("ceil", [x]) => Quantity::number(number(*x)?.ceil()),
        ("round", [x]) => Quantity::number(number(*x)?.round()),
        ("exp", [x]) => Quantity::number(number(*x)?.exp()),
        ("ln", [x]) => Quantity::number(number(*x)?.ln()),
        ("log10", [x]) => Quantity::number(number(*x)?.log10()),
        // Known function, wrong number of arguments.
        ("pi", _) => return Err(arity("0")),
        ("atan2" | "hypot", _) => return Err(arity("2")),
        ("min" | "max", _) => return Err(arity("at least 1")),
        (n, _) if FUNCTIONS.contains(&n) => return Err(arity("1")),
        (n, _) => return Err(ExprError::UnknownFunction(n.to_owned())),
    };
    finite(out, &format!("{name}()"))
}

// ---------------------------------------------------------------- lexer --

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(Quantity),
    Name(String),
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    LParen,
    RParen,
    Comma,
    Eq,
    End,
}

/// Splits the formula into tokens, each tagged with its 0-based character
/// position for error messages.
fn lex(text: &str) -> Result<Vec<(Tok, usize)>, ExprError> {
    let chars: Vec<char> = text.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let syntax = |pos: usize, msg: &str| ExprError::Syntax {
        pos: pos + 1,
        msg: msg.to_owned(),
    };
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(c) = at(i) {
        let start = i;
        let simple = match c {
            ' ' | '\t' | '\n' | '\r' => {
                i += 1;
                continue;
            }
            '+' => Some(Tok::Plus),
            '-' => Some(Tok::Minus),
            '*' => Some(Tok::Star),
            '/' => Some(Tok::Slash),
            '^' => Some(Tok::Caret),
            '(' => Some(Tok::LParen),
            ')' => Some(Tok::RParen),
            ',' => Some(Tok::Comma),
            '=' => Some(Tok::Eq),
            _ => None,
        };
        if let Some(tok) = simple {
            out.push((tok, start));
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            // Mantissa.
            while at(i).is_some_and(|d| d.is_ascii_digit()) {
                i += 1;
            }
            if at(i) == Some('.') {
                i += 1;
                while at(i).is_some_and(|d| d.is_ascii_digit()) {
                    i += 1;
                }
            }
            // Exponent, only if digits follow (so `2em` is "2" + unit "em").
            if matches!(at(i), Some('e' | 'E')) {
                let digits_at = if matches!(at(i + 1), Some('+' | '-')) {
                    i + 2
                } else {
                    i + 1
                };
                if at(digits_at).is_some_and(|d| d.is_ascii_digit()) {
                    i = digits_at;
                    while at(i).is_some_and(|d| d.is_ascii_digit()) {
                        i += 1;
                    }
                }
            }
            let literal: String = chars.get(start..i).unwrap_or_default().iter().collect();
            let value: f64 = literal
                .parse()
                .map_err(|_| syntax(start, "malformed number"))?;
            // Optional unit: adjacent letters, or a bracketed unit expression.
            let mut unit_end = i;
            while at(unit_end).is_some_and(|u| u.is_alphabetic() || u == '°' || u == 'µ') {
                unit_end += 1;
            }
            let quantity = if unit_end > i {
                let sym: String = chars.get(i..unit_end).unwrap_or_default().iter().collect();
                i = unit_end;
                parse_unit(&sym)?.quantity(value)
            } else {
                let mut j = i;
                while at(j).is_some_and(char::is_whitespace) {
                    j += 1;
                }
                if at(j) == Some('[') {
                    let close = (j..chars.len())
                        .find(|&k| at(k) == Some(']'))
                        .ok_or_else(|| syntax(j, "missing ']'"))?;
                    let sym: String = chars.get(j + 1..close).unwrap_or_default().iter().collect();
                    i = close + 1;
                    parse_unit(&sym)?.quantity(value)
                } else {
                    Quantity::number(value)
                }
            };
            out.push((Tok::Num(quantity), start));
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            while at(i).is_some_and(|d| d.is_ascii_alphanumeric() || d == '_') {
                i += 1;
            }
            let name: String = chars.get(start..i).unwrap_or_default().iter().collect();
            if name.len() > MAX_NAME_LEN {
                return Err(syntax(start, "name longer than 64 characters"));
            }
            out.push((Tok::Name(name), start));
            continue;
        }
        return Err(syntax(start, &format!("unexpected character '{c}'")));
    }
    out.push((Tok::End, chars.len()));
    Ok(out)
}

// --------------------------------------------------------------- parser --

struct Parser {
    tokens: Vec<(Tok, usize)>,
    pos: usize,
    depth: usize,
}

const END: Tok = Tok::End;

impl Parser {
    fn peek(&self) -> &Tok {
        self.tokens.get(self.pos).map_or(&END, |(t, _)| t)
    }

    fn char_pos(&self) -> usize {
        self.tokens.get(self.pos).map_or(0, |(_, p)| *p) + 1
    }

    fn next(&mut self) -> Tok {
        let t = self.peek().clone();
        self.pos += 1;
        t
    }

    fn enter(&mut self) -> Result<(), ExprError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(ExprError::TooDeep)
        } else {
            Ok(())
        }
    }

    fn sum(&mut self) -> Result<Ast, ExprError> {
        let mut lhs = self.product()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return Ok(lhs),
            };
            self.pos += 1;
            lhs = Ast::Bin(op, Box::new(lhs), Box::new(self.product()?));
        }
    }

    fn product(&mut self) -> Result<Ast, ExprError> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                _ => return Ok(lhs),
            };
            self.pos += 1;
            lhs = Ast::Bin(op, Box::new(lhs), Box::new(self.unary()?));
        }
    }

    fn unary(&mut self) -> Result<Ast, ExprError> {
        self.enter()?;
        let out = match self.peek() {
            Tok::Minus => {
                self.pos += 1;
                Ast::Neg(Box::new(self.unary()?))
            }
            Tok::Plus => {
                self.pos += 1;
                self.unary()?
            }
            _ => {
                let base = self.primary()?;
                if self.peek() == &Tok::Caret {
                    self.pos += 1;
                    Ast::Bin(BinOp::Pow, Box::new(base), Box::new(self.unary()?))
                } else {
                    base
                }
            }
        };
        self.depth -= 1;
        Ok(out)
    }

    fn primary(&mut self) -> Result<Ast, ExprError> {
        let pos = self.char_pos();
        match self.next() {
            Tok::Num(q) => Ok(Ast::Num(q)),
            Tok::Name(name) => {
                if self.peek() != &Tok::LParen {
                    return Ok(Ast::Name(name));
                }
                self.pos += 1;
                let mut args = Vec::new();
                if self.peek() != &Tok::RParen {
                    loop {
                        args.push(self.sum()?);
                        if self.peek() == &Tok::Comma {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                }
                self.expect_rparen()?;
                Ok(Ast::Call(name, args))
            }
            Tok::LParen => {
                let inner = self.sum()?;
                self.expect_rparen()?;
                Ok(inner)
            }
            Tok::End => Err(ExprError::Syntax {
                pos,
                msg: "unexpected end of expression".into(),
            }),
            _ => Err(ExprError::Syntax {
                pos,
                msg: "expected a number, name or '('".into(),
            }),
        }
    }

    fn expect_rparen(&mut self) -> Result<(), ExprError> {
        let pos = self.char_pos();
        if self.next() == Tok::RParen {
            Ok(())
        } else {
            Err(ExprError::Syntax {
                pos,
                msg: "expected ')'".into(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_str(text: &str) -> Result<Quantity, ExprError> {
        Expression::parse(text)?.evaluate(&mut |n| match n {
            "width" => Ok(Quantity::from_internal(100.0, Dim::LENGTH)),
            "angle" => Ok(Quantity::from_internal(
                std::f64::consts::FRAC_PI_6,
                Dim::ANGLE,
            )),
            "count" => Ok(Quantity::number(4.0)),
            other => Err(ExprError::UnknownName(other.to_owned())),
        })
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    #[test]
    fn precedence_and_associativity() {
        assert!(close(eval_str("1 + 2 * 3").unwrap().value(), 7.0));
        assert!(close(eval_str("(1 + 2) * 3").unwrap().value(), 9.0));
        assert!(
            close(eval_str("2 ^ 3 ^ 2").unwrap().value(), 512.0),
            "right-assoc"
        );
        assert!(close(eval_str("-2 ^ 2").unwrap().value(), -4.0));
        assert!(close(eval_str("2 ^ -1").unwrap().value(), 0.5));
        assert!(
            close(eval_str("10 - 4 - 3").unwrap().value(), 3.0),
            "left-assoc"
        );
    }

    #[test]
    fn units_on_literals() {
        let v = eval_str("1in + 25.4mm").unwrap();
        assert_eq!(v.dim(), Dim::LENGTH);
        assert!(close(v.value(), 50.8));
        let d = eval_str("7850[kg/m^3]").unwrap();
        assert_eq!(d.dim(), Dim::DENSITY);
        let g = eval_str("9.81 [m/s^2]").unwrap();
        assert!(close(g.value(), 9810.0));
        assert!(close(eval_str("1.5e3mm").unwrap().value(), 1500.0));
        assert!(close(eval_str(".5in").unwrap().value(), 12.7));
    }

    #[test]
    fn nx_dialog_forms() {
        assert!(close(eval_str("=width/2").unwrap().value(), 50.0));
        assert!(close(eval_str("width * count").unwrap().value(), 400.0));
    }

    #[test]
    fn dimension_errors_are_caught() {
        assert!(matches!(eval_str("10mm + 5deg"), Err(ExprError::Unit(_))));
        assert!(matches!(eval_str("sqrt(10mm)"), Err(ExprError::Unit(_))));
        assert!(matches!(eval_str("floor(2.5mm)"), Err(ExprError::Unit(_))));
        assert!(matches!(
            eval_str("min(1mm, 1deg)"),
            Err(ExprError::Unit(_))
        ));
        assert_eq!(eval_str("sqrt(width*width)").unwrap().dim(), Dim::LENGTH);
    }

    #[test]
    fn trig_follows_nx_degrees_for_bare_numbers() {
        assert!(close(eval_str("sin(30)").unwrap().value(), 0.5));
        assert!(close(eval_str("sin(angle)").unwrap().value(), 0.5));
        assert!(close(eval_str("cos(60deg)").unwrap().value(), 0.5));
        let a = eval_str("asin(0.5)").unwrap();
        assert_eq!(a.dim(), Dim::ANGLE);
        assert!(close(a.value(), std::f64::consts::FRAC_PI_6));
        assert!(matches!(eval_str("sin(1mm)"), Err(ExprError::Unit(_))));
    }

    #[test]
    fn functions_and_arity() {
        assert!(close(eval_str("max(1mm, 3mm, 2mm)").unwrap().value(), 3.0));
        assert!(close(eval_str("hypot(3mm, 4mm)").unwrap().value(), 5.0));
        assert!(close(
            eval_str("pi()").unwrap().value(),
            std::f64::consts::PI
        ));
        assert!(matches!(
            eval_str("sin(1, 2)"),
            Err(ExprError::Arity { .. })
        ));
        assert!(matches!(eval_str("pi(1)"), Err(ExprError::Arity { .. })));
        assert!(matches!(eval_str("max()"), Err(ExprError::Arity { .. })));
        assert!(matches!(
            eval_str("frobnicate(1)"),
            Err(ExprError::UnknownFunction(_))
        ));
    }

    #[test]
    fn non_finite_results_are_errors() {
        assert!(matches!(eval_str("1/0"), Err(ExprError::NotFinite(_))));
        assert!(matches!(eval_str("asin(2)"), Err(ExprError::NotFinite(_))));
        assert!(matches!(eval_str("ln(0)"), Err(ExprError::NotFinite(_))));
        assert!(matches!(eval_str("10^400"), Err(ExprError::NotFinite(_))));
    }

    #[test]
    fn syntax_errors_have_positions() {
        assert!(matches!(
            Expression::parse("1 +"),
            Err(ExprError::Syntax { pos: 4, .. })
        ));
        assert!(matches!(
            Expression::parse("(1 + 2"),
            Err(ExprError::Syntax { .. })
        ));
        assert!(matches!(
            Expression::parse("1 $ 2"),
            Err(ExprError::Syntax { pos: 3, .. })
        ));
        assert!(matches!(
            Expression::parse("2 3"),
            Err(ExprError::Syntax { .. })
        ));
        assert!(matches!(
            Expression::parse("5furlong"),
            Err(ExprError::Unit(_))
        ));
        assert!(matches!(
            Expression::parse("5[mm"),
            Err(ExprError::Syntax { .. })
        ));
        assert!(matches!(
            Expression::parse(""),
            Err(ExprError::Syntax { .. })
        ));
    }

    #[test]
    fn hostile_input_is_bounded() {
        let deep = format!("{}1{}", "(".repeat(10_000), ")".repeat(10_000));
        assert_eq!(Expression::parse(&deep), Err(ExprError::TooLong));
        let deep = format!("{}1{}", "(".repeat(200), ")".repeat(200));
        assert_eq!(Expression::parse(&deep), Err(ExprError::TooDeep));
        let minus = format!("{}1", "-".repeat(500));
        assert_eq!(Expression::parse(&minus), Err(ExprError::TooDeep));
    }

    #[test]
    fn references_are_collected_once() {
        let e = Expression::parse("a + b * a + sin(c)").unwrap();
        let refs: Vec<_> = e.references().into_iter().collect();
        assert_eq!(refs, vec!["a", "b", "c"]);
    }

    #[test]
    fn name_rules() {
        assert!(is_valid_name("p12"));
        assert!(is_valid_name("_hole_dia"));
        for bad in ["", "1abc", "has space", "sin", "pi", "a-b", &"x".repeat(65)] {
            assert!(!is_valid_name(bad), "{bad}");
        }
    }
}
