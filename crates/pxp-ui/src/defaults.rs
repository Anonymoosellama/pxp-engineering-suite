// SPDX-License-Identifier: LGPL-2.1-or-later
//! Layered customer defaults, as in NX: a value set at a higher level
//! overrides the levels below it.
//!
//! | Level | Where | Who edits it |
//! | --- | --- | --- |
//! | Shipped | compiled in ([`SHIPPED`]) | nobody |
//! | Site | the file named by `PXP_SITE_DEFAULTS` | the CAD administrator |
//! | User | `<config dir>/pxp/user_defaults.txt` | the user, from the Customer Defaults window |
//!
//! The file format is one `key = value` per line; `#` starts a comment;
//! blank lines are ignored. Unknown keys are kept (a newer version's
//! settings survive an older version's save) and listed as unknown.
//! A site file can lock a key with `!` (`theme != dark`): the user level
//! then cannot override it, and the window shows it as locked.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where a value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Compiled in.
    Shipped,
    /// The site file.
    Site,
    /// The user file.
    User,
}

impl Level {
    /// Label for the Customer Defaults window.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Shipped => "Shipped",
            Self::Site => "Site",
            Self::User => "User",
        }
    }
}

/// Every known key with its shipped value and a one-line description.
pub const SHIPPED: [(&str, &str, &str); 9] = [
    ("units", "mm", "Units of new parts: mm or in"),
    ("theme", "light", "Colour theme: light or dark"),
    (
        "role",
        "advanced",
        "Role: essentials (Home tab only) or advanced (every tab)",
    ),
    (
        "autosave_minutes",
        "5",
        "Minutes between autosaves of a modified part (0 turns autosave off)",
    ),
    ("edges", "true", "Draw B-rep edges in the graphics window"),
    (
        "silhouettes",
        "true",
        "Draw silhouette outlines in the graphics window",
    ),
    (
        "projection",
        "orthographic",
        "View projection: orthographic or perspective",
    ),
    (
        "pick_radius_px",
        "4",
        "Pixels a click may miss an object by and still select it (Stage 1)",
    ),
    ("recent_files", "8", "Files listed under File > Recent"),
];

/// Keys that remap a command's shortcut start with this prefix:
/// `shortcut.modeling.block = Ctrl+B`.
pub const SHORTCUT_PREFIX: &str = "shortcut.";

/// One parsed defaults file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Layer {
    /// Values by key.
    pub values: BTreeMap<String, String>,
    /// Keys locked at this level (site files only).
    pub locked: Vec<String>,
    /// Lines that could not be read, with their numbers.
    pub errors: Vec<(usize, String)>,
}

impl Layer {
    /// Parses the file format (see the module docs).
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let mut layer = Self::default();
        for (n, raw) in text.lines().enumerate() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                layer.errors.push((n + 1, format!("no '=' in {line:?}")));
                continue;
            };
            let (key, locked) = match k.trim().strip_suffix('!') {
                Some(key) => (key.trim(), true),
                None => (k.trim(), false),
            };
            if key.is_empty() {
                layer.errors.push((n + 1, "empty key".into()));
                continue;
            }
            layer.values.insert(key.to_owned(), v.trim().to_owned());
            if locked {
                layer.locked.push(key.to_owned());
            }
        }
        layer
    }

    /// The file text for these values (sorted by key, so saves are stable).
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::from("# PXP Engineering Suite user defaults (key = value)\n");
        for (k, v) in &self.values {
            out.push_str(k);
            out.push_str(" = ");
            out.push_str(v);
            out.push('\n');
        }
        out
    }
}

/// The three levels together.
#[derive(Debug, Clone, Default)]
pub struct Defaults {
    site: Layer,
    user: Layer,
    user_path: Option<PathBuf>,
}

impl Defaults {
    /// Builds from already-parsed levels (tests, or a shell that read them
    /// itself).
    #[must_use]
    pub fn from_layers(site: Layer, user: Layer, user_path: Option<PathBuf>) -> Self {
        Self {
            site,
            user,
            user_path,
        }
    }

    /// Reads the site file (`PXP_SITE_DEFAULTS`) and the user file. A
    /// missing file is an empty level; an unreadable one is reported in
    /// the returned warnings and treated as empty.
    #[must_use]
    pub fn load() -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut read = |path: Option<PathBuf>| -> Layer {
            let Some(path) = path else {
                return Layer::default();
            };
            match std::fs::read_to_string(&path) {
                Ok(text) => {
                    let layer = Layer::parse(&text);
                    for (n, e) in &layer.errors {
                        warnings.push(format!("{}:{n}: {e}", path.display()));
                    }
                    layer
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Layer::default(),
                Err(e) => {
                    warnings.push(format!("{}: {e}", path.display()));
                    Layer::default()
                }
            }
        };
        let site = read(std::env::var_os("PXP_SITE_DEFAULTS").map(PathBuf::from));
        let user_path = user_file();
        let user = read(user_path.clone());
        (
            Self {
                site,
                user,
                user_path,
            },
            warnings,
        )
    }

    /// The effective value and the level it came from.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<(&str, Level)> {
        let shipped = SHIPPED
            .iter()
            .find(|(k, _, _)| *k == key)
            .map(|(_, v, _)| (*v, Level::Shipped));
        let site = self.site.values.get(key).map(|v| (v.as_str(), Level::Site));
        let user = if self.is_locked(key) {
            None
        } else {
            self.user.values.get(key).map(|v| (v.as_str(), Level::User))
        };
        user.or(site).or(shipped)
    }

    /// The effective value as text, or `fallback`.
    #[must_use]
    pub fn text<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.get(key).map_or(fallback, |(v, _)| v)
    }

    /// A boolean value (`true/false/yes/no/1/0`); `fallback` if unset or
    /// unreadable.
    #[must_use]
    pub fn flag(&self, key: &str, fallback: bool) -> bool {
        match self.text(key, "").to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => true,
            "false" | "no" | "0" | "off" => false,
            _ => fallback,
        }
    }

    /// A whole number; `fallback` if unset or unreadable.
    #[must_use]
    pub fn number(&self, key: &str, fallback: u64) -> u64 {
        self.text(key, "").parse().unwrap_or(fallback)
    }

    /// The site locked this key.
    #[must_use]
    pub fn is_locked(&self, key: &str) -> bool {
        self.site.locked.iter().any(|k| k == key)
    }

    /// Sets a user-level value (refused for a locked key). `None` removes
    /// the user value, falling back to site or shipped.
    ///
    /// # Errors
    /// The key is locked by the site.
    pub fn set_user(&mut self, key: &str, value: Option<&str>) -> Result<(), String> {
        if self.is_locked(key) {
            return Err(format!("'{key}' is locked by the site defaults"));
        }
        match value {
            Some(v) => self.user.values.insert(key.to_owned(), v.to_owned()),
            None => self.user.values.remove(key),
        };
        Ok(())
    }

    /// Writes the user level to its file.
    ///
    /// # Errors
    /// No user file location, or an I/O error.
    pub fn save_user(&self) -> Result<PathBuf, String> {
        let path = self
            .user_path
            .clone()
            .ok_or("no per-user configuration folder")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&path, self.user.to_text())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// Every key with a value at any level, shipped keys first, for the
    /// Customer Defaults window.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = SHIPPED.iter().map(|(k, _, _)| (*k).to_owned()).collect();
        for k in self.site.values.keys().chain(self.user.values.keys()) {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
        keys
    }

    /// Shortcut overrides: command id → key chord, from every level.
    #[must_use]
    pub fn shortcuts(&self) -> Vec<(String, String)> {
        self.keys()
            .into_iter()
            .filter_map(|k| {
                let id = k.strip_prefix(SHORTCUT_PREFIX)?.to_owned();
                let (v, _) = self.get(&k)?;
                Some((id, v.to_owned()))
            })
            .collect()
    }

    /// The user file's location, if there is one.
    #[must_use]
    pub fn user_path(&self) -> Option<&Path> {
        self.user_path.as_deref()
    }
}

/// `<config dir>/pxp/user_defaults.txt`: `%APPDATA%` on Windows,
/// `~/Library/Preferences` on macOS, `$XDG_CONFIG_HOME` or `~/.config`
/// elsewhere. `PXP_USER_DEFAULTS` overrides it.
#[must_use]
pub fn user_file() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(p) = env("PXP_USER_DEFAULTS") {
        return Some(p);
    }
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Preferences"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    base.map(|b| b.join("pxp").join("user_defaults.txt"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn levels_override_in_order_and_locks_hold() {
        let site =
            Layer::parse("theme = dark\nunits != in  # the site works in inches\nbad line\n");
        assert_eq!(site.errors.len(), 1);
        let user = Layer::parse(
            "theme = light\nunits = mm\nshortcut.modeling.block = Ctrl+B\nfuture_key = 3\n",
        );
        let mut d = Defaults::from_layers(site, user, None);
        assert_eq!(
            d.get("theme"),
            Some(("light", Level::User)),
            "user beats site"
        );
        assert_eq!(
            d.get("units"),
            Some(("in", Level::Site)),
            "a locked site value beats the user"
        );
        assert_eq!(d.get("edges"), Some(("true", Level::Shipped)));
        assert!(d.flag("edges", false));
        assert_eq!(d.number("autosave_minutes", 0), 5);
        assert!(d.set_user("units", Some("mm")).is_err());
        d.set_user("theme", None).unwrap();
        assert_eq!(
            d.get("theme"),
            Some(("dark", Level::Site)),
            "removing the user value falls back"
        );
        assert_eq!(
            d.shortcuts(),
            vec![("modeling.block".to_owned(), "Ctrl+B".to_owned())]
        );
        assert!(
            d.keys().contains(&"future_key".to_owned()),
            "unknown keys survive"
        );
    }

    #[test]
    fn the_file_round_trips() {
        let l = Layer::parse("b = 2\na = 1\n");
        assert_eq!(Layer::parse(&l.to_text()), l);
    }
}
