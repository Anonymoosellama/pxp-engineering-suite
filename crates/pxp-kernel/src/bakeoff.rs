// SPDX-License-Identifier: LGPL-2.1-or-later
//! The kernel bake-off harness (master prompt, Stage 0; ADR-0004).
//!
//! Runs a corpus of [`Case`]s through any backend behind a [`KernelHost`]
//! and grades each result against an answer that **never comes from a
//! kernel**: a hand calculation, an analytic formula, or an invariant.
//!
//! ## Corpus
//!
//! - [`synthetic_cases`]: PXP's own stress cases (coincident faces, tangent
//!   contacts, tiny and near-tolerance features, voids, empty results,
//!   refusals), each with its expected answer worked out in its
//!   description.
//! - [`generated_cases`]: families that sweep one parameter (count, offset,
//!   angle, scale) through constructions with closed-form answers.
//! - [`solidcraft_cases`]: SolidCraft's kernel regression cases that can be
//!   expressed with primitives and booleans, converted from SolidCraft's
//!   `tests/kernel-cases` by `tools/import_solidcraft_cases.py` (MIT; the
//!   generated file carries the notice).
//!
//! - [`run_step_file`]: real parts from STEP files (written for the NIST
//!   MBE PMI test models), read by the backend's own translator
//!   ([`crate::GeometryKernel::import_step`]) and graded against the
//!   file's own geometric validation properties ([`validation_properties`])
//!   and against invariants: round trips and inclusion-exclusion identities
//!   of booleans on the imported part. The files are not compiled in;
//!   `pxp-cli stepstress` reads them from disk (`tools/fetch_nist_step.py`
//!   downloads the NIST set with checksums).
//!
//! The primitive corpus measures boolean robustness on exact answers; the
//! STEP corpus measures translator fidelity and boolean robustness on real
//! geometry. Blend success rates need the blend operations of Stage 1.
//!
//! ## Grading
//!
//! Every operation goes through [`KernelHost::op`], so an incomplete history
//! fails the case. A case **passes** when every expected measure is within
//! its range; **fails** when the kernel succeeded with a wrong answer; is an
//! **error** when the kernel refused or failed where the case expects
//! success (or succeeded where it expects a refusal). Each case runs
//! `repeats` times: identical geometry hashes across repeats make it
//! **deterministic** (cross-OS determinism compares the printed hashes of CI
//! runs on each platform).
//!
//! Measures are totals over every body left at the end, as in SolidCraft's
//! rig. Body counts are not graded: PXP has no rule yet for splitting a body
//! whose pieces touch only along an edge, so a count could differ for
//! reasons that are not the kernel's.

use crate::{
    BlockSpec, BooleanOp, ConeSpec, CylinderSpec, KernelError, KernelHost, ShapeHandle, SphereSpec,
    TopoKind,
};
use pxp_math::{Csys, Point3, UnitVec3, Vec3};
use pxp_units::Length;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Instant;

mod families;
mod p21;
mod solidcraft;
mod step;
mod synthetic;

pub use families::cases as generated_cases;
pub use p21::{Declared, ValidationProperties, validation_properties};
pub use solidcraft::cases as solidcraft_cases;
pub use step::{
    CHECKS as STEP_CHECKS, Check as StepCheck, IDENTITY_TOLERANCE, REFERENCE_TOLERANCE,
    StepOutcome, run_step_file, to_markdown as step_markdown,
};
pub use synthetic::cases as synthetic_cases;

/// Resolution used for the geometry hashes the harness prints.
pub const HASH_RESOLUTION: Length = crate::DEFAULT_HASH_RESOLUTION;

/// A primitive to create.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Prim {
    /// Block.
    Block(BlockSpec),
    /// Cylinder.
    Cylinder(CylinderSpec),
    /// Cone.
    Cone(ConeSpec),
    /// Sphere.
    Sphere(SphereSpec),
}

/// One step of a case.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// Creates body `name` (replacing nothing: names are unique).
    Make {
        /// Body name.
        name: &'static str,
        /// What to create.
        prim: Prim,
    },
    /// `target ← target op tool` for each tool in turn. Tools are removed
    /// unless `keep_tools`.
    Combine {
        /// Body changed in place.
        target: &'static str,
        /// Tool bodies, applied in order.
        tools: Vec<&'static str>,
        /// Unite, subtract or intersect.
        op: BooleanOp,
        /// Keep the tool bodies afterwards.
        keep_tools: bool,
    },
}

/// An inclusive range a measure must fall in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Range {
    /// Lowest accepted value.
    pub lo: f64,
    /// Highest accepted value.
    pub hi: f64,
}

impl Range {
    /// `value` within a relative tolerance `rel`.
    #[must_use]
    pub fn rel(value: f64, rel: f64) -> Self {
        let d = value.abs() * rel;
        Self {
            lo: value - d,
            hi: value + d,
        }
    }

    /// Explicit bounds.
    #[must_use]
    pub const fn between(lo: f64, hi: f64) -> Self {
        Self { lo, hi }
    }

    fn contains(self, v: f64) -> bool {
        v >= self.lo && v <= self.hi
    }
}

/// What a case expects.
#[derive(Debug, Clone, PartialEq)]
pub enum Expect {
    /// Success, with these measures (each `None` is not graded).
    Solid {
        /// Total volume, mm³.
        volume: Option<Range>,
        /// Total area, mm².
        area: Option<Range>,
        /// Total face count, adjacent pieces of one surface counted once
        /// ([`crate::MergedCounts`]): the count SolidCraft's rig grades and
        /// the only one comparable across kernels.
        faces: Option<usize>,
        /// Total shell count (outer shells plus voids).
        shells: Option<usize>,
    },
    /// The last operation leaves no material ([`KernelError::EmptyResult`]).
    Empty,
    /// A step is refused as invalid input ([`KernelError::InvalidInput`]).
    Refused,
}

/// One bake-off case.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    /// Stable identifier (`syn/…`, `solidcraft/<area>/<id>`).
    pub id: &'static str,
    /// Where it comes from (an issue, a stress category).
    pub source: &'static str,
    /// What it exercises and how the answer was derived.
    pub what: &'static str,
    /// Steps, in order.
    pub steps: Vec<Step>,
    /// Expected outcome.
    pub expect: Expect,
    /// Slow by nature (SolidCraft gives it a long timeout): skipped unless
    /// asked for, so a routine run stays quick.
    pub slow: bool,
}

/// How a case ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    /// Right answer.
    Pass,
    /// The kernel succeeded with a wrong answer.
    Fail,
    /// The kernel refused, failed or crashed where it should have succeeded,
    /// or succeeded where it should have refused.
    Error,
}

impl Status {
    /// Short label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "FAIL",
            Self::Error => "ERROR",
        }
    }
}

/// Result of one case.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Case identifier.
    pub id: &'static str,
    /// Graded result of the first run.
    pub status: Status,
    /// Why, for anything but a pass; measured values for a pass.
    pub detail: String,
    /// Wall time of the first run, seconds.
    pub seconds: f64,
    /// Geometry hash of the final bodies (first run), if it succeeded.
    pub hash: Option<u64>,
    /// Every repeat gave the same status and hash.
    pub deterministic: bool,
    /// Diagnostic fingerprint of the first run (raw entity counts, merged
    /// faces, shells, volume, area and summed centroids to 12 significant
    /// digits), for
    /// finding where two platforms' hashes part ways.
    pub fingerprint: Option<String>,
}

/// What a run of the whole corpus produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Backend identity.
    pub backend: &'static str,
    /// Runs per case.
    pub repeats: usize,
    /// One outcome per case, in corpus order.
    pub outcomes: Vec<Outcome>,
}

/// Bodies alive during a case, released when the case ends.
struct Bodies<'h> {
    host: &'h KernelHost,
    named: BTreeMap<&'static str, ShapeHandle>,
}

impl Drop for Bodies<'_> {
    fn drop(&mut self) {
        for s in self.named.values() {
            let _ = self.host.call(|k| {
                k.release(*s);
                Ok(())
            });
        }
    }
}

struct Final {
    volume: f64,
    area: f64,
    /// Sum of the bodies' centroids (diagnostic only).
    centroid: [f64; 3],
    vertices: usize,
    edges: usize,
    /// Faces as the kernel stores them.
    faces: usize,
    /// Merged faces, if the backend can count them.
    merged_faces: Option<usize>,
    shells: usize,
    hash: u64,
}

fn missing(name: &str) -> KernelError {
    KernelError::InvalidInput(format!("case refers to unknown body '{name}'"))
}

fn run_steps(host: &KernelHost, steps: &[Step]) -> Result<Final, KernelError> {
    let mut bodies = Bodies {
        host,
        named: BTreeMap::new(),
    };
    for step in steps {
        match step {
            Step::Make { name, prim } => {
                let r = host.op(|k| match prim {
                    Prim::Block(s) => k.block(s),
                    Prim::Cylinder(s) => k.cylinder(s),
                    Prim::Cone(s) => k.cone(s),
                    Prim::Sphere(s) => k.sphere(s),
                })?;
                if let Some(old) = bodies.named.insert(name, r.shape) {
                    host.call(|k| {
                        k.release(old);
                        Ok(())
                    })?;
                }
            }
            Step::Combine {
                target,
                tools,
                op,
                keep_tools,
            } => {
                for tool in tools {
                    let t = *bodies.named.get(target).ok_or_else(|| missing(target))?;
                    let u = *bodies.named.get(tool).ok_or_else(|| missing(tool))?;
                    let r = host.op(|k| k.boolean(*op, t, u))?;
                    bodies.named.insert(target, r.shape);
                    host.call(|k| {
                        k.release(t);
                        Ok(())
                    })?;
                }
                if !keep_tools {
                    for tool in tools {
                        if let Some(u) = bodies.named.remove(tool) {
                            host.call(|k| {
                                k.release(u);
                                Ok(())
                            })?;
                        }
                    }
                }
            }
        }
    }
    let mut fin = Final {
        volume: 0.0,
        area: 0.0,
        centroid: [0.0; 3],
        vertices: 0,
        edges: 0,
        faces: 0,
        merged_faces: Some(0),
        shells: 0,
        hash: 0,
    };
    let mut h = pxp_math::StableHasher::new();
    for (name, s) in &bodies.named {
        let m = host.call(|k| k.mass_properties(*s))?;
        fin.volume += m.volume.mm3();
        fin.area += m.area.mm2();
        for (sum, c) in fin
            .centroid
            .iter_mut()
            .zip([m.centroid.x, m.centroid.y, m.centroid.z])
        {
            *sum += c;
        }
        let topo = host.call(|k| k.topology(*s))?;
        let count = |kind| topo.iter().filter(|t| t.kind == kind).count();
        fin.vertices += count(TopoKind::Vertex);
        fin.edges += count(TopoKind::Edge);
        fin.faces += count(TopoKind::Face);
        fin.shells += count(TopoKind::Shell);
        fin.merged_faces = match (fin.merged_faces, host.call(|k| k.merged_counts(*s))) {
            (Some(sum), Ok(m)) => Some(sum + m.faces),
            (_, Err(KernelError::Unsupported { .. })) | (None, _) => None,
            (_, Err(e)) => return Err(e),
        };
        h.write_str(name);
        h.write_u64(host.call(|k| k.geometry_hash(*s, HASH_RESOLUTION))?);
    }
    fin.hash = h.finish();
    Ok(fin)
}

fn grade(case: &Case, result: &Result<Final, KernelError>) -> (Status, String) {
    match (&case.expect, result) {
        (Expect::Empty, Err(KernelError::EmptyResult { .. }))
        | (Expect::Refused, Err(KernelError::InvalidInput(_))) => {
            (Status::Pass, "refused as expected".into())
        }
        (Expect::Empty | Expect::Refused, Ok(f)) => (
            Status::Error,
            format!(
                "expected {:?}, got a body of volume {:.6}",
                case.expect, f.volume
            ),
        ),
        (_, Err(e)) => (Status::Error, e.to_string()),
        (
            Expect::Solid {
                volume,
                area,
                faces,
                shells,
            },
            Ok(f),
        ) => {
            let mut wrong = Vec::new();
            if let Some(r) = volume
                && !r.contains(f.volume)
            {
                wrong.push(format!(
                    "volume {:.6} not in [{:.6}, {:.6}]",
                    f.volume, r.lo, r.hi
                ));
            }
            if let Some(r) = area
                && !r.contains(f.area)
            {
                wrong.push(format!(
                    "area {:.6} not in [{:.6}, {:.6}]",
                    f.area, r.lo, r.hi
                ));
            }
            // Merged counts when the backend has them; otherwise the raw
            // count, which may differ for kernels that split periodic faces.
            let counted = f.merged_faces.unwrap_or(f.faces);
            if let Some(n) = faces
                && *n != counted
            {
                wrong.push(format!(
                    "{counted} faces ({} as stored), expected {n}",
                    f.faces
                ));
            }
            if let Some(n) = shells
                && *n != f.shells
            {
                wrong.push(format!("{} shells, expected {n}", f.shells));
            }
            if wrong.is_empty() {
                (
                    Status::Pass,
                    format!(
                        "volume {:.6}, area {:.6}, {} faces ({} as stored), {} shells",
                        f.volume,
                        f.area,
                        f.merged_faces
                            .map_or_else(|| "?".to_owned(), |n| n.to_string()),
                        f.faces,
                        f.shells
                    ),
                )
            } else {
                (Status::Fail, wrong.join("; "))
            }
        }
    }
}

/// Runs one case `repeats` times (at least once).
#[must_use]
pub fn run_case(host: &KernelHost, case: &Case, repeats: usize) -> Outcome {
    let start = Instant::now();
    let first = run_steps(host, &case.steps);
    let seconds = start.elapsed().as_secs_f64();
    let (status, detail) = grade(case, &first);
    let hash = first.as_ref().ok().map(|f| f.hash);
    let fingerprint = first.as_ref().ok().map(|f| {
        format!(
            "v{} e{} f{}/{} s{} V={:.11e} A={:.11e} C=({:.11e},{:.11e},{:.11e})",
            f.vertices,
            f.edges,
            f.faces,
            f.merged_faces
                .map_or_else(|| "?".to_owned(), |n| n.to_string()),
            f.shells,
            f.volume,
            f.area,
            f.centroid[0],
            f.centroid[1],
            f.centroid[2]
        )
    });
    let deterministic = (1..repeats).all(|_| {
        let again = run_steps(host, &case.steps);
        grade(case, &again).0 == status && again.as_ref().ok().map(|f| f.hash) == hash
    });
    Outcome {
        id: case.id,
        status,
        detail,
        seconds,
        hash,
        deterministic,
        fingerprint,
    }
}

/// Runs every case. `progress` is called after each case (for a CLI).
pub fn run(
    host: &KernelHost,
    cases: &[Case],
    repeats: usize,
    mut progress: impl FnMut(&Outcome),
) -> Report {
    let outcomes = cases
        .iter()
        .map(|c| {
            let o = run_case(host, c, repeats);
            progress(&o);
            o
        })
        .collect();
    Report {
        backend: host.backend_id().0,
        repeats,
        outcomes,
    }
}

impl Report {
    /// Counts of each status among cases whose id starts with `prefix`.
    #[must_use]
    pub fn tally(&self, prefix: &str) -> (usize, usize, usize, usize) {
        let mut t = (0, 0, 0, 0);
        for o in self.outcomes.iter().filter(|o| o.id.starts_with(prefix)) {
            match o.status {
                Status::Pass => t.0 += 1,
                Status::Fail => t.1 += 1,
                Status::Error => t.2 += 1,
            }
            if o.deterministic {
                t.3 += 1;
            }
        }
        t
    }

    /// `id hash` lines, for comparing runs on different platforms.
    #[must_use]
    pub fn hashes(&self) -> String {
        let mut out = String::new();
        for o in &self.outcomes {
            let h = o
                .hash
                .map_or_else(|| "-".to_owned(), |h| format!("{h:016x}"));
            let _ = writeln!(out, "{} {} {}", o.id, o.status.label(), h);
        }
        out
    }

    /// [`Report::hashes`] lines followed by each case's fingerprint, for
    /// diagnosing a cross-platform difference. The first three fields of
    /// each line are exactly the `hashes` line.
    #[must_use]
    pub fn details(&self) -> String {
        let mut out = String::new();
        for o in &self.outcomes {
            let h = o
                .hash
                .map_or_else(|| "-".to_owned(), |h| format!("{h:016x}"));
            let _ = writeln!(
                out,
                "{} {} {} {}",
                o.id,
                o.status.label(),
                h,
                o.fingerprint.as_deref().unwrap_or("-")
            );
        }
        out
    }

    /// A Markdown report: summary per corpus, then one row per case.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "| Corpus | Cases | Pass | Fail | Error | Deterministic | Time (s) |"
        );
        let _ = writeln!(out, "| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
        for (label, prefix) in [
            ("Synthetic stress", "syn/"),
            ("SolidCraft reference", "solidcraft/"),
            ("Generated families", "gen/"),
            ("All", ""),
        ] {
            let (p, f, e, d) = self.tally(prefix);
            let secs: f64 = self
                .outcomes
                .iter()
                .filter(|o| o.id.starts_with(prefix))
                .map(|o| o.seconds)
                .sum();
            let _ = writeln!(
                out,
                "| {label} | {} | {p} | {f} | {e} | {d} | {secs:.2} |",
                p + f + e
            );
        }
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "| Case | Result | Time (ms) | Same hash ×{} | Detail |",
            self.repeats
        );
        let _ = writeln!(out, "| --- | --- | ---: | --- | --- |");
        for o in &self.outcomes {
            let _ = writeln!(
                out,
                "| `{}` | {} | {:.0} | {} | {} |",
                o.id,
                o.status.label(),
                o.seconds * 1000.0,
                if o.deterministic { "yes" } else { "**no**" },
                o.detail.replace('|', "\\|")
            );
        }
        out
    }
}

// Helpers the corpus files use to stay readable.

fn mm(v: f64) -> Length {
    Length::from_mm(v)
}

fn axis(d: [f64; 3]) -> UnitVec3 {
    let [x, y, z] = d;
    Vec3::new(x, y, z).normalized().unwrap_or(UnitVec3::Z)
}

/// A block with one corner at `corner` and edges along X, Y, Z.
fn block(corner: [f64; 3], size: [f64; 3]) -> Prim {
    let [x, y, z] = size;
    let [cx, cy, cz] = corner;
    let csys = Csys::new(Point3::new(cx, cy, cz), Vec3::X, Vec3::Y).unwrap_or(Csys::ABSOLUTE);
    Prim::Block(BlockSpec {
        csys,
        x: mm(x),
        y: mm(y),
        z: mm(z),
    })
}

fn point(p: [f64; 3]) -> Point3 {
    let [x, y, z] = p;
    Point3::new(x, y, z)
}

fn cylinder(base: [f64; 3], dir: [f64; 3], diameter: f64, height: f64) -> Prim {
    Prim::Cylinder(CylinderSpec {
        base: point(base),
        axis: axis(dir),
        diameter: mm(diameter),
        height: mm(height),
    })
}

fn cone(base: [f64; 3], dir: [f64; 3], base_d: f64, top_d: f64, height: f64) -> Prim {
    Prim::Cone(ConeSpec {
        base: point(base),
        axis: axis(dir),
        base_diameter: mm(base_d),
        top_diameter: mm(top_d),
        height: mm(height),
    })
}

fn sphere(center: [f64; 3], diameter: f64) -> Prim {
    Prim::Sphere(SphereSpec {
        center: point(center),
        diameter: mm(diameter),
    })
}

fn make(name: &'static str, prim: Prim) -> Step {
    Step::Make { name, prim }
}

fn combine(target: &'static str, tools: &[&'static str], op: BooleanOp, keep_tools: bool) -> Step {
    Step::Combine {
        target,
        tools: tools.to_vec(),
        op,
        keep_tools,
    }
}

fn solid(
    volume: Option<Range>,
    area: Option<Range>,
    faces: Option<usize>,
    shells: Option<usize>,
) -> Expect {
    Expect::Solid {
        volume,
        area,
        faces,
        shells,
    }
}
