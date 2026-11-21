// SPDX-License-Identifier: LGPL-2.1-or-later
//! A minimal ISO 10303-21 reader that finds a file's **geometric validation
//! properties**: the volume, surface area and centroid the exporting CAD
//! system wrote into the file (CAx-IF "Recommended Practices for Geometric
//! and Assembly Validation Properties").
//!
//! This is a **test oracle**, not PXP's STEP translator (that is
//! `pxp-io-step`, Stage 1). It exists so the bake-off can grade a kernel's
//! STEP import against numbers that never came from the kernel under test:
//! the originating CAD system computed them when it wrote the file.
//!
//! ## What it reads
//!
//! ```text
//! #pd  = PROPERTY_DEFINITION('geometric validation property', …, #shape);
//! #pdr = PROPERTY_DEFINITION_REPRESENTATION(#pd, #rep);
//! #rep = REPRESENTATION('volume' | 'surface area' | 'centroid', (#item, …), #ctx);
//! #item = MEASURE_REPRESENTATION_ITEM('volume measure', VOLUME_MEASURE(v), #unit)
//!       | MEASURE_REPRESENTATION_ITEM('surface area measure', AREA_MEASURE(a), #unit)
//!       | CARTESIAN_POINT('centre point', (x, y, z))      (in #ctx's length unit)
//! ```
//!
//! Units are followed to millimetres: SI units with any prefix, derived
//! units (`DERIVED_UNIT` of `DERIVED_UNIT_ELEMENT`s with exponents),
//! conversion-based units (inch, foot) through their `…_MEASURE_WITH_UNIT`,
//! and the `SQUARE_METRE`/`CUBIC_METRE` SI names. A value whose unit cannot
//! be resolved is dropped with a note, never guessed.
//!
//! ## Must never
//!
//! - Panic or loop forever on malformed text (it reads untrusted files):
//!   unknown syntax ends the record, reference chains are depth-limited.

use std::collections::BTreeMap;

/// A parsed Part 21 value.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    /// `#123`
    Ref(u64),
    /// `'text'`
    Str(String),
    /// `1.5`, `-3`, `2.E-3`
    Num(f64),
    /// `.MILLI.`
    Enum(String),
    /// `(a, b, …)`; also a complex entity instance `(A(…) B(…))`.
    List(Vec<Value>),
    /// `NAME(args)`: an entity or a typed value such as `VOLUME_MEASURE(1.)`.
    Typed(String, Vec<Value>),
    /// `$` or `*`
    Omitted,
}

impl Value {
    fn typed(&self, name: &str) -> Option<&[Value]> {
        match self {
            Self::Typed(n, args) if n == name => Some(args),
            // A complex instance: look among its partial entities.
            Self::List(parts) => parts.iter().find_map(|p| p.typed(name)),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    fn as_id(&self) -> Option<u64> {
        match self {
            Self::Ref(r) => Some(*r),
            _ => None,
        }
    }

    fn as_num(&self) -> Option<f64> {
        match self {
            Self::Num(v) => Some(*v),
            // A typed measure, e.g. `VOLUME_MEASURE(1.5)` or `LENGTH_MEASURE(25.4)`.
            Self::Typed(_, args) => match args.as_slice() {
                [only] => only.as_num(),
                _ => None,
            },
            _ => None,
        }
    }

    fn as_list(&self) -> Option<&[Value]> {
        match self {
            Self::List(v) => Some(v),
            _ => None,
        }
    }
}

/// Character cursor over one record's right-hand side.
struct Cursor<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl Cursor<'_> {
    fn skip_ws(&mut self) {
        while self.chars.peek().is_some_and(|c| c.is_whitespace()) {
            self.chars.next();
        }
    }

    fn eat(&mut self, c: char) -> bool {
        self.skip_ws();
        if self.chars.peek() == Some(&c) {
            self.chars.next();
            true
        } else {
            false
        }
    }

    fn word(&mut self, first: char) -> String {
        let mut s = String::from(first);
        while let Some(&c) = self.chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                s.push(c);
                self.chars.next();
            } else {
                break;
            }
        }
        s
    }

    /// Comma-separated values up to the closing parenthesis (consumed).
    /// Inside a complex instance the members are separated by spaces only,
    /// so a missing comma is accepted.
    fn list(&mut self, depth: usize) -> Option<Vec<Value>> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            match self.chars.peek() {
                Some(')') => {
                    self.chars.next();
                    return Some(out);
                }
                Some(',') => {
                    self.chars.next();
                }
                None => return None,
                Some(_) => out.push(self.value(depth + 1)?),
            }
        }
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth > 64 {
            return None;
        }
        self.skip_ws();
        let c = self.chars.next()?;
        Some(match c {
            '#' => {
                let digits = self.word('0');
                Value::Ref(digits.parse().ok()?)
            }
            '\'' => {
                let mut s = String::new();
                loop {
                    match self.chars.next()? {
                        '\'' if self.chars.peek() == Some(&'\'') => {
                            self.chars.next();
                            s.push('\'');
                        }
                        '\'' => break,
                        other => s.push(other),
                    }
                }
                Value::Str(s)
            }
            '.' => {
                let mut s = String::new();
                loop {
                    match self.chars.next()? {
                        '.' => break,
                        other => s.push(other),
                    }
                }
                Value::Enum(s)
            }
            '(' => Value::List(self.list(depth)?),
            '$' | '*' => Value::Omitted,
            c if c.is_ascii_digit() || c == '-' || c == '+' => {
                let mut s = String::from(c);
                while let Some(&d) = self.chars.peek() {
                    if d.is_ascii_digit() || matches!(d, '.' | 'E' | 'e' | '-' | '+') {
                        s.push(d);
                        self.chars.next();
                    } else {
                        break;
                    }
                }
                // Part 21 allows "1." and "2.E-3"; Rust's parser wants a
                // digit after the point.
                let fixed = s.replace(".E", ".0E").replace(".e", ".0e");
                Value::Num(fixed.strip_suffix('.').unwrap_or(&fixed).parse().ok()?)
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let name = self.word(c).to_ascii_uppercase();
                if self.eat('(') {
                    Value::Typed(name, self.list(depth)?)
                } else {
                    // A bare keyword (not valid Part 21 outside a list, but
                    // harmless to keep).
                    Value::Typed(name, Vec::new())
                }
            }
            _ => return None,
        })
    }
}

/// The DATA section as `id → instance`, comments and strings respected.
fn records(text: &str) -> BTreeMap<u64, Value> {
    let mut out = BTreeMap::new();
    let Some(start) = text.find("DATA;") else {
        return out;
    };
    let body = text.get(start + 5..).unwrap_or("");
    let mut rec = String::new();
    let mut chars = body.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            rec.push(c);
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    rec.push('\'');
                    chars.next();
                } else {
                    in_str = false;
                }
            }
            continue;
        }
        match c {
            '\'' => {
                in_str = true;
                rec.push(c);
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for d in chars.by_ref() {
                    if prev == '*' && d == '/' {
                        break;
                    }
                    prev = d;
                }
            }
            ';' => {
                let r = rec.trim();
                if r.starts_with("ENDSEC") {
                    break;
                }
                if let Some((id, rhs)) = r.strip_prefix('#').and_then(|r| r.split_once('='))
                    && let Ok(id) = id.trim().parse::<u64>()
                {
                    let mut cur = Cursor {
                        chars: rhs.chars().peekable(),
                    };
                    if let Some(v) = cur.value(0) {
                        out.insert(id, v);
                    }
                }
                rec.clear();
            }
            _ => rec.push(c),
        }
    }
    out
}

/// What a file says about its own geometry, in millimetres.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ValidationProperties {
    /// Volume of the part's solids, mm³.
    pub volume: Option<f64>,
    /// Surface area of the part's solids, mm².
    pub area: Option<f64>,
    /// Centroid, mm, in the part's own coordinates.
    pub centroid: Option<[f64; 3]>,
    /// Every solid volume value found, in file order, before choosing
    /// (diagnostic).
    pub volumes: Vec<f64>,
    /// Set when the file labels its volume or area in a unit whose length
    /// differs from its model's own length unit: the model's length unit
    /// divided by the label's (25.4 for properties labelled in mm in an
    /// inch model). The values above are as labelled; a grader should not
    /// hold a kernel to them.
    pub unit_mismatch: Option<f64>,
    /// Values that were found but could not be used, and why.
    pub notes: Vec<String>,
    /// The bodies the file declares, by kind.
    pub declared: Declared,
}

/// The bodies a STEP file declares, counted from its entities (not from
/// any kernel's reading of them), so an import can be graded on what it
/// should have produced.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Declared {
    /// Solids: `MANIFOLD_SOLID_BREP` (with `BREP_WITH_VOIDS`) and
    /// `FACETED_BREP` instances.
    pub solids: usize,
    /// `SHELL_BASED_SURFACE_MODEL` instances: sheet bodies by design (CAD
    /// systems export construction surfaces and datum target areas this
    /// way), never solids.
    pub surface_models: usize,
    /// Shells listed by those surface models.
    pub surface_shells: usize,
    /// AP242 tessellated solids and shells (meshes, no exact geometry).
    pub tessellated: usize,
}

fn declared(recs: &BTreeMap<u64, Value>) -> Declared {
    let mut d = Declared::default();
    for v in recs.values() {
        if ["MANIFOLD_SOLID_BREP", "BREP_WITH_VOIDS", "FACETED_BREP"]
            .iter()
            .any(|n| v.typed(n).is_some())
        {
            d.solids += 1;
        }
        if let Some(args) = v.typed("SHELL_BASED_SURFACE_MODEL") {
            d.surface_models += 1;
            d.surface_shells += args
                .get(1)
                .and_then(Value::as_list)
                .map_or(0, <[Value]>::len);
        }
        if ["TESSELLATED_SOLID", "TESSELLATED_SHELL"]
            .iter()
            .any(|n| v.typed(n).is_some())
        {
            d.tessellated += 1;
        }
    }
    d
}

impl ValidationProperties {
    /// True if the file states at least one usable property.
    #[must_use]
    pub fn any(&self) -> bool {
        self.volume.is_some() || self.area.is_some() || self.centroid.is_some()
    }
}

fn prefix_factor(p: &Value) -> Option<f64> {
    Some(match p {
        Value::Omitted => 1.0,
        Value::Enum(e) => match e.as_str() {
            "EXA" => 1e18,
            "PETA" => 1e15,
            "TERA" => 1e12,
            "GIGA" => 1e9,
            "MEGA" => 1e6,
            "KILO" => 1e3,
            "HECTO" => 1e2,
            "DECA" => 1e1,
            "DECI" => 1e-1,
            "CENTI" => 1e-2,
            "MILLI" => 1e-3,
            "MICRO" => 1e-6,
            "NANO" => 1e-9,
            "PICO" => 1e-12,
            _ => return None,
        },
        _ => return None,
    })
}

/// Millimetres-based factor of unit `id`: how many mm (for a length), mm²
/// (area) or mm³ (volume) one unit is.
fn unit_factor(recs: &BTreeMap<u64, Value>, id: u64, depth: usize) -> Option<f64> {
    if depth > 16 {
        return None;
    }
    let v = recs.get(&id)?;
    if let Some(args) = v.typed("SI_UNIT") {
        let [prefix, name] = args else { return None };
        let k = prefix_factor(prefix)?;
        return match name {
            // The prefix applies to the metre, then the power is taken
            // (ISO 10303-41: a prefixed SQUARE_METRE is a square of the
            // prefixed metre).
            Value::Enum(n) if n == "METRE" => Some(1e3 * k),
            Value::Enum(n) if n == "SQUARE_METRE" => Some((1e3 * k).powi(2)),
            Value::Enum(n) if n == "CUBIC_METRE" => Some((1e3 * k).powi(3)),
            _ => None,
        };
    }
    if let Some(args) = v.typed("CONVERSION_BASED_UNIT") {
        let [_, measure] = args else { return None };
        let m = recs.get(&measure.as_id()?)?;
        // LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4), #mm) or the
        // generic MEASURE_WITH_UNIT.
        let margs = match m {
            Value::Typed(_, a) => a.as_slice(),
            Value::List(parts) => parts.iter().find_map(|p| match p {
                Value::Typed(n, a) if n.ends_with("MEASURE_WITH_UNIT") && a.len() == 2 => {
                    Some(a.as_slice())
                }
                _ => None,
            })?,
            _ => return None,
        };
        let [value, base] = margs else { return None };
        return Some(value.as_num()? * unit_factor(recs, base.as_id()?, depth + 1)?);
    }
    let derived = ["DERIVED_UNIT", "AREA_UNIT", "VOLUME_UNIT"]
        .into_iter()
        .find_map(|n| v.typed(n).filter(|a| a.len() == 1));
    if let Some(args) = derived {
        let [elements] = args else { return None };
        let mut k = 1.0;
        for e in elements.as_list()? {
            let el = recs.get(&e.as_id()?)?.typed("DERIVED_UNIT_ELEMENT")?;
            let [unit, exponent] = el else { return None };
            let f = unit_factor(recs, unit.as_id()?, depth + 1)?;
            k *= f.powf(exponent.as_num()?);
        }
        return Some(k);
    }
    None
}

/// The length unit (mm per unit) of representation context `ctx`.
fn context_length(recs: &BTreeMap<u64, Value>, ctx: u64) -> Option<f64> {
    let units = recs.get(&ctx)?.typed("GLOBAL_UNIT_ASSIGNED_CONTEXT")?;
    let [list] = units else { return None };
    list.as_list()?.iter().find_map(|u| {
        let id = u.as_id()?;
        recs.get(&id)?.typed("LENGTH_UNIT")?;
        unit_factor(recs, id, 0)
    })
}

/// What one shape (a part's `PRODUCT_DEFINITION_SHAPE` or a
/// `SHAPE_ASPECT` of it) states about its solids.
#[derive(Debug, Clone, Copy, Default)]
struct Stated {
    volume: Option<f64>,
    area: Option<f64>,
    centroid: Option<[f64; 3]>,
}

/// The CAx-IF item names for a **solid's** properties. Sheet and wire
/// bodies use other names (`independent surface area measure`, `surface
/// centre point`, `curve length measure`, …), which a solid import is not
/// graded against.
fn solid_item(label: &str) -> Option<&'static str> {
    match label.trim().to_ascii_lowercase().as_str() {
        "volume measure" => Some("volume"),
        "surface area measure" | "wetted area measure" => Some("area"),
        "centre point" | "center point" => Some("centroid"),
        _ => None,
    }
}

fn positive(v: f64) -> Option<f64> {
    (v.is_finite() && v > 0.0).then_some(v)
}

/// The property definitions that are geometric validation properties, and
/// the shape each is attached to. One exporter swaps the name and
/// description arguments, so either may carry the marker.
fn definitions(recs: &BTreeMap<u64, Value>, notes: &mut Vec<String>) -> BTreeMap<u64, u64> {
    let is_marker = |v: &Value| {
        v.as_str().is_some_and(|s| {
            s.trim()
                .eq_ignore_ascii_case("geometric validation property")
        })
    };
    let mut defs = BTreeMap::new();
    let mut swapped = false;
    for (id, v) in recs {
        if let Some([name, description, def]) = v.typed("PROPERTY_DEFINITION")
            && let Some(shape) = def.as_id()
        {
            if is_marker(name) {
                defs.insert(*id, shape);
            } else if is_marker(description) {
                swapped = true;
                defs.insert(*id, shape);
            }
        }
    }
    if swapped {
        notes.push("the exporter swapped PROPERTY_DEFINITION name and description".into());
    }
    defs
}

/// One measure item of a solid (volume or area), converted to mm³ or mm².
/// Also notes when its unit's length differs from the model's own length
/// unit (its representation context): the file then labels the value in
/// one unit and models in another.
fn measure_item(
    recs: &BTreeMap<u64, Value>,
    item: u64,
    kind: &str,
    measure: &Value,
    unit: &Value,
    ctx: Option<u64>,
    out: &mut ValidationProperties,
) -> Option<f64> {
    let raw = measure.as_num()?;
    let Some(k) = unit.as_id().and_then(|u| unit_factor(recs, u, 0)) else {
        out.notes
            .push(format!("#{item} {kind}: unit could not be resolved"));
        return None;
    };
    let power = if kind == "volume" { 3.0 } else { 2.0 };
    if let Some(model) = ctx.and_then(|c| context_length(recs, c)) {
        let label = k.powf(1.0 / power);
        let ratio = model / label;
        if (ratio - 1.0).abs() > 1e-9 && out.unit_mismatch.is_none() {
            out.unit_mismatch = Some(ratio);
            out.notes.push(format!(
                "#{item} {kind} is labelled in a {label:.6} mm unit but the model's length unit is {model:.6} mm"
            ));
        }
    }
    Some(raw * k)
}

/// What each shape states about its solids.
fn stated(
    recs: &BTreeMap<u64, Value>,
    defs: &BTreeMap<u64, u64>,
    out: &mut ValidationProperties,
) -> BTreeMap<u64, Stated> {
    let mut stated: BTreeMap<u64, Stated> = BTreeMap::new();
    for v in recs.values() {
        let Some([def, rep]) = v.typed("PROPERTY_DEFINITION_REPRESENTATION") else {
            continue;
        };
        let (Some(&shape), Some(rep)) = (def.as_id().and_then(|d| defs.get(&d)), rep.as_id())
        else {
            continue;
        };
        let Some(Value::Typed(_, rargs)) = recs.get(&rep) else {
            continue;
        };
        let [_, items, ctx] = rargs.as_slice() else {
            continue;
        };
        let (Some(items), ctx) = (items.as_list(), ctx.as_id()) else {
            continue;
        };
        let entry = stated.entry(shape).or_default();
        for item in items.iter().filter_map(Value::as_id) {
            let Some(iv) = recs.get(&item) else { continue };
            if let Some([name, measure, unit]) = iv.typed("MEASURE_REPRESENTATION_ITEM") {
                let Some(kind) = name.as_str().and_then(solid_item) else {
                    continue;
                };
                let Some(value) = measure_item(recs, item, kind, measure, unit, ctx, out) else {
                    continue;
                };
                if kind == "volume" {
                    out.volumes.push(value);
                    entry.volume.get_or_insert(value);
                } else {
                    entry.area.get_or_insert(value);
                }
            } else if let Some([name, coords]) = iv.typed("CARTESIAN_POINT")
                && name.as_str().and_then(solid_item) == Some("centroid")
            {
                let k = ctx.and_then(|c| context_length(recs, c));
                let xyz: Option<Vec<f64>> = coords
                    .as_list()
                    .map(|l| l.iter().filter_map(Value::as_num).collect());
                match (k, xyz.as_deref()) {
                    (Some(k), Some(&[x, y, z])) => {
                        entry.centroid.get_or_insert([x * k, y * k, z * k]);
                    }
                    _ => out
                        .notes
                        .push(format!("#{item}: centroid unit or coordinates unreadable")),
                }
            }
        }
    }
    stated
}

/// Reads the geometric validation properties of a STEP file's solids.
///
/// Files state them on the part's `PRODUCT_DEFINITION_SHAPE` (everything
/// the part holds) and often again on `SHAPE_ASPECT`s for its solids and
/// for its surface bodies. The solids' aspects are used when they state a
/// volume, else the part's own values; adding both would count the part
/// twice. Zero values (some exporters write zeros for empty aspects) are
/// dropped with a note.
#[must_use]
pub fn validation_properties(text: &str) -> ValidationProperties {
    let recs = records(text);
    let mut out = ValidationProperties {
        declared: declared(&recs),
        ..ValidationProperties::default()
    };
    let defs = definitions(&recs, &mut out.notes);
    let stated = stated(&recs, &defs, &mut out);
    let is_aspect =
        |shape: &u64| matches!(recs.get(shape), Some(v) if v.typed("SHAPE_ASPECT").is_some());
    let zeros = stated
        .values()
        .filter(|s| s.volume.is_some() && s.volume.and_then(positive).is_none())
        .count();
    if zeros > 0 {
        out.notes.push(format!(
            "{zeros} shape(s) state a zero or invalid volume (ignored)"
        ));
    }
    let solids: Vec<Stated> = stated
        .iter()
        .filter(|(shape, s)| is_aspect(shape) && s.volume.and_then(positive).is_some())
        .map(|(_, s)| *s)
        .collect();
    let chosen: Vec<Stated> = if solids.is_empty() {
        stated
            .iter()
            .filter(|(shape, s)| {
                !is_aspect(shape)
                    && (s.volume.and_then(positive).is_some()
                        || s.area.and_then(positive).is_some())
            })
            .map(|(_, s)| *s)
            .collect()
    } else {
        solids
    };
    // A total is stated only if every chosen shape states it.
    let total = |f: &dyn Fn(&Stated) -> Option<f64>| -> Option<f64> {
        let values: Option<Vec<f64>> = chosen.iter().map(|s| f(s).and_then(positive)).collect();
        values.filter(|v| !v.is_empty()).map(|v| v.iter().sum())
    };
    out.volume = total(&|s| s.volume);
    out.area = total(&|s| s.area);
    // Volume-weighted centroid of the chosen solids.
    let weighted: Option<Vec<([f64; 3], f64)>> = chosen
        .iter()
        .map(|s| Some((s.centroid?, s.volume.and_then(positive).unwrap_or(1.0))))
        .collect();
    if let Some(w) = weighted.filter(|w| !w.is_empty()) {
        let sum: f64 = w.iter().map(|(_, v)| v).sum();
        let mut c = [0.0; 3];
        for (p, v) in &w {
            for (ci, pi) in c.iter_mut().zip(p) {
                *ci += pi * v / sum;
            }
        }
        out.centroid = Some(c);
    }
    if chosen.len() > 1 {
        out.notes
            .push(format!("{} solids' properties combined", chosen.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const MM_FILE: &str = "ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n\
        #1=PROPERTY_DEFINITION('geometric validation property','volume of part',#9);\n\
        #2=PROPERTY_DEFINITION_REPRESENTATION(#1,#3);\n\
        #3=REPRESENTATION('volume',(#4),#20);\n\
        #4=MEASURE_REPRESENTATION_ITEM('volume measure',VOLUME_MEASURE(1500.),#5);\n\
        #5=DERIVED_UNIT((#6));\n#6=DERIVED_UNIT_ELEMENT(#7,3.);\n\
        #7=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\n\
        /* a comment; with a semicolon */\n\
        #10=PROPERTY_DEFINITION('geometric validation property','centroid',#9);\n\
        #11=PROPERTY_DEFINITION_REPRESENTATION(#10,#12);\n\
        #12=REPRESENTATION('centroid',(#13),#20);\n\
        #13=CARTESIAN_POINT('centre point',(1.,2.5,-3.E-1));\n\
        #20=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#7,#21))\
        REPRESENTATION_CONTEXT('it''s',''));\n\
        #21=(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.));\n\
        #30=PROPERTY_DEFINITION('pmi validation property','',#9);\nENDSEC;\nEND-ISO-10303-21;\n";

    #[test]
    fn reads_millimetre_volume_and_centroid() {
        let p = validation_properties(MM_FILE);
        assert_eq!(p.volume, Some(1500.0));
        assert_eq!(p.centroid, Some([1.0, 2.5, -0.3]));
        assert_eq!(p.area, None);
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    #[test]
    fn converts_inches() {
        let text = "DATA;\n\
            #1=PROPERTY_DEFINITION('geometric validation property','',#9);\n\
            #2=PROPERTY_DEFINITION_REPRESENTATION(#1,#3);\n\
            #3=REPRESENTATION('surface area',(#4),#20);\n\
            #4=MEASURE_REPRESENTATION_ITEM('surface area measure',AREA_MEASURE(2.),#5);\n\
            #5=DERIVED_UNIT((#6));\n#6=DERIVED_UNIT_ELEMENT(#7,2.);\n\
            #7=(CONVERSION_BASED_UNIT('INCH',#8)LENGTH_UNIT()NAMED_UNIT(#40));\n\
            #8=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#11);\n\
            #11=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\nENDSEC;\n";
        let p = validation_properties(text);
        let a = p.area.unwrap();
        assert!((a - 2.0 * 25.4 * 25.4).abs() < 1e-9, "{a}");
    }

    /// The layouts the NIST files use: the part states its area including a
    /// surface body (CTC-05 AP203), a solid aspect states the solid's, a
    /// sheet aspect uses the "independent" names, an empty aspect states
    /// zeros (STC-06), one definition swaps name and description (STC-09
    /// 2024), and units are `AREA_UNIT`/`VOLUME_UNIT` in inches.
    #[test]
    fn prefers_solid_aspects_and_ignores_sheets_and_zeros() {
        let text = "DATA;\n\
            #1=PRODUCT_DEFINITION_SHAPE('','part',#2);\n\
            #3=SHAPE_ASPECT('','solid data',#1,.F.);\n\
            #4=SHAPE_ASPECT('','surface data',#1,.F.);\n\
            #5=SHAPE_ASPECT('','',#1,.F.);\n\
            #10=PROPERTY_DEFINITION('geometric validation property','area of part',#1);\n\
            #11=PROPERTY_DEFINITION_REPRESENTATION(#10,#12);\n\
            #12=REPRESENTATION('surface area',(#13,#14),#90);\n\
            #13=MEASURE_REPRESENTATION_ITEM('surface area measure',AREA_MEASURE(12.5),#80);\n\
            #14=MEASURE_REPRESENTATION_ITEM('volume measure',VOLUME_MEASURE(2.),#81);\n\
            #20=PROPERTY_DEFINITION('volume of solid','geometric validation property',#3);\n\
            #21=PROPERTY_DEFINITION_REPRESENTATION(#20,#22);\n\
            #22=REPRESENTATION('volume',(#23,#24,#25),#90);\n\
            #23=MEASURE_REPRESENTATION_ITEM('volume measure',VOLUME_MEASURE(2.),#81);\n\
            #24=MEASURE_REPRESENTATION_ITEM('wetted area measure',AREA_MEASURE(10.),#80);\n\
            #25=CARTESIAN_POINT('centre point',(1.,0.,0.));\n\
            #30=PROPERTY_DEFINITION('geometric validation property','',#4);\n\
            #31=PROPERTY_DEFINITION_REPRESENTATION(#30,#32);\n\
            #32=REPRESENTATION('surface area',(#33,#34),#90);\n\
            #33=MEASURE_REPRESENTATION_ITEM('independent surface area measure',AREA_MEASURE(2.5),#80);\n\
            #34=CARTESIAN_POINT('surface centre point',(9.,9.,9.));\n\
            #40=PROPERTY_DEFINITION('geometric validation property','',#5);\n\
            #41=PROPERTY_DEFINITION_REPRESENTATION(#40,#42);\n\
            #42=REPRESENTATION('',(#43,#44),#90);\n\
            #43=MEASURE_REPRESENTATION_ITEM('volume measure',VOLUME_MEASURE(0.0),#81);\n\
            #44=CARTESIAN_POINT('centre point',(0.,0.,0.));\n\
            #80=AREA_UNIT((#82));\n#81=VOLUME_UNIT((#83));\n\
            #82=DERIVED_UNIT_ELEMENT(#84,2.);\n#83=DERIVED_UNIT_ELEMENT(#84,3.);\n\
            #84=(CONVERSION_BASED_UNIT('INCH',#85)LENGTH_UNIT()NAMED_UNIT(#86));\n\
            #85=LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),#87);\n\
            #87=(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.));\n\
            #90=(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNIT_ASSIGNED_CONTEXT((#84))REPRESENTATION_CONTEXT('',''));\n\
            ENDSEC;\n";
        let p = validation_properties(text);
        let inch = 25.4_f64;
        assert!((p.volume.unwrap() - 2.0 * inch.powi(3)).abs() < 1e-6);
        // The solid's 10 in², not the part's 12.5 (solid + sheet) nor a sum.
        assert!((p.area.unwrap() - 10.0 * inch * inch).abs() < 1e-6);
        let c = p.centroid.unwrap();
        assert!((c[0] - inch).abs() < 1e-12 && c[1] == 0.0 && c[2] == 0.0);
        assert!(
            p.notes.iter().any(|n| n.contains("swapped")),
            "{:?}",
            p.notes
        );
        assert!(p.notes.iter().any(|n| n.contains("zero")), "{:?}", p.notes);
    }

    #[test]
    fn counts_declared_bodies() {
        let text = "DATA;\n\
            #1=MANIFOLD_SOLID_BREP('',#9);\n\
            #2=BREP_WITH_VOIDS('',#9,(#8));\n\
            #3=SHELL_BASED_SURFACE_MODEL('',(#7,#6));\n\
            #4=SHELL_BASED_SURFACE_MODEL('',(#5));\n\
            #10=TESSELLATED_SOLID('',(#11),$);\nENDSEC;\n";
        let d = validation_properties(text).declared;
        assert_eq!(
            d,
            Declared {
                solids: 2,
                surface_models: 2,
                surface_shells: 3,
                tessellated: 1
            }
        );
    }

    #[test]
    fn unresolved_units_are_noted_not_guessed() {
        let text = "DATA;\n\
            #1=PROPERTY_DEFINITION('geometric validation property','',#9);\n\
            #2=PROPERTY_DEFINITION_REPRESENTATION(#1,#3);\n\
            #3=REPRESENTATION('volume',(#4),#20);\n\
            #4=MEASURE_REPRESENTATION_ITEM('volume measure',VOLUME_MEASURE(7.),#99);\nENDSEC;\n";
        let p = validation_properties(text);
        assert_eq!(p.volume, None);
        assert_eq!(p.notes.len(), 1);
    }

    /// Prints what every STEP file under `$PXP_STEP_DIR` states about
    /// itself, and fails if a file names a geometric validation property
    /// but none could be read. Run with
    /// `PXP_STEP_DIR=… cargo test -p pxp-kernel -- --ignored --nocapture step_dir`.
    #[test]
    #[ignore = "needs STEP files on disk (PXP_STEP_DIR)"]
    fn step_dir_validation_properties() {
        let dir = std::env::var("PXP_STEP_DIR").expect("set PXP_STEP_DIR");
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("stp")))
            .collect();
        files.sort();
        let mut unread = Vec::new();
        for f in files {
            let text = String::from_utf8_lossy(&std::fs::read(&f).unwrap()).into_owned();
            let p = validation_properties(&text);
            println!(
                "{}\tV={:?}\tA={:?}\tC={:?}\t{:?}",
                f.file_name().unwrap().to_string_lossy(),
                p.volume,
                p.area,
                p.centroid,
                p.notes
            );
            let names_one = text
                .to_ascii_lowercase()
                .contains("'geometric validation property'");
            let zeros_only = p.notes.iter().any(|n| n.contains("zero"));
            if names_one && !p.any() && !zeros_only {
                unread.push(f);
            }
        }
        assert!(unread.is_empty(), "properties named but unread: {unread:?}");
    }

    #[test]
    fn malformed_text_does_not_panic() {
        for t in [
            "",
            "DATA;",
            "DATA;#1=(((((((;",
            "DATA;#1='unterminated",
            "DATA;#x=FOO();#2=BAR(#;ENDSEC;",
        ] {
            let p = validation_properties(t);
            assert!(!p.any());
        }
    }
}
