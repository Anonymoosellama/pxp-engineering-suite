// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-diag: error reports and panic capture
//!
//! ## Responsibility
//!
//! Turns errors and caught panics into an [`ErrorReport`]: the text behind the
//! "copy report" button the master prompt requires whenever a command fails.
//! Later in Stage 0 (sub-stage 0J) this crate also owns structured tracing and
//! the **local** crash reporter. There is no telemetry: reports are written
//! to disk or the clipboard, never sent anywhere.
//!
//! ## Data flow
//!
//! `pxp-doc` catches a panic at the transaction boundary, rolls back, and asks
//! [`panic_message`] for a readable message. The shell builds an
//! [`ErrorReport`] from the resulting error and shows it to the user.
//!
//! ## Invariants
//!
//! - Building a report never panics and never allocates unboundedly: cause
//!   chains are capped at [`ErrorReport::MAX_CAUSES`].
//! - Reports contain build and platform information so a pasted report is
//!   enough to start debugging.
//!
//! ## Must never
//!
//! - Send data over the network (no telemetry, per the master prompt).
//! - Include document contents in a report unless the user adds them.
#![forbid(unsafe_code)]

use std::any::Any;
use std::error::Error;
use std::fmt::Write as _;

/// Build and platform facts included in every report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInfo {
    /// Workspace version (`CARGO_PKG_VERSION` of this crate).
    pub version: &'static str,
    /// Operating system (`std::env::consts::OS`).
    pub os: &'static str,
    /// CPU architecture (`std::env::consts::ARCH`).
    pub arch: &'static str,
}

impl BuildInfo {
    /// Facts about the running build.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
        }
    }
}

/// A user-facing error report: a title, the error message, its chain of
/// causes, optional key/value context (command name, transaction label), and
/// build information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorReport {
    /// Short title, e.g. `"Block failed"`.
    pub title: String,
    /// The top-level error message.
    pub message: String,
    /// `source()` chain, outermost first.
    pub causes: Vec<String>,
    /// Extra context such as `("command", "modeling.block")`.
    pub context: Vec<(String, String)>,
    /// Build and platform facts.
    pub build: BuildInfo,
}

impl ErrorReport {
    /// Cause chains longer than this are truncated (protects against cyclic
    /// or pathological `source()` implementations).
    pub const MAX_CAUSES: usize = 32;

    /// Builds a report from any error, walking its `source()` chain.
    #[must_use]
    pub fn from_error(title: impl Into<String>, err: &(dyn Error + 'static)) -> Self {
        let mut causes = Vec::new();
        let mut next = err.source();
        while let Some(cause) = next {
            if causes.len() == Self::MAX_CAUSES {
                causes.push("… (cause chain truncated)".to_owned());
                break;
            }
            causes.push(cause.to_string());
            next = cause.source();
        }
        Self {
            title: title.into(),
            message: err.to_string(),
            causes,
            context: Vec::new(),
            build: BuildInfo::current(),
        }
    }

    /// Adds a context line (builder style).
    #[must_use]
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.push((key.into(), value.into()));
        self
    }

    /// Plain-text rendering for the clipboard ("copy report").
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        // Writing to a String cannot fail; the results are ignored on purpose.
        let _ = writeln!(out, "{}", self.title);
        let _ = writeln!(out, "error: {}", self.message);
        for (i, c) in self.causes.iter().enumerate() {
            let _ = writeln!(out, "  cause {}: {c}", i + 1);
        }
        for (k, v) in &self.context {
            let _ = writeln!(out, "{k}: {v}");
        }
        let _ = writeln!(
            out,
            "build: pxp {} ({}-{})",
            self.build.version, self.build.os, self.build.arch
        );
        out
    }
}

/// Extracts a readable message from a panic payload returned by
/// `std::panic::catch_unwind`. Panics carry either a `&'static str` or a
/// `String`; anything else is reported generically.
#[must_use]
pub fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic with a non-string payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct Outer(Inner);
    #[derive(Debug)]
    struct Inner;
    impl fmt::Display for Outer {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("outer failed")
        }
    }
    impl fmt::Display for Inner {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("inner cause")
        }
    }
    impl Error for Inner {}
    impl Error for Outer {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn report_walks_cause_chain() {
        let r = ErrorReport::from_error("Block failed", &Outer(Inner))
            .with_context("command", "modeling.block");
        assert_eq!(r.causes, vec!["inner cause".to_owned()]);
        let text = r.to_text();
        assert!(text.contains("error: outer failed"));
        assert!(text.contains("cause 1: inner cause"));
        assert!(text.contains("command: modeling.block"));
        assert!(text.contains("build: pxp"));
    }

    #[test]
    fn panic_payloads_are_readable() {
        let p = std::panic::catch_unwind(|| panic!("static message")).unwrap_err();
        assert_eq!(panic_message(p.as_ref()), "static message");
        let p = std::panic::catch_unwind(|| panic!("formatted {}", 42)).unwrap_err();
        assert_eq!(panic_message(p.as_ref()), "formatted 42");
        let p = std::panic::catch_unwind(|| std::panic::panic_any(7_u8)).unwrap_err();
        assert_eq!(panic_message(p.as_ref()), "panic with a non-string payload");
    }
}
