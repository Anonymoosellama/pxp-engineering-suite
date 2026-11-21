// SPDX-License-Identifier: LGPL-2.1-or-later
//! `pxp-cli bakeoff`: runs the kernel bake-off corpus (`pxp_kernel::bakeoff`)
//! against the native backend and prints a summary, a Markdown report or
//! one hash line per case (for cross-OS comparison in CI).

use crate::CliError;
use pxp_kernel::KernelHost;
use pxp_kernel::bakeoff::{self, Case, Report, Status};
use std::fmt::Write as _;

pub(crate) const USAGE: &str = "\
usage: pxp-cli bakeoff [--corpus all|synthetic|solidcraft] [--filter TEXT] [--repeat N]
                       [--format summary|markdown|hashes|details] [--include-slow] [--quiet]";

fn usage(m: impl Into<String>) -> CliError {
    CliError::Usage(format!("{}\n\n{USAGE}", m.into()))
}

/// Parsed command line.
struct Options<'a> {
    corpus: &'a str,
    filter: Option<&'a str>,
    repeats: usize,
    format: &'a str,
    include_slow: bool,
    quiet: bool,
}

impl<'a> Options<'a> {
    fn parse(args: &'a [String]) -> Result<Option<Self>, CliError> {
        let mut o = Self {
            corpus: "all",
            filter: None,
            repeats: 2,
            format: "summary",
            include_slow: false,
            quiet: false,
        };
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = || {
                it.next()
                    .map(String::as_str)
                    .ok_or_else(|| usage(format!("{a} needs a value")))
            };
            match a.as_str() {
                "--corpus" => o.corpus = value()?,
                "--filter" => o.filter = Some(value()?),
                "--repeat" => {
                    o.repeats = value()?
                        .parse()
                        .ok()
                        .filter(|n| (1..=100).contains(n))
                        .ok_or_else(|| usage("--repeat needs a number 1-100"))?;
                }
                "--format" => o.format = value()?,
                "--quiet" => o.quiet = true,
                "--include-slow" => o.include_slow = true,
                "help" | "--help" | "-h" => return Ok(None),
                other => return Err(usage(format!("unknown option {other}"))),
            }
        }
        if !["summary", "markdown", "hashes", "details"].contains(&o.format) {
            return Err(usage(format!("unknown format '{}'", o.format)));
        }
        Ok(Some(o))
    }

    /// The selected cases, and the ids of slow ones left out.
    fn cases(&self) -> Result<(Vec<Case>, Vec<&'static str>), CliError> {
        let mut cases: Vec<Case> = match self.corpus {
            "all" => bakeoff::synthetic_cases()
                .into_iter()
                .chain(bakeoff::solidcraft_cases())
                .collect(),
            "synthetic" => bakeoff::synthetic_cases(),
            "solidcraft" => bakeoff::solidcraft_cases(),
            other => return Err(usage(format!("unknown corpus '{other}'"))),
        };
        if let Some(f) = self.filter {
            cases.retain(|c| c.id.contains(f));
        }
        let slow: Vec<&'static str> = cases
            .iter()
            .filter(|c| c.slow && !self.include_slow)
            .map(|c| c.id)
            .collect();
        cases.retain(|c| !slow.contains(&c.id));
        Ok((cases, slow))
    }
}

fn summary(report: &Report) -> String {
    let (p, f, e, d) = report.tally("");
    let mut out = format!(
        "kernel {}: {} cases, {p} pass, {f} fail, {e} error, {d} deterministic over {} run(s)\n",
        report.backend,
        report.outcomes.len(),
        report.repeats
    );
    for o in report.outcomes.iter().filter(|o| o.status != Status::Pass) {
        let _ = writeln!(out, "  {} {}: {}", o.status.label(), o.id, o.detail);
    }
    out
}

pub(crate) fn run(args: &[String], kernel: &KernelHost) -> Result<String, CliError> {
    let Some(opts) = Options::parse(args)? else {
        return Ok(format!("{USAGE}\n"));
    };
    let (cases, slow) = opts.cases()?;
    let total = cases.len();
    let mut done = 0_usize;
    let report = bakeoff::run(kernel, &cases, opts.repeats, |o| {
        done += 1;
        if !opts.quiet {
            eprintln!(
                "[{done:>3}/{total}] {:<5} {:>8.0} ms  {}{}",
                o.status.label(),
                o.seconds * 1000.0,
                o.id,
                if o.deterministic {
                    ""
                } else {
                    "  (NOT deterministic)"
                }
            );
        }
    });
    let skipped = if slow.is_empty() {
        String::new()
    } else {
        format!(
            "skipped {} slow case(s) (--include-slow runs them): {}\n",
            slow.len(),
            slow.join(", ")
        )
    };
    Ok(match opts.format {
        "markdown" if skipped.is_empty() => report.to_markdown(),
        "markdown" => format!("{}\n{skipped}", report.to_markdown()),
        "hashes" => report.hashes(),
        "details" => report.details(),
        _ => format!("{}{skipped}", summary(&report)),
    })
}
