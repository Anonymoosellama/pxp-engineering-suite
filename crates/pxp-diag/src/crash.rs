// SPDX-License-Identifier: LGPL-2.1-or-later
//! The local crash reporter (master prompt, Stage 0: "local crash reporter,
//! no telemetry").
//!
//! ## How it works
//!
//! ```text
//! startup ─► install(dir, app) ─► std::panic::set_hook(our hook, then the previous hook)
//! panic anywhere ─► hook writes dir/crash-<unix ms>-<pid>-<n>.txt  (build, thread, message, location, backtrace)
//!                └► previous hook runs (stderr message as usual)
//! next startup ─► pending(dir) lists the reports ─► the shell offers "view / copy / delete"
//! ```
//!
//! A panic inside a transaction is caught and rolled back by `pxp-doc`, so
//! most reports describe a failure the user survived; a report is still
//! written because a panic is always a bug worth a report. A crash that
//! does not unwind (an abort, a kill) leaves no report; the recovery
//! journal and autosave (`pxp-format`) cover the user's work in that case.
//!
//! ## Must never
//!
//! - Send anything over the network. Reports stay in `dir` until the user
//!   copies or deletes them.
//! - Include document contents: only the panic message, its source
//!   location, the thread name, a backtrace and build facts.
//! - Panic or block inside the hook: every write is best effort and errors
//!   are ignored, because the hook already runs during a failure.

use crate::BuildInfo;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// File-name prefix of every report; [`pending`] lists only these.
pub const PREFIX: &str = "crash-";

/// Reports kept by [`install`]: older ones are deleted at startup so a
/// crash loop cannot fill the disk.
pub const KEEP: usize = 20;

/// Distinguishes reports written in the same millisecond by one process.
static SEQUENCE: AtomicU32 = AtomicU32::new(0);

/// The per-user directory for crash reports:
///
/// | OS | Directory |
/// | --- | --- |
/// | Windows | `%LOCALAPPDATA%\PXP\crash-reports` |
/// | macOS | `~/Library/Logs/PXP/crash-reports` |
/// | Linux and others | `$XDG_STATE_HOME/pxp/crash-reports`, else `~/.local/state/pxp/crash-reports` |
///
/// `PXP_CRASH_DIR` overrides all of these (tests, portable installs). With
/// no usable home directory the system temporary directory is used.
#[must_use]
pub fn default_dir() -> PathBuf {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = env("PXP_CRASH_DIR") {
        return dir;
    }
    let base = if cfg!(windows) {
        env("LOCALAPPDATA").map(|d| d.join("PXP"))
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Logs/PXP"))
    } else {
        env("XDG_STATE_HOME")
            .map(|d| d.join("pxp"))
            .or_else(|| env("HOME").map(|h| h.join(".local/state/pxp")))
    };
    base.unwrap_or_else(std::env::temp_dir)
        .join("crash-reports")
}

/// Installs the crash-report panic hook for this process. `app` names the
/// program in each report (`"pxp-cli"`, `"pxp-app"`). The previous hook
/// (normally the standard one that prints to stderr) still runs after the
/// report is written. Old reports beyond [`KEEP`] are pruned here.
///
/// Call once, early in `main`. Calling again stacks a second hook.
pub fn install(dir: PathBuf, app: &'static str) {
    prune(&dir, KEEP);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let text = render(
            app,
            thread.name().unwrap_or("<unnamed>"),
            &crate::panic_message(info.payload()),
            location.as_deref(),
            &std::backtrace::Backtrace::force_capture().to_string(),
        );
        // Best effort: a failure to write must not turn into a second panic.
        let _ = write_report(&dir, &text);
        previous(info);
    }));
}

/// The text of one report. Kept separate from the hook so it can be tested
/// without panicking.
#[must_use]
pub fn render(
    app: &str,
    thread: &str,
    message: &str,
    location: Option<&str>,
    backtrace: &str,
) -> String {
    let b = BuildInfo::current();
    let mut out = String::new();
    // Writing to a String cannot fail; the results are ignored on purpose.
    let _ = writeln!(out, "PXP Engineering Suite crash report");
    let _ = writeln!(out, "program: {app}");
    let _ = writeln!(out, "build: pxp {} ({}-{})", b.version, b.os, b.arch);
    let _ = writeln!(out, "time (unix ms): {}", now_ms());
    let _ = writeln!(out, "thread: {thread}");
    let _ = writeln!(out, "panic: {message}");
    let _ = writeln!(out, "location: {}", location.unwrap_or("<unknown>"));
    let _ = writeln!(out, "\nbacktrace:\n{backtrace}");
    let _ = writeln!(
        out,
        "\nThis report was written on this computer only. Nothing was sent anywhere.\n\
         It holds no document contents; check it before sharing."
    );
    out
}

/// Writes `text` as a new report in `dir` (created if missing) and returns
/// its path. The write goes to a temporary name first and is renamed, so a
/// listed report is always complete.
///
/// # Errors
/// I/O errors creating the directory or writing the file.
pub fn write_report(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let n = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = format!("{PREFIX}{:013}-{}-{n}.txt", now_ms(), std::process::id());
    let path = dir.join(&name);
    let temp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, &path)?;
    Ok(path)
}

/// Reports in `dir`, newest first. A missing directory means none.
#[must_use]
pub fn pending(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                // Only names this module writes (always lower-case `.txt`).
                .is_some_and(|n| {
                    n.starts_with(PREFIX) && Path::new(n).extension().is_some_and(|e| e == "txt")
                })
        })
        .collect();
    // Names start with a zero-padded millisecond time, so name order is age
    // order; reversed for newest first.
    out.sort();
    out.reverse();
    out
}

/// Deletes every report in `dir`; returns how many were deleted.
///
/// # Errors
/// The first I/O error deleting a report.
pub fn clear(dir: &Path) -> std::io::Result<usize> {
    let reports = pending(dir);
    for p in &reports {
        std::fs::remove_file(p)?;
    }
    Ok(reports.len())
}

/// Deletes all but the newest `keep` reports (best effort).
fn prune(dir: &Path, keep: usize) {
    for old in pending(dir).iter().skip(keep) {
        let _ = std::fs::remove_file(old);
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pxp-diag-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn reports_are_listed_newest_first_and_pruned() {
        let dir = scratch("list");
        assert!(pending(&dir).is_empty(), "a missing directory means none");
        let mut written = Vec::new();
        for i in 0..5 {
            written.push(write_report(&dir, &format!("report {i}")).unwrap());
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        std::fs::write(dir.join("unrelated.txt"), "x").unwrap();
        let listed = pending(&dir);
        assert_eq!(listed.len(), 5, "only reports are listed");
        assert_eq!(listed.first(), written.last(), "newest first");
        prune(&dir, 2);
        assert_eq!(pending(&dir), vec![written[4].clone(), written[3].clone()]);
        assert_eq!(clear(&dir).unwrap(), 2);
        assert!(pending(&dir).is_empty());
        assert!(
            dir.join("unrelated.txt").exists(),
            "other files are left alone"
        );
    }

    #[test]
    fn rendered_report_names_the_failure_and_promises_locality() {
        let text = render("pxp-cli", "main", "boom", Some("a.rs:1:2"), "frames");
        for needle in [
            "program: pxp-cli",
            "thread: main",
            "panic: boom",
            "location: a.rs:1:2",
            "frames",
            "Nothing was sent anywhere",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in\n{text}");
        }
    }
}
