// SPDX-License-Identifier: LGPL-2.1-or-later
//! Session journals: record a session of commands, replay it, and compare
//! the geometry hashes it produces (master prompt, Stage 0: "journaling:
//! record a session, replay it, compare resulting geometry hashes. Journals
//! become regression tests").
//!
//! ## Format (`.pxj`, version 1)
//!
//! JSON lines. The first line is a header; every other line is one command:
//!
//! ```text
//! {"pxp_journal":1,"pxp":"0.0.1","kernel":"solidcraft","platform":"linux-x86_64"}
//! {"args":["part","new","bracket.pxp"],"ok":true,"parts":{"bracket.pxp":{"Datum CSYS":null}}}
//! {"args":["part","block","bracket.pxp","--size","10","20","30"],"ok":true,
//!  "parts":{"bracket.pxp":{"Datum CSYS":null,"Block(1)":"91c0…"}}}
//! {"args":["part","block","bracket.pxp","--size","0","1","1"],"ok":false,"error":"…"}
//! ```
//!
//! `parts` holds, for every `.pxp` file the command names, each feature's
//! stored geometry hash (`null` for features without geometry, such as a
//! datum). `ok` records whether the command succeeded, so a journal also
//! pins refusals.
//!
//! ## Recording and replaying
//!
//! - `pxp-cli journal record FILE.pxj -- COMMAND ARGS…` runs one command
//!   in the current directory and appends its entry (creating the journal
//!   with its header when it does not exist). `pxp-cli mcp --journal FILE`
//!   records every tool call the same way.
//! - `pxp-cli journal replay FILE.pxj [--keep DIR]` runs every command
//!   again in a fresh empty directory and compares outcomes and hashes. Any
//!   difference is reported and the command fails (exit code 1).
//!
//! Paths in journaled commands must be relative, so a replay creates its
//! own files instead of touching the recorder's.
//!
//! ## Must never
//!
//! - Report a replay as matching when a feature is missing, extra, or
//!   hashes differently, or when a command that failed now succeeds (or the
//!   reverse).

use crate::CliError;
use serde_json::{Map, Value, json};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// Journal format version.
pub(crate) const VERSION: u64 = 1;

pub(crate) const USAGE: &str = "\
usage: pxp-cli journal record FILE.pxj -- COMMAND [ARGS...]
       pxp-cli journal replay FILE.pxj [--keep DIR]";

fn usage(m: impl Into<String>) -> CliError {
    CliError::Usage(format!("{}\n\n{USAGE}", m.into()))
}

fn cmd_err(e: impl std::fmt::Display) -> CliError {
    CliError::Command(e.to_string())
}

/// Runs one CLI command (the same function `main` uses).
pub(crate) type Runner = fn(&[String]) -> Result<String, CliError>;

/// Reads the feature hashes of one part: `name → hash` (`None` for a
/// feature without geometry).
pub(crate) type PartHashes = fn(&Path) -> Result<Vec<(String, Option<u64>)>, CliError>;

/// The header line written at the top of a new journal.
fn header(kernel: &str) -> Value {
    json!({
        "pxp_journal": VERSION,
        "pxp": env!("CARGO_PKG_VERSION"),
        "kernel": kernel,
        "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
    })
}

/// The `.pxp` files a command names, in order, without repeats.
fn part_paths(args: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in args {
        if Path::new(a)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pxp"))
            && !out.contains(a)
        {
            out.push(a.clone());
        }
    }
    out
}

/// Feature hashes of every part `args` names that exists under `dir`.
fn snapshot(dir: &Path, args: &[String], hashes: PartHashes) -> Result<Value, CliError> {
    let mut parts = Map::new();
    for p in part_paths(args) {
        let path = dir.join(&p);
        if !path.exists() {
            continue;
        }
        let mut features = Map::new();
        for (name, h) in hashes(&path)? {
            features.insert(name, h.map_or(Value::Null, |h| json!(format!("{h:016x}"))));
        }
        parts.insert(p, Value::Object(features));
    }
    Ok(Value::Object(parts))
}

/// One journal entry for `args`, run with `run` in `dir`.
fn execute(
    dir: &Path,
    args: &[String],
    run: Runner,
    hashes: PartHashes,
) -> Result<(Value, String), CliError> {
    if let Some(abs) = part_paths(args)
        .into_iter()
        .find(|p| Path::new(p).is_absolute())
    {
        return Err(usage(format!(
            "journaled commands must use relative paths, got {abs}"
        )));
    }
    let previous = std::env::current_dir().map_err(cmd_err)?;
    std::env::set_current_dir(dir).map_err(cmd_err)?;
    let result = run(args);
    let parts = snapshot(Path::new("."), args, hashes);
    std::env::set_current_dir(previous).map_err(cmd_err)?;
    let parts = parts?;
    Ok(match result {
        Ok(out) => (json!({"args": args, "ok": true, "parts": parts}), out),
        Err(e) => (
            json!({"args": args, "ok": false, "error": e.to_string()}),
            format!("error: {e}\n"),
        ),
    })
}

/// Appends `entry` to the journal at `path`, writing the header first if
/// the journal is new.
pub(crate) fn append(path: &Path, entry: &Value, kernel: &str) -> Result<(), CliError> {
    let new = !path.exists();
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| cmd_err(format!("cannot write journal {}: {e}", path.display())))?;
    if new {
        writeln!(f, "{}", header(kernel)).map_err(cmd_err)?;
    }
    writeln!(f, "{entry}").map_err(cmd_err)
}

/// Runs `args` in the current directory and records it in `journal`.
pub(crate) fn record_one(
    journal: &Path,
    args: &[String],
    run: Runner,
    hashes: PartHashes,
    kernel: &str,
) -> Result<String, CliError> {
    let dir = std::env::current_dir().map_err(cmd_err)?;
    let (entry, out) = execute(&dir, args, run, hashes)?;
    append(journal, &entry, kernel)?;
    Ok(out)
}

/// Reads a journal: its header and its entries.
fn read(path: &Path) -> Result<(Value, Vec<Value>), CliError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| cmd_err(format!("cannot read journal {}: {e}", path.display())))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let head: Value = lines
        .next()
        .and_then(|l| serde_json::from_str(l).ok())
        .ok_or_else(|| cmd_err(format!("{} has no journal header", path.display())))?;
    if head.get("pxp_journal").and_then(Value::as_u64) != Some(VERSION) {
        return Err(cmd_err(format!(
            "{} is not a version {VERSION} journal",
            path.display()
        )));
    }
    let entries = lines
        .enumerate()
        .map(|(i, l)| {
            serde_json::from_str(l)
                .map_err(|e| cmd_err(format!("{} line {}: {e}", path.display(), i + 2)))
        })
        .collect::<Result<Vec<Value>, _>>()?;
    Ok((head, entries))
}

/// Differences between a recorded and a replayed entry, in words.
fn compare(recorded: &Value, replayed: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let ok = |v: &Value| v.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if ok(recorded) != ok(replayed) {
        out.push(format!(
            "recorded {} but replay {}",
            if ok(recorded) { "succeeded" } else { "failed" },
            if ok(replayed) {
                "succeeded".to_owned()
            } else {
                format!(
                    "failed: {}",
                    replayed.get("error").and_then(Value::as_str).unwrap_or("?")
                )
            }
        ));
        return out;
    }
    let empty = Map::new();
    let parts = |v: &Value| {
        v.get("parts")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default()
    };
    let (a, b) = (parts(recorded), parts(replayed));
    for (file, fa) in &a {
        let fa = fa.as_object().unwrap_or(&empty);
        let Some(fb) = b.get(file).and_then(Value::as_object) else {
            out.push(format!("{file}: missing after replay"));
            continue;
        };
        for (name, ha) in fa {
            match fb.get(name) {
                None => out.push(format!("{file}: feature {name} missing after replay")),
                Some(hb) if hb != ha => out.push(format!(
                    "{file}: {name} hash {} recorded, {} replayed",
                    ha.as_str().unwrap_or("none"),
                    hb.as_str().unwrap_or("none")
                )),
                Some(_) => {}
            }
        }
        for name in fb.keys().filter(|n| !fa.contains_key(*n)) {
            out.push(format!("{file}: extra feature {name} after replay"));
        }
    }
    for file in b.keys().filter(|f| !a.contains_key(*f)) {
        out.push(format!(
            "{file}: created by the replay but not in the recording"
        ));
    }
    out
}

/// Replays `journal` in a fresh directory (or `keep`, which must be empty
/// or absent) and compares every entry.
pub(crate) fn replay(
    journal: &Path,
    keep: Option<&Path>,
    run: Runner,
    hashes: PartHashes,
) -> Result<String, CliError> {
    let (head, entries) = read(journal)?;
    let dir: PathBuf = if let Some(k) = keep {
        std::fs::create_dir_all(k).map_err(cmd_err)?;
        if std::fs::read_dir(k).map_err(cmd_err)?.next().is_some() {
            return Err(usage(format!("--keep {} must be empty", k.display())));
        }
        k.to_path_buf()
    } else {
        let d = std::env::temp_dir().join(format!(
            "pxp-replay-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        std::fs::create_dir_all(&d).map_err(cmd_err)?;
        d
    };
    let mut out = format!(
        "replaying {} ({} commands; recorded with pxp {} on {} with kernel {})\n",
        journal.display(),
        entries.len(),
        head.get("pxp").and_then(Value::as_str).unwrap_or("?"),
        head.get("platform").and_then(Value::as_str).unwrap_or("?"),
        head.get("kernel").and_then(Value::as_str).unwrap_or("?"),
    );
    let mut differences = 0_usize;
    let mut compared = 0_usize;
    for (i, rec) in entries.iter().enumerate() {
        let args: Vec<String> = rec
            .get("args")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if args.is_empty() {
            return Err(cmd_err(format!("entry {} has no command", i + 1)));
        }
        let (now, _) = execute(&dir, &args, run, hashes)?;
        compared += now.get("parts").and_then(Value::as_object).map_or(0, |p| {
            p.values()
                .filter_map(Value::as_object)
                .map(|f| f.values().filter(|h| h.is_string()).count())
                .sum()
        });
        let diffs = compare(rec, &now);
        let _ = writeln!(
            out,
            "  [{:>3}] {:<5} {}",
            i + 1,
            if diffs.is_empty() { "same" } else { "DIFF" },
            args.join(" ")
        );
        for d in &diffs {
            let _ = writeln!(out, "        {d}");
        }
        differences += diffs.len();
    }
    if keep.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    let _ = writeln!(
        out,
        "{} commands, {compared} feature hashes compared, {differences} difference(s)",
        entries.len()
    );
    if differences == 0 {
        Ok(out)
    } else {
        Err(CliError::Command(out))
    }
}

/// `pxp-cli journal …`.
pub(crate) fn run(
    args: &[String],
    runner: Runner,
    hashes: PartHashes,
    kernel: &str,
) -> Result<String, CliError> {
    match args {
        [cmd, file, sep, rest @ ..] if cmd == "record" && sep == "--" && !rest.is_empty() => {
            if rest.first().is_some_and(|c| c == "journal" || c == "mcp") {
                return Err(usage("a journal cannot record journal or mcp commands"));
            }
            record_one(Path::new(file), rest, runner, hashes, kernel)
        }
        [cmd, file] if cmd == "replay" => replay(Path::new(file), None, runner, hashes),
        [cmd, file, flag, dir] if cmd == "replay" && flag == "--keep" => {
            replay(Path::new(file), Some(Path::new(dir)), runner, hashes)
        }
        [h] if matches!(h.as_str(), "help" | "--help" | "-h") => Ok(format!("{USAGE}\n")),
        _ => Err(usage("unknown journal command")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_part_paths_once() {
        let a: Vec<String> = [
            "part", "block", "a.pxp", "--size", "1", "1", "1", "a.pxp", "B.PXP",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        assert_eq!(part_paths(&a), vec!["a.pxp".to_owned(), "B.PXP".to_owned()]);
    }

    #[test]
    fn compare_names_every_kind_of_difference() {
        let rec = json!({"ok": true, "parts": {"a.pxp": {"Block(1)": "01", "Datum": null}}});
        assert!(compare(&rec, &rec).is_empty());
        let changed = json!({"ok": true, "parts": {"a.pxp": {"Block(1)": "02", "Extra": "03"}}});
        let d = compare(&rec, &changed);
        assert_eq!(d.len(), 3, "{d:?}"); // hash, missing datum, extra feature
        let failed = json!({"ok": false, "error": "boom"});
        assert_eq!(compare(&rec, &failed).len(), 1);
    }
}
