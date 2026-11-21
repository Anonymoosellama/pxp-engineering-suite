// SPDX-License-Identifier: LGPL-2.1-or-later
//! The STEP-file stress corpus: real parts, read from STEP files by the
//! backend's own translator, then pushed through checks whose answers
//! **never come from the kernel under test** (the rule of the whole
//! bake-off).
//!
//! Written for the NIST MBE PMI test models (CTC 1-5, FTC 6-11, STC 6-10;
//! public domain, from <https://www.nist.gov/ctl/smart-connected-systems-division/smart-connected-manufacturing-systems-group/mbe-pmi-0>),
//! but any STEP file works: the harness needs no expected values compiled
//! in, because every grade is an invariant or a number the file states
//! about itself.
//!
//! ## Checks, per file
//!
//! | Check | Passes when | Reference |
//! | --- | --- | --- |
//! | `import` | every solid the file declares comes in closed and passes the kernel's own validity check ([`crate::GeometryKernel::check_validity`]), and open bodies number no more than the surface shells it declares as sheets (a tessellated file: at least one mesh body) | the file's own entities, counted by [`super::p21`] |
//! | `reference` | the closed solids' volume and area within [`REFERENCE_TOLERANCE`] (relative) and centroid within it × the box diagonal of the file's **geometric validation properties** | numbers the exporting CAD system wrote ([`super::p21`]); not applicable when the file states none |
//! | `native-roundtrip` | export/import of the backend's native bytes gives the same geometry hash | identity |
//! | `step-roundtrip` | writing the bodies as STEP and reading them back keeps volume, area, centroid within [`IDENTITY_TOLERANCE`] and the merged face count | the first import |
//! | `box-identity` | `A ∩ E`, with `E` a block 5 % of the diagonal larger than A's exact box, equals A (volume, area, merged faces) | identity |
//! | `half-split` | with `H` a block above a plane just past A's centroid: `V(A−H)+V(A∩H)=V(A)`, `V(A∪H)+V(A∩H)=V(A)+V(H)`, `S(A∪H)+S(A∩H)=S(A)+S(H)` (areas), and the section area implied by `A−H`, `A∩H` is not negative | inclusion-exclusion |
//! | `drill` | the same identities for a cylinder through A near its centroid along Z (a quarter of the smaller X/Y extent across) | inclusion-exclusion |
//!
//! The planes and axes are moved off the centroid by small odd fractions of
//! the box so they do not coincide with a face of a symmetric part by
//! accident: coincident faces are the synthetic corpus's job; these checks
//! ask whether a kernel stays exact on real, messy geometry.
//!
//! Every boolean and primitive goes through [`KernelHost::op`], so an
//! incomplete history is an error here as everywhere. A file **passes** when
//! no applicable check fails or errors. The whole procedure runs `repeats`
//! times; identical hashes of every body it makes (imports, round trips,
//! boolean results) make the file **deterministic**.

use super::Status;
use super::p21::{Declared, ValidationProperties, validation_properties};
use crate::{
    BlockSpec, BodyKind, BooleanOp, CylinderSpec, KernelError, KernelHost, ShapeHandle, TopoKind,
};
use pxp_math::{Csys, Point3, StableHasher, UnitVec3, Vec3};
use pxp_units::Length;
use std::fmt::Write as _;
use std::time::Instant;

/// Relative tolerance against a file's stated validation properties: the
/// exporting system computed them with its own (unknown) accuracy, and
/// SolidCraft's comparison with them uses 0.1 % as well.
pub const REFERENCE_TOLERANCE: f64 = 1e-3;

/// Relative tolerance for round trips and boolean identities: the Stage 1
/// gate tolerance (`pxp-io-step`'s invariant).
pub const IDENTITY_TOLERANCE: f64 = 1e-6;

/// One graded check of one file.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    /// Check name (`import`, `reference`, …).
    pub name: &'static str,
    /// `None`: not applicable to this file (said why in `detail`).
    pub status: Option<Status>,
    /// Measured values, residuals or the error.
    pub detail: String,
    /// Wall time of the check, seconds.
    pub seconds: f64,
}

/// What the stress procedure made of one STEP file.
#[derive(Debug, Clone, PartialEq)]
pub struct StepOutcome {
    /// File identifier (its stem).
    pub id: String,
    /// Size of the STEP text, bytes.
    pub bytes: usize,
    /// `FILE_SCHEMA` as the translator read it.
    pub schema: String,
    /// Worst applicable check of the first run.
    pub status: Status,
    /// Checks of the first run, in order.
    pub checks: Vec<Check>,
    /// Bodies imported (mesh bodies included).
    pub bodies: usize,
    /// Of those, facet (mesh) bodies.
    pub mesh_bodies: usize,
    /// Faces as the kernel stores them, all bodies.
    pub faces: usize,
    /// Faces with adjacent pieces of one surface merged, if the backend
    /// counts them.
    pub merged_faces: Option<usize>,
    /// Edges as stored.
    pub edges: usize,
    /// Vertices as stored.
    pub vertices: usize,
    /// Total imported volume, mm³.
    pub volume: Option<f64>,
    /// Total imported area, mm².
    pub area: Option<f64>,
    /// Volume-weighted centroid of the imported bodies, mm.
    pub centroid: Option<[f64; 3]>,
    /// What the file states about itself.
    pub reference: ValidationProperties,
    /// The translator's warnings (first run).
    pub warnings: Vec<String>,
    /// Wall time of the first run, seconds.
    pub seconds: f64,
    /// Wall time of the first import alone, seconds.
    pub import_seconds: f64,
    /// Hash over every body the first run made.
    pub hash: Option<u64>,
    /// Every repeat gave the same statuses and hash.
    pub deterministic: bool,
}

/// Shapes made during one run, released when it ends.
struct Owned<'h> {
    host: &'h KernelHost,
    shapes: Vec<ShapeHandle>,
}

impl Owned<'_> {
    fn keep(&mut self, s: ShapeHandle) -> ShapeHandle {
        self.shapes.push(s);
        s
    }
}

impl Drop for Owned<'_> {
    fn drop(&mut self) {
        for s in &self.shapes {
            let _ = self.host.call(|k| {
                k.release(*s);
                Ok(())
            });
        }
    }
}

/// Volume, area and centroid of a shape or a set of shapes.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Meas {
    v: f64,
    a: f64,
    c: [f64; 3],
}

impl Meas {
    /// Volume-weighted (area-weighted for sheets) combination.
    fn combine(items: &[Self]) -> Self {
        let v: f64 = items.iter().map(|m| m.v).sum();
        let a: f64 = items.iter().map(|m| m.a).sum();
        let (w, total) = if v.abs() > 0.0 {
            (items.iter().map(|m| m.v).collect::<Vec<_>>(), v)
        } else {
            (items.iter().map(|m| m.a).collect::<Vec<_>>(), a)
        };
        let mut c = [0.0; 3];
        if total.abs() > 0.0 {
            for (m, wi) in items.iter().zip(w) {
                for (ci, mi) in c.iter_mut().zip(m.c) {
                    *ci += mi * wi / total;
                }
            }
        }
        Self { v, a, c }
    }
}

fn meas(host: &KernelHost, s: ShapeHandle) -> Result<Meas, KernelError> {
    let m = host.call(|k| k.mass_properties(s))?;
    Ok(Meas {
        v: m.volume.mm3(),
        a: m.area.mm2(),
        c: [m.centroid.x, m.centroid.y, m.centroid.z],
    })
}

/// Geometry hash; for a shape the backend cannot hash exactly (a mesh
/// body), a hash of its native bytes, which is just as deterministic.
fn shape_hash(host: &KernelHost, s: ShapeHandle) -> Result<u64, KernelError> {
    match host.call(|k| k.geometry_hash(s, super::HASH_RESOLUTION)) {
        Err(KernelError::Unsupported { .. }) => {
            let bytes = host.call(|k| k.export_native(s))?;
            let mut h = StableHasher::new();
            h.write_bytes(&bytes);
            Ok(h.finish())
        }
        other => other,
    }
}

fn merged_faces(host: &KernelHost, s: ShapeHandle) -> Result<Option<usize>, KernelError> {
    match host.call(|k| k.merged_counts(s)) {
        Ok(m) => Ok(Some(m.faces)),
        Err(KernelError::Unsupported { .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

fn rel(value: f64, reference: f64, scale: f64) -> f64 {
    if scale.abs() > 0.0 {
        (value - reference).abs() / scale.abs()
    } else {
        (value - reference).abs()
    }
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y) * (x - y))
        .sum::<f64>()
        .sqrt()
}

/// Exact box corner and size of a shape.
fn bbox(host: &KernelHost, s: ShapeHandle) -> Result<(Point3, Vec3), KernelError> {
    let b = host.call(|k| k.bounding_box(s))?;
    match (b.min(), b.max()) {
        (Some(lo), Some(hi)) => Ok((lo, hi - lo)),
        _ => Err(KernelError::GeometryFailure {
            op: "bounding_box",
            reason: "empty box".into(),
            entities: Vec::new(),
        }),
    }
}

fn block_spec(corner: Point3, size: Vec3) -> Result<BlockSpec, KernelError> {
    let csys = Csys::new(corner, Vec3::X, Vec3::Y)
        .map_err(|e| KernelError::InvalidInput(format!("block placement: {e:?}")))?;
    Ok(BlockSpec {
        csys,
        x: Length::from_mm(size.x),
        y: Length::from_mm(size.y),
        z: Length::from_mm(size.z),
    })
}

/// Runs `f` as check `name`, turning a kernel error into an `Error` grade.
fn attempt(
    name: &'static str,
    f: impl FnOnce() -> Result<(Option<Status>, String), KernelError>,
) -> Check {
    let t = Instant::now();
    let (status, detail) = f().unwrap_or_else(|e| (Some(Status::Error), e.to_string()));
    Check {
        name,
        status,
        detail,
        seconds: t.elapsed().as_secs_f64(),
    }
}

const fn graded(ok: bool) -> Status {
    if ok { Status::Pass } else { Status::Fail }
}

/// What one run measured, besides its checks.
#[derive(Default)]
struct Summary {
    schema: String,
    bodies: usize,
    mesh_bodies: usize,
    faces: usize,
    merged_faces: Option<usize>,
    edges: usize,
    vertices: usize,
    total: Option<Meas>,
    warnings: Vec<String>,
    import_seconds: f64,
}

struct Run {
    checks: Vec<Check>,
    hash: Option<u64>,
    summary: Summary,
}

/// One body of the import, measured.
struct Body {
    shape: ShapeHandle,
    mesh: bool,
    closed: bool,
    /// The kernel's own validity check: the tests it fails (empty: valid);
    /// `None` when the backend has no check or the body is a mesh.
    invalid: Option<Vec<String>>,
    meas: Meas,
    merged: Option<usize>,
}

/// Counts, measures, hashes and checks one imported body.
fn measure_body(
    host: &KernelHost,
    s: &crate::ImportedShape,
    summary: &mut Summary,
    hasher: &mut StableHasher,
) -> Result<Body, KernelError> {
    // A facet (mesh) body has triangles, not B-rep faces: the topology
    // queries refuse it, so it adds no face counts.
    let merged = if s.mesh {
        None
    } else {
        let topo = host.call(|k| k.topology(s.shape))?;
        let count = |kind| topo.iter().filter(|t| t.kind == kind).count();
        summary.faces += count(TopoKind::Face);
        summary.edges += count(TopoKind::Edge);
        summary.vertices += count(TopoKind::Vertex);
        merged_faces(host, s.shape)?
    };
    summary.merged_faces = summary.merged_faces.zip(merged).map(|(a, b)| a + b);
    hasher.write_u64(shape_hash(host, s.shape)?);
    let invalid = if s.mesh {
        None
    } else {
        match host.call(|k| k.check_validity(s.shape)) {
            Ok(failed) => Some(failed),
            Err(KernelError::Unsupported { .. }) => None,
            Err(e) => return Err(e),
        }
    };
    Ok(Body {
        shape: s.shape,
        mesh: s.mesh,
        closed: s.closed,
        invalid,
        meas: meas(host, s.shape)?,
        merged,
    })
}

/// The import check: read, count, measure and hash every body.
fn import(
    host: &KernelHost,
    owned: &mut Owned<'_>,
    text: &str,
    declared: Declared,
    summary: &mut Summary,
    hasher: &mut StableHasher,
) -> (Check, Vec<Body>) {
    let mut bodies = Vec::new();
    let t = Instant::now();
    let check = attempt("import", || {
        let imp = host.call(|k| k.import_step(text))?;
        for s in &imp.shapes {
            owned.keep(s.shape);
        }
        summary.import_seconds = t.elapsed().as_secs_f64();
        summary.schema.clone_from(&imp.schema);
        summary.warnings.clone_from(&imp.warnings);
        summary.merged_faces = Some(0);
        for s in &imp.shapes {
            bodies.push(measure_body(host, s, summary, hasher)?);
        }
        summary.bodies = bodies.len();
        summary.mesh_bodies = bodies.iter().filter(|b| b.mesh).count();
        if bodies.is_empty() {
            return Ok((
                Some(Status::Error),
                format!(
                    "no bodies read; {} warnings: {}",
                    imp.warnings.len(),
                    imp.warnings.first().map_or("", String::as_str)
                ),
            ));
        }
        summary.total = Some(Meas::combine(
            &bodies
                .iter()
                .filter(|b| !b.mesh && b.closed)
                .map(|b| b.meas)
                .collect::<Vec<_>>(),
        ));
        // Graded on what the file declares (counted by the independent
        // reader, not the translator): every solid must come in closed;
        // open bodies are fine up to the surface shells the file declares
        // as sheets, and anything beyond that is a solid that failed.
        // A closed solid that the kernel's own check fails is not ready
        // for modeling (later operations refuse it), so it fails the import.
        let solids = bodies.iter().filter(|b| !b.mesh && b.closed).count();
        let open = bodies.iter().filter(|b| !b.mesh && !b.closed).count();
        let failing: Vec<&Body> = bodies
            .iter()
            .filter(|b| !b.mesh && b.closed && b.invalid.as_ref().is_some_and(|f| !f.is_empty()))
            .collect();
        let ok = if declared.solids == 0 && declared.tessellated > 0 {
            summary.mesh_bodies > 0
        } else {
            solids == declared.solids && open <= declared.surface_shells && failing.is_empty()
        };
        let mut detail = format!(
            "{solids}/{} solids closed, {open} sheet bod{} (file declares {} surface model(s), {} shell(s)), {} mesh, {} faces ({} merged), {} warnings",
            declared.solids,
            if open == 1 { "y" } else { "ies" },
            declared.surface_models,
            declared.surface_shells,
            summary.mesh_bodies,
            summary.faces,
            summary
                .merged_faces
                .map_or_else(|| "?".to_owned(), |n| n.to_string()),
            imp.warnings.len()
        );
        let checked = bodies
            .iter()
            .filter(|b| !b.mesh && b.closed && b.invalid.is_some())
            .count();
        let _ = write!(
            detail,
            "; kernel check: {}/{checked} solids valid",
            checked - failing.len()
        );
        if let Some(f) = failing.first() {
            let _ = write!(
                detail,
                " (fails: {})",
                f.invalid.as_deref().unwrap_or_default().join("; ")
            );
        }
        if !ok {
            let _ = write!(
                detail,
                "; first warning: {}",
                imp.warnings.first().map_or("(none)", String::as_str)
            );
        }
        Ok((Some(graded(ok)), detail))
    });
    (check, bodies)
}

fn reference_check(host: &KernelHost, reference: &ValidationProperties, bodies: &[Body]) -> Check {
    attempt("reference", || {
        if !reference.any() {
            return Ok((
                None,
                "the file states no geometric validation properties".into(),
            ));
        }
        // The file's properties describe its solids; sheet bodies (open)
        // and meshes are not part of them.
        let solids: Vec<&Body> = bodies.iter().filter(|b| !b.mesh && b.closed).collect();
        if solids.is_empty() {
            return Ok((None, "no closed solid to compare (see import)".into()));
        }
        let total = Meas::combine(&solids.iter().map(|b| b.meas).collect::<Vec<_>>());
        let mut diag = 0.0_f64;
        for b in &solids {
            diag = diag.max(bbox(host, b.shape)?.1.length());
        }
        if let Some(r) = reference.unit_mismatch {
            // The file labels its properties in one unit and models in
            // another: not a fair reference. Show how the read part
            // compares with the values taken in the model's unit, ungraded.
            let mut parts = vec![format!(
                "file labels its properties in a unit {r}× smaller than its model's; ungraded"
            )];
            if let Some(v) = reference.volume {
                let d = rel(total.v, v * r.powi(3), v * r.powi(3));
                parts.push(format!("against the model unit: ΔV {d:.1e}"));
            }
            if let Some(a) = reference.area {
                let d = rel(total.a, a * r * r, a * r * r);
                parts.push(format!("ΔA {d:.1e}"));
            }
            return Ok((None, parts.join("; ")));
        }
        let mut ok = true;
        let mut parts = Vec::new();
        if let Some(v) = reference.volume {
            let d = rel(total.v, v, v);
            ok &= d <= REFERENCE_TOLERANCE;
            parts.push(format!("ΔV {d:.1e} (file {v:.6e}, read {:.6e})", total.v));
        }
        if let Some(a) = reference.area {
            let d = rel(total.a, a, a);
            ok &= d <= REFERENCE_TOLERANCE;
            parts.push(format!("ΔA {d:.1e} (file {a:.6e}, read {:.6e})", total.a));
        }
        if let Some(c) = reference.centroid {
            let d = dist(total.c, c) / diag.max(f64::MIN_POSITIVE);
            ok &= d <= REFERENCE_TOLERANCE;
            parts.push(format!("ΔC {d:.1e}·diag"));
        }
        Ok((Some(graded(ok)), parts.join("; ")))
    })
}

fn native_roundtrip(
    host: &KernelHost,
    owned: &mut Owned<'_>,
    bodies: &[Body],
    hasher: &mut StableHasher,
) -> Check {
    attempt("native-roundtrip", || {
        let mut same = 0;
        let mut bytes = 0;
        for b in bodies {
            let native = host.call(|k| k.export_native(b.shape))?;
            bytes += native.len();
            let back = owned.keep(host.call(|k| k.import_native(&native))?);
            let (h1, h2) = (shape_hash(host, b.shape)?, shape_hash(host, back)?);
            hasher.write_u64(h2);
            if h1 == h2 {
                same += 1;
            }
        }
        Ok((
            Some(graded(same == bodies.len())),
            format!(
                "{same}/{} same hash, {} KiB native",
                bodies.len(),
                bytes / 1024
            ),
        ))
    })
}

fn step_roundtrip(
    host: &KernelHost,
    owned: &mut Owned<'_>,
    bodies: &[Body],
    hasher: &mut StableHasher,
) -> Check {
    attempt("step-roundtrip", || {
        let exact: Vec<&Body> = bodies.iter().filter(|b| !b.mesh).collect();
        if exact.is_empty() {
            return Ok((None, "no exact (B-rep) bodies to write".into()));
        }
        let handles: Vec<ShapeHandle> = exact.iter().map(|b| b.shape).collect();
        let text = host.call(|k| k.export_step(&handles))?;
        let imp = host.call(|k| k.import_step(&text))?;
        for s in &imp.shapes {
            owned.keep(s.shape);
        }
        let mut back = Vec::new();
        let mut merged_back = Some(0);
        for s in &imp.shapes {
            back.push(meas(host, s.shape)?);
            merged_back = merged_back
                .zip(merged_faces(host, s.shape)?)
                .map(|(a, b)| a + b);
            hasher.write_u64(shape_hash(host, s.shape)?);
        }
        let before = Meas::combine(&exact.iter().map(|b| b.meas).collect::<Vec<_>>());
        let after = Meas::combine(&back);
        let merged_before = exact.iter().try_fold(0, |acc, b| b.merged.map(|m| acc + m));
        let mut diag = 0.0_f64;
        for b in &exact {
            diag = diag.max(bbox(host, b.shape)?.1.length());
        }
        let (dv, da, dc) = (
            rel(after.v, before.v, before.v),
            rel(after.a, before.a, before.a),
            dist(after.c, before.c) / diag.max(f64::MIN_POSITIVE),
        );
        let closed_before = exact.iter().filter(|b| b.closed).count();
        let closed_after = imp.shapes.iter().filter(|s| s.closed).count();
        let closed = closed_after == closed_before;
        let faces_same = merged_before.is_none() || merged_before == merged_back;
        let ok = imp.shapes.len() == exact.len()
            && closed
            && dv <= IDENTITY_TOLERANCE
            && da <= IDENTITY_TOLERANCE
            && dc <= IDENTITY_TOLERANCE
            && faces_same;
        let show = |n: Option<usize>| n.map_or_else(|| "?".to_owned(), |n| n.to_string());
        Ok((
            Some(graded(ok)),
            format!(
                "{} → {} bodies{}, ΔV {dv:.1e} ΔA {da:.1e} ΔC {dc:.1e}·diag, merged faces {} → {}, {} KiB STEP",
                exact.len(),
                imp.shapes.len(),
                if closed {
                    String::new()
                } else {
                    format!(" (closed {closed_before} → {closed_after})")
                },
                show(merged_before),
                show(merged_back),
                text.len() / 1024
            ),
        ))
    })
}

/// A boolean whose empty result counts as zero material (an intersection
/// or subtraction may legitimately leave nothing).
fn boolean(
    host: &KernelHost,
    owned: &mut Owned<'_>,
    op: BooleanOp,
    lhs: ShapeHandle,
    rhs: ShapeHandle,
    hasher: &mut StableHasher,
) -> Result<(Meas, bool), KernelError> {
    match host.op(|k| k.boolean(op, lhs, rhs)) {
        Ok(r) => {
            let s = owned.keep(r.shape);
            hasher.write_u64(shape_hash(host, s)?);
            let solid = host.call(|k| k.body_kind(s))? == BodyKind::Solid;
            Ok((meas(host, s)?, solid))
        }
        Err(KernelError::EmptyResult { .. }) if op != BooleanOp::Unite => {
            hasher.write_str("empty");
            Ok((Meas::default(), true))
        }
        Err(e) => Err(e),
    }
}

/// `A − T`, `A ∩ T`, `A ∪ T` and their inclusion-exclusion identities.
fn identities(
    host: &KernelHost,
    owned: &mut Owned<'_>,
    a: ShapeHandle,
    ma: Meas,
    tool: ShapeHandle,
    hasher: &mut StableHasher,
) -> Result<(Option<Status>, String), KernelError> {
    let mt = meas(host, tool)?;
    let (sub, s1) = boolean(host, owned, BooleanOp::Subtract, a, tool, hasher)?;
    let (int, s2) = boolean(host, owned, BooleanOp::Intersect, a, tool, hasher)?;
    let (uni, s3) = boolean(host, owned, BooleanOp::Unite, a, tool, hasher)?;
    let e1 = rel(sub.v + int.v, ma.v, ma.v);
    let e2 = rel(uni.v + int.v, ma.v + mt.v, ma.v + mt.v);
    let e3 = rel(uni.a + int.a, ma.a + mt.a, ma.a + mt.a);
    let section = (sub.a + int.a - ma.a) / 2.0;
    let section_ok = section >= -IDENTITY_TOLERANCE * ma.a;
    let solids = s1 && s2 && s3;
    let ok = e1 <= IDENTITY_TOLERANCE
        && e2 <= IDENTITY_TOLERANCE
        && e3 <= IDENTITY_TOLERANCE
        && section_ok
        && solids;
    Ok((
        Some(graded(ok)),
        format!(
            "V(A−T) {:.6e}, V(A∩T) {:.6e}, V(A∪T) {:.6e}; residuals {e1:.1e} {e2:.1e} {e3:.1e}; section {section:.4e} mm²{}",
            sub.v,
            int.v,
            uni.v,
            if solids {
                ""
            } else {
                "; a result is not a solid"
            }
        ),
    ))
}

fn boolean_checks(
    host: &KernelHost,
    owned: &mut Owned<'_>,
    bodies: &[Body],
    hasher: &mut StableHasher,
) -> Vec<Check> {
    let target = bodies
        .iter()
        .filter(|b| !b.mesh && b.closed)
        .max_by(|x, y| x.meas.v.total_cmp(&y.meas.v));
    let Some(target) = target else {
        let why = "no closed exact body to cut".to_owned();
        return ["box-identity", "half-split", "drill"]
            .into_iter()
            .map(|name| Check {
                name,
                status: None,
                detail: why.clone(),
                seconds: 0.0,
            })
            .collect();
    };
    let (part, ma) = (target.shape, target.meas);
    let bb = bbox(host, part);
    let mut out = Vec::new();
    out.push(attempt("box-identity", || {
        let (lo, size) = bb.clone()?;
        let margin = 0.05 * size.length();
        let grow = Vec3::new(margin, margin, margin);
        let enclosing = owned.keep(
            host.op(|k| k.block(&block_spec(lo - grow, size + grow * 2.0)?))?
                .shape,
        );
        let (common, solid) = boolean(host, owned, BooleanOp::Intersect, part, enclosing, hasher)?;
        let (dv, da) = (rel(common.v, ma.v, ma.v), rel(common.a, ma.a, ma.a));
        Ok((
            Some(graded(
                dv <= IDENTITY_TOLERANCE && da <= IDENTITY_TOLERANCE && solid,
            )),
            format!("ΔV {dv:.1e} ΔA {da:.1e}"),
        ))
    }));
    out.push(attempt("half-split", || {
        let (lo, size) = bb.clone()?;
        let margin = 0.05 * size.length();
        // Just past the centroid (0.37 % of the height), so the plane cuts
        // material but is unlikely to lie on one of the part's faces.
        let z0 = ma.c[2] + 0.0037 * size.z;
        let corner = Point3::new(lo.x - margin, lo.y - margin, z0);
        let extent = Vec3::new(
            size.x + 2.0 * margin,
            size.y + 2.0 * margin,
            lo.z + size.z + margin - z0,
        );
        let tool = owned.keep(host.op(|k| k.block(&block_spec(corner, extent)?))?.shape);
        identities(host, owned, part, ma, tool, hasher)
    }));
    out.push(attempt("drill", || {
        let (lo, size) = bb.clone()?;
        let margin = 0.05 * size.length();
        // Near the centroid first; a hollow part (a tube, a ring) can have
        // nothing there, so then a few fixed points spread over the box.
        // The first drill that meets material is graded, so the identities
        // test a real cut rather than holding trivially.
        let candidates = [
            (ma.c[0] + 0.0213 * size.x, ma.c[1] + 0.0171 * size.y),
            (lo.x + 0.303 * size.x, lo.y + 0.371 * size.y),
            (lo.x + 0.631 * size.x, lo.y + 0.289 * size.y),
            (lo.x + 0.413 * size.x, lo.y + 0.707 * size.y),
            (lo.x + 0.769 * size.x, lo.y + 0.653 * size.y),
        ];
        let mut tool = None;
        let mut tried = 0;
        for (x, y) in candidates {
            tried += 1;
            let spec = CylinderSpec {
                base: Point3::new(x, y, lo.z - margin),
                axis: UnitVec3::Z,
                diameter: Length::from_mm(0.25 * size.x.min(size.y)),
                height: Length::from_mm(size.z + 2.0 * margin),
            };
            let t = owned.keep(host.op(|k| k.cylinder(&spec))?.shape);
            tool = Some(t);
            match host.op(|k| k.boolean(BooleanOp::Intersect, part, t)) {
                Ok(r) => {
                    owned.keep(r.shape);
                    break;
                }
                Err(KernelError::EmptyResult { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        let tool = tool.ok_or_else(|| KernelError::InvalidInput("no drill position".into()))?;
        let (status, detail) = identities(host, owned, part, ma, tool, hasher)?;
        let place = if tried == 1 {
            "near the centroid".to_owned()
        } else {
            format!("at position {tried} of {}", candidates.len())
        };
        Ok((status, format!("drill {place}; {detail}")))
    }));
    out
}

fn run_once(host: &KernelHost, text: &str, reference: &ValidationProperties) -> Run {
    let mut owned = Owned {
        host,
        shapes: Vec::new(),
    };
    let mut hasher = StableHasher::new();
    let mut summary = Summary::default();
    let (import_check, bodies) = import(
        host,
        &mut owned,
        text,
        reference.declared,
        &mut summary,
        &mut hasher,
    );
    let failed_import = import_check.status == Some(Status::Error);
    let mut checks = vec![import_check];
    if !failed_import {
        checks.push(reference_check(host, reference, &bodies));
        checks.push(native_roundtrip(host, &mut owned, &bodies, &mut hasher));
        checks.push(step_roundtrip(host, &mut owned, &bodies, &mut hasher));
        checks.extend(boolean_checks(host, &mut owned, &bodies, &mut hasher));
    }
    for c in &checks {
        hasher.write_str(c.name);
        hasher.write_str(c.status.map_or("n/a", Status::label));
    }
    Run {
        checks,
        hash: (!failed_import).then(|| hasher.finish()),
        summary,
    }
}

/// Runs the stress procedure on one STEP file `repeats` times (at least
/// once).
#[must_use]
pub fn run_step_file(host: &KernelHost, id: &str, text: &str, repeats: usize) -> StepOutcome {
    let reference = validation_properties(text);
    let start = Instant::now();
    let first = run_once(host, text, &reference);
    let seconds = start.elapsed().as_secs_f64();
    let statuses = |r: &Run| r.checks.iter().map(|c| c.status).collect::<Vec<_>>();
    let deterministic = (1..repeats).all(|_| {
        let again = run_once(host, text, &reference);
        again.hash == first.hash && statuses(&again) == statuses(&first)
    });
    let status = first
        .checks
        .iter()
        .filter_map(|c| c.status)
        .max()
        .unwrap_or(Status::Error);
    let s = first.summary;
    StepOutcome {
        id: id.to_owned(),
        bytes: text.len(),
        schema: s.schema,
        status,
        checks: first.checks,
        bodies: s.bodies,
        mesh_bodies: s.mesh_bodies,
        faces: s.faces,
        merged_faces: s.merged_faces,
        edges: s.edges,
        vertices: s.vertices,
        volume: s.total.map(|m| m.v),
        area: s.total.map(|m| m.a),
        centroid: s.total.map(|m| m.c),
        reference,
        warnings: s.warnings,
        seconds,
        import_seconds: s.import_seconds,
        hash: first.hash,
        deterministic,
    }
}

/// Check names in report order.
pub const CHECKS: [&str; 7] = [
    "import",
    "reference",
    "native-roundtrip",
    "step-roundtrip",
    "box-identity",
    "half-split",
    "drill",
];

fn label(c: Option<&Check>) -> &'static str {
    match c.map(|c| c.status) {
        None => "—",
        Some(None) => "n/a",
        Some(Some(s)) => s.label(),
    }
}

impl StepOutcome {
    /// The check called `name`, if it ran.
    #[must_use]
    pub fn check(&self, name: &str) -> Option<&Check> {
        self.checks.iter().find(|c| c.name == name)
    }

    /// One tab-separated line: id, status, sizes, measures, each check's
    /// grade, times, hash and determinism (for comparing runs and OSes).
    #[must_use]
    pub fn tsv(&self) -> String {
        let f = |v: Option<f64>| v.map_or_else(|| "-".to_owned(), |v| format!("{v:.12e}"));
        let c = self.centroid.map_or_else(
            || "-".to_owned(),
            |c| format!("{:.9e};{:.9e};{:.9e}", c[0], c[1], c[2]),
        );
        let mut line = format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{c}\t{}\t{}",
            self.id,
            self.status.label(),
            self.bytes,
            self.schema.split_whitespace().next().unwrap_or("-"),
            self.bodies,
            self.mesh_bodies,
            self.faces,
            self.merged_faces
                .map_or_else(|| "-".to_owned(), |n| n.to_string()),
            self.edges,
            self.vertices,
            f(self.volume),
            f(self.area),
            f(self.reference.volume),
            f(self.reference.area),
        );
        for name in CHECKS {
            let _ = write!(line, "\t{}", label(self.check(name)));
        }
        let _ = write!(
            line,
            "\t{:.3}\t{:.3}\t{}\t{}\t{}",
            self.import_seconds,
            self.seconds,
            self.hash
                .map_or_else(|| "-".to_owned(), |h| format!("{h:016x}")),
            if self.deterministic { "yes" } else { "no" },
            self.warnings.len()
        );
        line
    }

    /// The header line for [`StepOutcome::tsv`].
    #[must_use]
    pub fn tsv_header() -> String {
        let mut line = "id\tstatus\tbytes\tschema\tbodies\tmesh\tfaces\tmerged_faces\tedges\tvertices\tvolume_mm3\tarea_mm2\tcentroid_mm\tref_volume_mm3\tref_area_mm2".to_owned();
        for name in CHECKS {
            let _ = write!(line, "\t{name}");
        }
        line.push_str("\timport_s\ttotal_s\thash\tdeterministic\twarnings");
        line
    }

    /// Every check's detail, then the translator's warnings (Markdown).
    #[must_use]
    pub fn details_markdown(&self) -> String {
        let mut out = format!(
            "### `{}` — {}\n\n{} KiB, {}, {:.1} s (import {:.1} s), same hash on every run: {}\n\n",
            self.id,
            self.status.label(),
            self.bytes / 1024,
            self.schema.split_whitespace().next().unwrap_or("?"),
            self.seconds,
            self.import_seconds,
            if self.deterministic { "yes" } else { "**no**" }
        );
        for c in &self.checks {
            let _ = writeln!(
                out,
                "- **{}** ({}, {:.2} s): {}",
                c.name,
                label(Some(c)),
                c.seconds,
                c.detail
            );
        }
        if !self.reference.notes.is_empty() {
            let _ = writeln!(
                out,
                "- reference notes: {}",
                self.reference.notes.join("; ")
            );
        }
        if !self.warnings.is_empty() {
            let _ = writeln!(out, "- translator warnings ({}):", self.warnings.len());
            for w in self.warnings.iter().take(8) {
                let _ = writeln!(out, "  - {}", w.replace('\n', " "));
            }
            if self.warnings.len() > 8 {
                let _ = writeln!(out, "  - … {} more", self.warnings.len() - 8);
            }
        }
        out
    }
}

/// A Markdown report of several files: totals per check, one row per file,
/// then the details of every file that did not pass.
#[must_use]
pub fn to_markdown(backend: &str, repeats: usize, outcomes: &[StepOutcome]) -> String {
    let mut out = format!(
        "Backend `{backend}`, {} file(s), {repeats} run(s) each.\n\n| Check | Pass | Fail | Error | n/a |\n| --- | ---: | ---: | ---: | ---: |\n",
        outcomes.len()
    );
    for name in CHECKS {
        let mut tally = [0_usize; 4];
        for o in outcomes {
            let column = match o.check(name).map(|c| c.status) {
                Some(Some(Status::Pass)) => 0,
                Some(Some(Status::Fail)) => 1,
                Some(Some(Status::Error)) | None => 2,
                Some(None) => 3,
            };
            if let Some(slot) = tally.get_mut(column) {
                *slot += 1;
            }
        }
        let [pass, fail, error, na] = tally;
        let _ = writeln!(out, "| `{name}` | {pass} | {fail} | {error} | {na} |");
    }
    let det = outcomes.iter().filter(|o| o.deterministic).count();
    let _ = writeln!(
        out,
        "\nSame hash on every run: {det} / {} files.\n",
        outcomes.len()
    );
    let _ = writeln!(
        out,
        "| File | Result | Faces (merged) | Import | Reference | Native RT | STEP RT | Box | Half split | Drill | Time (s) | Same hash |"
    );
    let _ = writeln!(
        out,
        "| --- | --- | ---: | --- | --- | --- | --- | --- | --- | --- | ---: | --- |"
    );
    for o in outcomes {
        let mut row = format!(
            "| `{}` | {} | {} ({}) |",
            o.id,
            o.status.label(),
            o.faces,
            o.merged_faces
                .map_or_else(|| "?".to_owned(), |n| n.to_string())
        );
        for name in CHECKS {
            let _ = write!(row, " {} |", label(o.check(name)));
        }
        let _ = write!(
            row,
            " {:.1} | {} |",
            o.seconds,
            if o.deterministic { "yes" } else { "**no**" }
        );
        let _ = writeln!(out, "{row}");
    }
    for o in outcomes.iter().filter(|o| o.status != Status::Pass) {
        let _ = write!(out, "\n{}", o.details_markdown());
    }
    out
}
