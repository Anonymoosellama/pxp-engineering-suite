// SPDX-License-Identifier: LGPL-2.1-or-later
//! Keyboard chords and the remappable shortcut table.
//!
//! A chord is written `Ctrl+Shift+D`: modifiers in the fixed order Ctrl,
//! Alt, Shift, then one key. The table starts from each command's default
//! shortcut (its `CommandSpec`), applies customer-default overrides
//! (`shortcut.<command id> = <chord>`, empty to remove), and reports
//! conflicts instead of letting the last one silently win.

use std::collections::BTreeMap;

/// A key chord.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    /// Ctrl (Cmd on macOS) held.
    pub ctrl: bool,
    /// Alt held.
    pub alt: bool,
    /// Shift held.
    pub shift: bool,
    /// The key's name, as written in chords (`M`, `F8`, `Home`).
    pub key: String,
}

impl Chord {
    /// Parses `Ctrl+Shift+D` (case-insensitive modifiers; `Cmd` = Ctrl).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut c = Self {
            ctrl: false,
            alt: false,
            shift: false,
            key: String::new(),
        };
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let (key, mods) = parts.split_last()?;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" | "cmd" | "command" => c.ctrl = true,
                "alt" | "option" => c.alt = true,
                "shift" => c.shift = true,
                _ => return None,
            }
        }
        if key.is_empty() {
            return None;
        }
        // Single letters are stored upper-case so `ctrl+m` = `Ctrl+M`.
        c.key = if key.chars().count() == 1 {
            key.to_ascii_uppercase()
        } else {
            (*key).to_owned()
        };
        Some(c)
    }
}

impl std::fmt::Display for Chord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        f.write_str(&self.key)
    }
}

/// Command shortcuts after overrides.
#[derive(Debug, Clone, Default)]
pub struct ShortcutMap {
    by_chord: BTreeMap<Chord, String>,
    /// (chord, the command kept, the command that lost), in the order found.
    pub conflicts: Vec<(Chord, String, String)>,
    /// Override values that are not chords.
    pub invalid: Vec<(String, String)>,
}

impl ShortcutMap {
    /// Builds the map: `defaults` are (command id, chord) from the
    /// commands; `overrides` from customer defaults (an empty chord
    /// removes the command's shortcut).
    #[must_use]
    pub fn build(defaults: &[(String, String)], overrides: &[(String, String)]) -> Self {
        let mut chosen: BTreeMap<String, Option<Chord>> = BTreeMap::new();
        let mut map = Self::default();
        for (id, text) in defaults {
            chosen.insert(id.clone(), Chord::parse(text));
        }
        for (id, text) in overrides {
            if text.trim().is_empty() {
                chosen.insert(id.clone(), None);
            } else if let Some(c) = Chord::parse(text) {
                chosen.insert(id.clone(), Some(c));
            } else {
                map.invalid.push((id.clone(), text.clone()));
            }
        }
        // Overridden commands claim their chord first, so a remap moves a
        // shortcut away from a default holder rather than losing to it.
        let overridden: Vec<&String> = overrides.iter().map(|(id, _)| id).collect();
        let mut order: Vec<(&String, &Option<Chord>)> = chosen.iter().collect();
        order.sort_by_key(|(id, _)| !overridden.contains(id));
        for (id, chord) in order {
            let Some(c) = chord else { continue };
            if let Some(holder) = map.by_chord.get(c) {
                map.conflicts.push((c.clone(), holder.clone(), id.clone()));
            } else {
                map.by_chord.insert(c.clone(), id.clone());
            }
        }
        map
    }

    /// The command bound to a chord.
    #[must_use]
    pub fn command(&self, chord: &Chord) -> Option<&str> {
        self.by_chord.get(chord).map(String::as_str)
    }

    /// The chord bound to a command (for tooltips and the Command Finder).
    #[must_use]
    pub fn chord_of(&self, id: &str) -> Option<&Chord> {
        self.by_chord
            .iter()
            .find(|(_, c)| c.as_str() == id)
            .map(|(k, _)| k)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn chords_parse_and_print_canonically() {
        let c = Chord::parse("shift+ctrl+d").unwrap();
        assert_eq!(c.to_string(), "Ctrl+Shift+D");
        assert_eq!(Chord::parse("Cmd+M"), Chord::parse("Ctrl+M"));
        assert_eq!(Chord::parse("F8").unwrap().to_string(), "F8");
        assert!(Chord::parse("Hyper+X").is_none());
        assert!(Chord::parse("Ctrl+").is_none());
    }

    #[test]
    fn overrides_move_shortcuts_and_conflicts_are_reported() {
        let s = |a: &str, b: &str| (a.to_owned(), b.to_owned());
        let defaults = [
            s("app.modeling", "Ctrl+M"),
            s("modeling.block", ""),
            s("modeling.extrude", "X"),
        ];
        // Give Block the X key: Extrude loses it, and the conflict is listed.
        let m = ShortcutMap::build(
            &defaults,
            &[
                s("modeling.block", "x"),
                s("modeling.sketch", "Ctrl+Hyper+Q"),
            ],
        );
        assert_eq!(
            m.command(&Chord::parse("X").unwrap()),
            Some("modeling.block")
        );
        assert_eq!(m.conflicts.len(), 1);
        assert_eq!(m.conflicts[0].2, "modeling.extrude");
        assert_eq!(m.invalid.len(), 1);
        assert_eq!(m.chord_of("app.modeling").unwrap().to_string(), "Ctrl+M");
        // An empty override removes a shortcut.
        let m = ShortcutMap::build(&defaults, &[s("app.modeling", "")]);
        assert!(m.command(&Chord::parse("Ctrl+M").unwrap()).is_none());
    }
}
