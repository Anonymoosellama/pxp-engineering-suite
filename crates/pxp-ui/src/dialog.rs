// SPDX-License-Identifier: LGPL-2.1-or-later
//! The dialog framework: NX-style feature dialogs as data.
//!
//! A dialog is a title, a cue (what to do next, shown on the cue line),
//! groups of fields, and the three NX buttons OK / Apply / Cancel. MB2 in
//! the graphics window means OK; Esc means Cancel. Every field is a
//! formula: a number, `25 mm`, `p0 * 2`; a bare number takes the part's
//! unit, as NX does. Values are checked by creating the feature in a
//! transaction: if the document refuses (bad formula, unit mismatch), the
//! dialog stays open with the reason under the fields and nothing changes.
//!
//! The active field is mirrored in an on-screen input box beside the
//! cursor in the graphics window, so values can be typed without leaving
//! the model.

use pxp_app_modeling::session::Primitive;

/// What the user did with the dialog this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing.
    None,
    /// Create and close.
    Ok,
    /// Create and keep the dialog open for another.
    Apply,
    /// Close without creating.
    Cancel,
}

/// One input field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// Label (NX wording).
    pub label: &'static str,
    /// The formula typed.
    pub value: String,
    /// A length (gets the part unit) or a plain number.
    pub is_length: bool,
}

/// A selection step that fills three fields with a picked point (NX's
/// "Specify Point"). While active, MB1 in the graphics window picks, the
/// snap toolbar decides what a click snaps to, and the cue line says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointStep {
    /// Label on the step's button.
    pub label: &'static str,
    /// The X, Y, Z fields it fills.
    pub fields: [usize; 3],
    /// Waiting for a click.
    pub active: bool,
}

/// A feature dialog.
#[derive(Debug, Clone)]
pub struct Dialog {
    /// The feature it creates.
    pub kind: Primitive,
    /// Groups of fields: (group caption, field indices).
    pub groups: Vec<(&'static str, Vec<usize>)>,
    /// Every field.
    pub fields: Vec<Field>,
    /// Optional feature name (empty: `Block(3)` and so on).
    pub name: String,
    /// Why the last OK/Apply was refused.
    pub error: Option<String>,
    /// Index of the field last edited (the on-screen input box shows it).
    pub active: usize,
    /// The point selection step, active when the dialog opens.
    pub step: Option<PointStep>,
}

impl Dialog {
    /// The dialog for a primitive, with NX-like defaults scaled to the
    /// part's unit (an inch part gets 4 in instead of 100 mm).
    #[must_use]
    pub fn new(kind: Primitive, inch: bool) -> Self {
        let fields: Vec<Field> = kind
            .fields()
            .iter()
            .zip(kind.defaults())
            .map(|((label, is_length), default)| {
                let value = if inch && *is_length {
                    inch_default(default)
                } else {
                    (*default).to_owned()
                };
                Field {
                    label,
                    value,
                    is_length: *is_length,
                }
            })
            .collect();
        let groups = match kind {
            Primitive::Block => vec![("Origin", vec![0, 1, 2]), ("Dimensions", vec![3, 4, 5])],
            Primitive::Cylinder => {
                vec![("Axis", vec![0, 1, 2, 3, 4, 5]), ("Dimensions", vec![6, 7])]
            }
            Primitive::Cone => vec![
                ("Axis", vec![0, 1, 2, 3, 4, 5]),
                ("Dimensions", vec![6, 7, 8]),
            ],
            Primitive::Sphere => vec![("Center Point", vec![0, 1, 2]), ("Dimensions", vec![3])],
        };
        let active = groups
            .get(1)
            .and_then(|(_, f)| f.first())
            .copied()
            .unwrap_or(0);
        let label = match kind {
            Primitive::Block => "Specify Point (origin)",
            Primitive::Cylinder | Primitive::Cone => "Specify Point (base)",
            Primitive::Sphere => "Specify Point (center)",
        };
        let step = Some(PointStep {
            label,
            fields: [0, 1, 2],
            active: true,
        });
        Self {
            kind,
            groups,
            fields,
            name: String::new(),
            error: None,
            active,
            step,
        }
    }

    /// Dialog title.
    #[must_use]
    pub fn title(&self) -> &'static str {
        self.kind.label()
    }

    /// Cue-line text for this dialog: the active selection step first.
    #[must_use]
    pub fn cue(&self) -> String {
        match self.step {
            Some(st) if st.active => format!(
                "{}: {} — click a face, an edge's end or middle, or a datum origin; or type the values",
                self.kind.label(),
                st.label
            ),
            _ => format!(
                "{}: type the values (a bare number is in the part's unit), then OK or MB2; Esc cancels",
                self.kind.label()
            ),
        }
    }

    /// Fills the point step's fields with `p` (mm), written in the part's
    /// unit, and completes the step.
    pub fn set_point(&mut self, p: [f64; 3], inch: bool) {
        let Some(st) = self.step.as_mut() else { return };
        for (i, v) in st.fields.iter().zip(p) {
            if let Some(f) = self.fields.get_mut(*i) {
                f.value = format_length(if inch { v / 25.4 } else { v });
            }
        }
        st.active = false;
    }

    /// The values in field order, as the session takes them.
    #[must_use]
    pub fn values(&self) -> Vec<String> {
        self.fields.iter().map(|f| f.value.clone()).collect()
    }

    /// A field is empty: OK would certainly fail, so it is refused here
    /// with the field's name.
    #[must_use]
    pub fn missing(&self) -> Option<&'static str> {
        self.fields
            .iter()
            .find(|f| f.value.trim().is_empty())
            .map(|f| f.label)
    }
}

/// A picked coordinate as typed text: 4 decimals (0.1 µm in mm parts;
/// face points come from a 32-bit depth buffer, good to about 1e-6
/// relative), trailing zeros dropped, so `12.5000` reads `12.5` and
/// `-0.00001` reads `0`.
fn format_length(v: f64) -> String {
    let t = format!("{v:.4}");
    let t = t.trim_end_matches('0').trim_end_matches('.');
    if t == "-0" {
        "0".to_owned()
    } else {
        t.to_owned()
    }
}

/// Converts a millimetre default to a round inch value (100 → 4).
fn inch_default(mm: &str) -> String {
    match mm.parse::<f64>() {
        Ok(v) if v.abs() < 1e-12 => "0".to_owned(),
        Ok(v) => format!("{}", (v / 25.0).round().max(1.0)),
        Err(_) => mm.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_point_step_fills_the_origin_in_the_part_unit() {
        let mut d = Dialog::new(Primitive::Block, false);
        assert!(
            d.step.is_some_and(|s| s.active),
            "the dialog opens waiting for a point"
        );
        assert!(d.cue().contains("Specify Point (origin)"));
        d.set_point([12.5, -0.000_01, 99.999_999], false);
        assert_eq!(d.values()[..3], ["12.5", "0", "100"]);
        assert!(d.step.is_some_and(|s| !s.active));
        assert!(!d.cue().contains("Specify Point"));
        let mut i = Dialog::new(Primitive::Sphere, true);
        i.set_point([25.4, 50.8, 0.0], true);
        assert_eq!(i.values()[..3], ["1", "2", "0"]);
    }

    #[test]
    fn block_dialog_matches_nx_layout() {
        let d = Dialog::new(Primitive::Block, false);
        assert_eq!(d.title(), "Block");
        assert_eq!(
            d.groups.iter().map(|g| g.0).collect::<Vec<_>>(),
            ["Origin", "Dimensions"]
        );
        assert_eq!(d.values(), ["0", "0", "0", "100", "100", "100"]);
        assert_eq!(
            d.fields.get(d.active).map(|f| f.label),
            Some("Length (XC)"),
            "typing starts at the first dimension"
        );
        assert!(d.missing().is_none());
        let inch = Dialog::new(Primitive::Block, true);
        assert_eq!(inch.values(), ["0", "0", "0", "4", "4", "4"]);
        let mut c = Dialog::new(Primitive::Cylinder, true);
        assert_eq!(
            c.values()[3..6],
            ["0", "0", "1"],
            "axis components are not lengths"
        );
        if let Some(f) = c.fields.get_mut(6) {
            f.value.clear();
        }
        assert_eq!(c.missing(), Some("Diameter"));
    }
}
