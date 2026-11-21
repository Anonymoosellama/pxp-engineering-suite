// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-cli: the headless command-line interface
//!
//! The CLI runs the same command registry and document core as the desktop
//! application, without a window. In Stage 0 it is the main way to exercise
//! what exists.
//!
//! ## Commands
//!
//! | Command | What it does |
//! | --- | --- |
//! | `pxp-cli part new FILE [--units mm\|in]` | New `.pxp` part from the mm or inch template (a datum CSYS at the origin) |
//! | `pxp-cli part block\|cylinder\|cone\|sphere FILE …` | Adds a primitive: parameters become expressions, the Modeling application regenerates it through the kernel, the file is saved (see `part help`) |
//! | `pxp-cli part info FILE` | Features, states, kernels, geometry hashes, and mass properties read from the stored exact B-reps |
//! | `pxp-cli part verify FILE` | Re-imports every stored B-rep and regenerates every feature: hashes and bytes must match (exit 1 otherwise) |
//! | `pxp-cli bakeoff [--corpus …] [--format summary\|markdown\|hashes]` | Runs the kernel bake-off corpus against the native backend |
//! | `pxp-cli stepstress [--repeat N] [--format summary\|markdown\|tsv\|details] FILE\|DIR...` | Imports STEP files (the NIST MBE PMI test models) with the native backend and stress-tests them: file-stated validation properties, round trips, boolean identities |
//! | `pxp-cli apps` | Lists the six applications and when their commands arrive |
//! | `pxp-cli commands [APP]` | Lists commands (all, or one application's) with status, ribbon placement and shortcut |
//! | `pxp-cli find QUERY` | Command Finder: search commands by label, ID or description |
//! | `pxp-cli shortcut KEYS` | Which command a default shortcut runs (`Ctrl+M`) |
//! | `pxp-cli eval FORMULA [--let NAME=FORMULA]... [--units mm\|in] [--decimals N]` | Evaluates an NX-style expression through the document core (transactions, dependency tracking and recompute) |
//! | `pxp-cli status` | Which crates are implemented and which are planned, read from the crates themselves |
//! | `pxp-cli version` | Version and platform |
//!
//! Planned for sub-stage 0I: `pxp-cli mcp` (MCP server over stdio) and
//! session journaling with record, replay and geometry-hash comparison. They
//! are not offered until they work.
//!
//! ## Kernel
//!
//! The CLI is a shell crate, so it is the layer that chooses the geometry
//! backend: SolidCraft's kernel through `pxp-kernel-native`, hosted
//! in-process by a `KernelHost`. Linking it means this binary is conveyed
//! under LGPL-3.0 (ADR-0001).
//!
//! ## Responsibility and limits
//!
//! The CLI is part of the application shell: it is the only layer that turns
//! typed errors into user messages (exit code 1 for command errors, 2 for
//! usage errors). It never computes anything itself; every result comes from
//! the library crates.
#![forbid(unsafe_code)]

mod bakeoff;
mod part;
mod stepstress;

use pxp_appkit::{AppRegistry, CommandSpec, CommandStatus};
use pxp_compute::CancelToken;
use pxp_doc::{Document, ExpressionsOnly};
use pxp_kernel::KernelHost;
use pxp_units::UnitSystem;
use std::fmt::Write as _;
use std::process::ExitCode;
use std::sync::Arc;

/// A CLI failure: a usage error (exit 2) or a command error (exit 1).
#[derive(Debug)]
enum CliError {
    Usage(String),
    Command(String),
}

impl CliError {
    fn exit_code(&self) -> u8 {
        match self {
            Self::Usage(_) => 2,
            Self::Command(_) => 1,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(m) => write!(f, "{m}\n\n{USAGE}"),
            Self::Command(m) => f.write_str(m),
        }
    }
}

const USAGE: &str = "usage: pxp-cli <apps|commands [APP]|find QUERY|shortcut KEYS|eval FORMULA [--let NAME=FORMULA]... [--units mm|in] [--decimals N]|part COMMAND FILE ...|bakeoff [...]|stepstress [...] FILE|DIR...|status|version>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(out) => {
            print!("{out}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

/// The geometry kernel this shell wires in: SolidCraft's, in-process.
fn native_kernel() -> Arc<KernelHost> {
    Arc::new(KernelHost::in_process(Box::new(
        pxp_kernel_native::NativeKernel::new(),
    )))
}

/// Builds the registry with every built-in application, in Application-tab
/// order. Modeling regenerates geometry through `kernel` when one is given.
fn builtin_registry(kernel: Option<Arc<KernelHost>>) -> Result<AppRegistry, CliError> {
    let mut r = AppRegistry::new();
    let modeling = kernel.map_or_else(
        pxp_app_modeling::Modeling::new,
        pxp_app_modeling::Modeling::with_kernel,
    );
    let apps: [Box<dyn pxp_appkit::Application>; 6] = [
        Box::new(modeling),
        Box::new(pxp_app_drafting::Drafting),
        Box::new(pxp_app_manufacturing::Manufacturing),
        Box::new(pxp_app_postgen::PostGeneration),
        Box::new(pxp_app_machine_kit::MachineKitBuilder),
        Box::new(pxp_app_cae::Cae),
    ];
    for app in apps {
        r.register(app)
            .map_err(|e| CliError::Command(format!("registry: {e}")))?;
    }
    Ok(r)
}

fn run(args: &[String]) -> Result<String, CliError> {
    let (cmd, rest) = args
        .split_first()
        .ok_or_else(|| CliError::Usage("no command given".into()))?;
    match cmd.as_str() {
        "apps" => apps(),
        "commands" => commands(rest.first().map(String::as_str)),
        "find" => find(&rest.join(" ")),
        "shortcut" => shortcut(&rest.join(" ")),
        "eval" => eval(rest),
        "part" => {
            let kernel = native_kernel();
            let registry = builtin_registry(Some(Arc::clone(&kernel)))?;
            part::run(rest, &registry, &kernel)
        }
        "bakeoff" => bakeoff::run(rest, &native_kernel()),
        "stepstress" => stepstress::run(rest, &native_kernel()),
        "status" => Ok(status()),
        "version" | "--version" | "-V" => Ok(version()),
        "help" | "--help" | "-h" => Ok(format!("{USAGE}\n")),
        other => Err(CliError::Usage(format!("unknown command '{other}'"))),
    }
}

fn apps() -> Result<String, CliError> {
    let r = builtin_registry(Some(native_kernel()))?;
    let mut out = String::new();
    for app in r.apps() {
        let switch = r
            .commands()
            .iter()
            .find(|c| c.id == format!("app.switch.{}", app.id()))
            .and_then(|c| c.shortcut.clone())
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{:<26} {:<18} commands from Stage {}  {}",
            app.name(),
            app.id().as_str(), // as_str: AppId's Display does not apply padding
            app.planned_stage(),
            switch
        );
    }
    Ok(out)
}

fn format_command(c: &CommandSpec) -> String {
    let status = match c.status {
        CommandStatus::Available => "available".to_owned(),
        CommandStatus::Planned { stage } => format!("planned: Stage {stage}"),
    };
    format!(
        "{:<36} {:<24} {:<34} {:<14} {}",
        c.id,
        c.label,
        format!("{} > {}", c.placement.tab, c.placement.group),
        c.shortcut.as_deref().unwrap_or("-"),
        status
    )
}

fn commands(app: Option<&str>) -> Result<String, CliError> {
    let r = builtin_registry(Some(native_kernel()))?;
    let filter = app.map(|a| {
        if a.contains('.') {
            a.to_owned()
        } else {
            format!("pxp.{a}")
        }
    });
    if let Some(f) = &filter
        && !r.apps().any(|a| a.id().as_str() == f)
    {
        return Err(CliError::Usage(format!(
            "unknown application '{f}' (see `pxp-cli apps`)"
        )));
    }
    let mut out = String::new();
    for c in r.commands() {
        if filter.as_deref().is_none_or(|f| c.app.as_str() == f) {
            let _ = writeln!(out, "{}", format_command(c));
        }
    }
    Ok(out)
}

fn find(query: &str) -> Result<String, CliError> {
    if query.trim().is_empty() {
        return Err(CliError::Usage("find needs a search term".into()));
    }
    let r = builtin_registry(Some(native_kernel()))?;
    let hits = r.find(query.trim());
    if hits.is_empty() {
        return Ok(format!("no command matches '{}'\n", query.trim()));
    }
    let mut out = String::new();
    for c in hits {
        let _ = writeln!(out, "{}", format_command(c));
        let _ = writeln!(out, "    {}", c.tooltip());
    }
    Ok(out)
}

fn shortcut(keys: &str) -> Result<String, CliError> {
    let r = builtin_registry(Some(native_kernel()))?;
    match r.by_shortcut(keys.trim()) {
        Some(c) => Ok(format!("{}\n    {}\n", format_command(c), c.tooltip())),
        None => Err(CliError::Command(format!(
            "no default command is bound to '{}'",
            keys.trim()
        ))),
    }
}

/// `eval`: defines each `--let` expression in its own transaction (as a user
/// would in the Expressions dialog), recomputes, then evaluates `FORMULA`.
fn eval(args: &[String]) -> Result<String, CliError> {
    let mut formula: Option<&str> = None;
    let mut lets: Vec<(&str, &str)> = Vec::new();
    let mut system = UnitSystem::Millimetre;
    let mut decimals = 6_usize;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--let" => {
                let def = it
                    .next()
                    .ok_or_else(|| CliError::Usage("--let needs NAME=FORMULA".into()))?;
                let (name, f) = def
                    .split_once('=')
                    .ok_or_else(|| CliError::Usage(format!("--let '{def}' is not NAME=FORMULA")))?;
                lets.push((name.trim(), f.trim()));
            }
            "--units" => {
                system = match it.next().map(String::as_str) {
                    Some("mm") => UnitSystem::Millimetre,
                    Some("in") => UnitSystem::Inch,
                    other => {
                        return Err(CliError::Usage(format!(
                            "--units must be mm or in, got {other:?}"
                        )));
                    }
                };
            }
            "--decimals" => {
                decimals = it
                    .next()
                    .and_then(|d| d.parse().ok())
                    .filter(|d| *d <= 15)
                    .ok_or_else(|| CliError::Usage("--decimals needs a number 0-15".into()))?;
            }
            f if formula.is_none() => formula = Some(f),
            extra => return Err(CliError::Usage(format!("unexpected argument '{extra}'"))),
        }
    }
    let formula = formula.ok_or_else(|| CliError::Usage("eval needs a formula".into()))?;
    let mut doc = Document::new();
    for (name, f) in &lets {
        doc.transact(&format!("Expression {name}"), |tx| {
            tx.set_expression(name, f)
        })
        .map_err(|e| CliError::Command(e.to_string()))?;
    }
    let report = doc
        .recompute(&ExpressionsOnly, &CancelToken::new())
        .map_err(|e| CliError::Command(e.to_string()))?;
    if let Some((_, why)) = report.failed.first() {
        return Err(CliError::Command(why.clone()));
    }
    let value = doc
        .evaluate(formula)
        .map_err(|e| CliError::Command(e.to_string()))?;
    let text = system
        .format(value, decimals)
        .map_err(|e| CliError::Command(e.to_string()))?;
    Ok(format!("{text}\n"))
}

/// Crate status, read from each placeholder crate's `PLANNED_STAGE`.
fn status() -> String {
    let implemented = [
        (
            "pxp-units",
            "typed quantities, unit parsing, mm/inch display",
        ),
        (
            "pxp-math",
            "vectors, transforms, Csys, tolerances, stable hash",
        ),
        ("pxp-diag", "error reports, panic capture"),
        ("pxp-compute", "cancellation, ordered parallel map (CPU)"),
        (
            "pxp-doc",
            "objects, expressions, transactions, undo, recompute",
        ),
        (
            "pxp-platform",
            "file locks, durable atomic file replacement",
        ),
        (
            "pxp-format",
            ".pxp (SQLite): save, open, recovery journal, backups, migrations",
        ),
        (
            "pxp-kernel",
            "kernel trait, OpResult/TopoHistory contracts, bake-off harness",
        ),
        (
            "pxp-kernel-native",
            "SolidCraft kernel backend: primitives, booleans, history, hash",
        ),
        ("pxp-appkit", "Application trait, commands, registry"),
        (
            "pxp-app-modeling",
            "datum CSYS and primitives regenerated through the kernel",
        ),
        (
            "pxp-app-*",
            "five more applications registered; commands planned",
        ),
        (
            "pxp-cli",
            "this tool (MCP server and journaling planned, 0I)",
        ),
    ];
    let planned: [(&str, u8); 22] = [
        ("pxp-kernel-occt", pxp_kernel_occt::PLANNED_STAGE),
        ("pxp-sketch", pxp_sketch::PLANNED_STAGE),
        ("pxp-tess", pxp_tess::PLANNED_STAGE),
        ("pxp-render", pxp_render::PLANNED_STAGE),
        ("pxp-ui", pxp_ui::PLANNED_STAGE),
        ("pxp-io", pxp_io::PLANNED_STAGE),
        ("pxp-io-step", pxp_io_step::PLANNED_STAGE),
        ("pxp-io-iges", pxp_io_iges::PLANNED_STAGE),
        ("pxp-io-dxf", pxp_io_dxf::PLANNED_STAGE),
        ("pxp-io-pdf", pxp_io_pdf::PLANNED_STAGE),
        ("pxp-io-stl", pxp_io_stl::PLANNED_STAGE),
        ("pxp-io-3mf", pxp_io_3mf::PLANNED_STAGE),
        ("pxp-cam", pxp_cam::PLANNED_STAGE),
        ("pxp-additive", pxp_additive::PLANNED_STAGE),
        ("pxp-machine", pxp_machine::PLANNED_STAGE),
        ("pxp-post", pxp_post::PLANNED_STAGE),
        ("pxp-vnc", pxp_vnc::PLANNED_STAGE),
        ("pxp-fea", pxp_fea::PLANNED_STAGE),
        ("pxp-topopt", pxp_topopt::PLANNED_STAGE),
        ("pxp-scan", pxp_scan::PLANNED_STAGE),
        ("pxp-pointcloud", pxp_pointcloud::PLANNED_STAGE),
        ("pxp-app (GUI)", 0),
    ];
    let mut out = String::from("Implemented (Stage 0, sessions 1-2):\n");
    for (name, what) in implemented {
        let _ = writeln!(out, "  {name:<20} {what}");
    }
    out.push_str("\nNot yet implemented (planned stage):\n");
    for (name, stage) in planned {
        let _ = writeln!(out, "  {name:<20} Stage {stage}");
    }
    out
}

fn version() -> String {
    format!(
        "pxp-cli {} ({}-{}), PXP Engineering Suite, LGPL-2.1-or-later\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_str(args: &[&str]) -> Result<String, CliError> {
        run(&args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn eval_with_units_and_lets() {
        assert_eq!(
            run_str(&["eval", "1in + 25.4mm", "--decimals", "3"]).unwrap(),
            "50.800 mm\n"
        );
        assert_eq!(
            run_str(&[
                "eval",
                "p1",
                "--let",
                "width=4in",
                "--let",
                "p1=width*2",
                "--units",
                "in",
                "--decimals",
                "2"
            ])
            .unwrap(),
            "8.00 in\n"
        );
        assert!(matches!(
            run_str(&["eval", "1mm + 1deg"]),
            Err(CliError::Command(_))
        ));
        assert!(matches!(run_str(&["eval"]), Err(CliError::Usage(_))));
    }

    #[test]
    fn app_switch_shortcuts_match_master_prompt() {
        assert!(
            run_str(&["shortcut", "Ctrl+M"])
                .unwrap()
                .contains("app.switch.pxp.modeling")
        );
        assert!(
            run_str(&["shortcut", "Ctrl+Shift+D"])
                .unwrap()
                .contains("app.switch.pxp.drafting")
        );
        assert!(
            run_str(&["shortcut", "Ctrl+Alt+M"])
                .unwrap()
                .contains("app.switch.pxp.manufacturing")
        );
    }

    #[test]
    fn planned_commands_are_greyed_with_stage() {
        let out = run_str(&["find", "extrude"]).unwrap();
        assert!(out.contains("Planned for Stage 1."));
        let all = run_str(&["commands", "cae"]).unwrap();
        assert!(all.contains("cae.solve") && all.contains("planned: Stage 5"));
    }

    #[test]
    fn six_applications_registered() {
        assert_eq!(run_str(&["apps"]).unwrap().lines().count(), 6);
    }

    #[test]
    fn usage_errors() {
        assert!(matches!(run_str(&[]), Err(CliError::Usage(_))));
        assert!(matches!(run_str(&["frob"]), Err(CliError::Usage(_))));
        assert!(matches!(
            run_str(&["commands", "nosuchapp"]),
            Err(CliError::Usage(_))
        ));
    }
}
