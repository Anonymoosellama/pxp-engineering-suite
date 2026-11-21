// SPDX-License-Identifier: LGPL-2.1-or-later
//! `pxp-cli stepstress`: runs the STEP-file stress procedure
//! (`pxp_kernel::bakeoff::run_step_file`) on STEP files or directories of
//! them, against the native backend.
//!
//! Each file is imported by the backend's own translator, graded against
//! the geometric validation properties the file states about itself, round
//! tripped (native bytes and STEP), and cut by a box, a half-space and a
//! drilled cylinder whose results must satisfy inclusion-exclusion. Built
//! for the NIST MBE PMI test models (`tools/fetch_nist_step.py`).
//!
//! The whole run happens in this process. For a long corpus where one file
//! might hang or abort the process, drive it one file per process (the
//! `tsv` format without `--header` prints one line per file, so the lines
//! of separate runs concatenate).

use crate::CliError;
use pxp_kernel::KernelHost;
use pxp_kernel::bakeoff::{self, Status, StepOutcome};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub(crate) const USAGE: &str = "\
usage: pxp-cli stepstress [--repeat N] [--format summary|markdown|tsv|details] [--header]
                          [--details FILE] [--quiet] FILE|DIR...
  Imports each STEP file (a DIR contributes its *.stp and *.step files, sorted) with the
  native backend and stress-tests it; see `pxp_kernel::bakeoff` for the checks.
  --details FILE also appends every file's per-check details (Markdown) to FILE.";

fn usage(m: impl Into<String>) -> CliError {
    CliError::Usage(format!("{}\n\n{USAGE}", m.into()))
}

/// Parsed command line.
struct Options<'a> {
    repeats: usize,
    format: &'a str,
    header: bool,
    quiet: bool,
    details: Option<&'a str>,
    paths: Vec<&'a str>,
}

impl<'a> Options<'a> {
    fn parse(args: &'a [String]) -> Result<Option<Self>, CliError> {
        let mut o = Self {
            repeats: 2,
            format: "summary",
            header: false,
            quiet: false,
            details: None,
            paths: Vec::new(),
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = || {
                it.next()
                    .map(String::as_str)
                    .ok_or_else(|| usage(format!("{a} needs a value")))
            };
            match a.as_str() {
                "--repeat" => {
                    o.repeats = value()?
                        .parse()
                        .ok()
                        .filter(|n| (1..=20).contains(n))
                        .ok_or_else(|| usage("--repeat needs a number 1-20"))?;
                }
                "--format" => o.format = value()?,
                "--header" => o.header = true,
                "--details" => o.details = Some(value()?),
                "--quiet" => o.quiet = true,
                "help" | "--help" | "-h" => return Ok(None),
                other if other.starts_with("--") => {
                    return Err(usage(format!("unknown option {other}")));
                }
                path => o.paths.push(path),
            }
        }
        if !["summary", "markdown", "tsv", "details"].contains(&o.format) {
            return Err(usage(format!("unknown format '{}'", o.format)));
        }
        if o.paths.is_empty() {
            return Err(usage("no STEP file or directory given"));
        }
        Ok(Some(o))
    }
}

fn is_step(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("stp") || e.eq_ignore_ascii_case("step"))
}

/// The files named, directories expanded one level (sorted, so runs on
/// different machines visit files in the same order).
fn files(paths: &[&str]) -> Result<Vec<PathBuf>, CliError> {
    let mut out = Vec::new();
    for p in paths {
        let path = Path::new(p);
        if path.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(path)
                .map_err(|e| CliError::Command(format!("cannot list {p}: {e}")))?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|f| f.is_file() && is_step(f))
                .collect();
            found.sort();
            out.extend(found);
        } else {
            out.push(path.to_path_buf());
        }
    }
    Ok(out)
}

fn summary(backend: &str, repeats: usize, outcomes: &[StepOutcome]) -> String {
    let count = |s| outcomes.iter().filter(|o| o.status == s).count();
    let det = outcomes.iter().filter(|o| o.deterministic).count();
    let mut out = format!(
        "kernel {backend}: {} STEP files, {} pass, {} fail, {} error, {det} deterministic over {repeats} run(s)\n",
        outcomes.len(),
        count(Status::Pass),
        count(Status::Fail),
        count(Status::Error),
    );
    for o in outcomes.iter().filter(|o| o.status != Status::Pass) {
        for c in o
            .checks
            .iter()
            .filter(|c| matches!(c.status, Some(Status::Fail | Status::Error)))
        {
            let label = c.status.map_or("n/a", Status::label);
            let _ = writeln!(out, "  {label} {} {}: {}", o.id, c.name, c.detail);
        }
    }
    out
}

pub(crate) fn run(args: &[String], kernel: &KernelHost) -> Result<String, CliError> {
    let Some(opts) = Options::parse(args)? else {
        return Ok(format!("{USAGE}\n"));
    };
    let files = files(&opts.paths)?;
    let total = files.len();
    let mut outcomes = Vec::with_capacity(total);
    for (i, f) in files.iter().enumerate() {
        let bytes = std::fs::read(f)
            .map_err(|e| CliError::Command(format!("cannot read {}: {e}", f.display())))?;
        // STEP is ISO 8859-1 at worst in its strings; geometry is ASCII.
        let text = String::from_utf8_lossy(&bytes);
        let id = f.file_stem().map_or_else(
            || f.display().to_string(),
            |s| s.to_string_lossy().into_owned(),
        );
        let o = bakeoff::run_step_file(kernel, &id, &text, opts.repeats);
        if !opts.quiet {
            eprintln!(
                "[{:>3}/{total}] {:<5} {:>8.1} s (import {:>6.1} s)  {}{}",
                i + 1,
                o.status.label(),
                o.seconds,
                o.import_seconds,
                o.id,
                if o.deterministic {
                    ""
                } else {
                    "  (NOT deterministic)"
                }
            );
        }
        if let Some(path) = opts.details {
            use std::io::Write as _;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .and_then(|mut f| writeln!(f, "{}", o.details_markdown()))
                .map_err(|e| CliError::Command(format!("cannot write {path}: {e}")))?;
        }
        outcomes.push(o);
    }
    let backend = kernel.backend_id().0;
    Ok(match opts.format {
        "markdown" => bakeoff::step_markdown(backend, opts.repeats, &outcomes),
        "tsv" => {
            let mut out = String::new();
            if opts.header {
                out.push_str(&StepOutcome::tsv_header());
                out.push('\n');
            }
            for o in &outcomes {
                out.push_str(&o.tsv());
                out.push('\n');
            }
            out
        }
        "details" => outcomes
            .iter()
            .map(StepOutcome::details_markdown)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => summary(backend, opts.repeats, &outcomes),
    })
}
