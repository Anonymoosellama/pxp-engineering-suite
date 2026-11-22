// SPDX-License-Identifier: LGPL-2.1-or-later
//! `pxp-cli part …`: headless part files with primitives (sub-stage 0G).
//!
//! Each command opens the `.pxp` (taking its lock), changes it in one
//! transaction, recomputes through the Modeling application and the chosen
//! kernel, saves atomically and closes. Running two commands one after the
//! other is therefore "save, close, reopen" — the Stage 0 checklist item
//! `part verify` checks:
//!
//! 1. every stored exact B-rep, re-imported into the kernel, hashes to the
//!    stored `geometry_hash`;
//! 2. regenerating every feature from its parameters gives the same hash
//!    and byte-identical B-rep.
//!
//! Bare numbers are taken in the part's length unit (`10` in an inch part is
//! `10 in`), as a dialog field would; anything else is an expression
//! formula (`p0 * 2`, `25.4 mm`).

use crate::CliError;
use pxp_app_modeling::primitives::{
    self, BLOCK, CONE, CYLINDER, DATUM_CSYS, GEOMETRY_HASH, KERNEL, SPHERE,
};
use pxp_appkit::AppRegistry;
use pxp_compute::CancelToken;
use pxp_doc::{Document, Object, ObjectKind, ObjectState, Value};
use pxp_format::{Opened, PartFile, PartMeta};
use pxp_kernel::{DEFAULT_HASH_RESOLUTION, KernelHost, decode_stored_shape};
use pxp_math::{Point3, Vec3};
use pxp_units::UnitSystem;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

pub(crate) const USAGE: &str = "\
usage: pxp-cli part <command> FILE [options]
  new FILE [--units mm|in]                 new part from the mm or inch template (datum CSYS at the origin)
  block FILE --size X Y Z [--on-datum] [--name N]
  cylinder FILE --diameter D --height H [--base X Y Z] [--axis I J K] [--name N]
  cone FILE --diameters D1 D2 --height H [--base X Y Z] [--axis I J K] [--name N]
  sphere FILE --diameter D [--center X Y Z] [--name N]
  info FILE                                features, states, kernels, hashes, mass properties of stored B-reps
  verify FILE                              re-import stored B-reps and regenerate every feature; hashes must match
  recover FILE | discard-recovery FILE     apply or drop unsaved work from an interrupted session";

fn cmd_err(e: impl std::fmt::Display) -> CliError {
    CliError::Command(e.to_string())
}

fn usage(m: impl Into<String>) -> CliError {
    CliError::Usage(format!("{}\n\n{USAGE}", m.into()))
}

/// Parsed `--flag v1 v2 …` options.
struct Opts<'a> {
    rest: &'a [String],
}

impl<'a> Opts<'a> {
    /// The values after `flag`, exactly `n` of them, if the flag is present.
    fn values(&self, flag: &str, n: usize) -> Result<Option<Vec<&'a str>>, CliError> {
        let Some(pos) = self.rest.iter().position(|a| a == flag) else {
            return Ok(None);
        };
        let vals: Vec<&str> = self
            .rest
            .iter()
            .skip(pos + 1)
            .take(n)
            .map(String::as_str)
            .collect();
        if vals.len() != n || vals.iter().any(|v| v.starts_with("--")) {
            return Err(usage(format!("{flag} needs {n} value(s)")));
        }
        Ok(Some(vals))
    }

    /// Exactly `N` values after `flag`, if present.
    fn array<const N: usize>(&self, flag: &str) -> Result<Option<[&'a str; N]>, CliError> {
        self.values(flag, N)?
            .map(|v| {
                <[&str; N]>::try_from(v).map_err(|_| usage(format!("{flag} needs {N} value(s)")))
            })
            .transpose()
    }

    fn one(&self, flag: &str) -> Result<Option<&'a str>, CliError> {
        Ok(self.array::<1>(flag)?.map(|[v]| v))
    }

    fn required<const N: usize>(&self, flag: &str) -> Result<[&'a str; N], CliError> {
        self.array::<N>(flag)?
            .ok_or_else(|| usage(format!("missing {flag}")))
    }

    fn has(&self, flag: &str) -> bool {
        self.rest.iter().any(|a| a == flag)
    }

    /// Rejects flags this command does not take.
    fn only(&self, allowed: &[&str]) -> Result<(), CliError> {
        for a in self.rest {
            if a.starts_with("--") && !allowed.contains(&a.as_str()) {
                return Err(usage(format!("unknown option {a}")));
            }
        }
        Ok(())
    }
}

/// A bare number gets the part's length unit; anything else is a formula.
fn length_formula(v: &str, units: UnitSystem) -> String {
    if v.trim().parse::<f64>().is_ok() {
        let unit = match units {
            UnitSystem::Millimetre => "mm",
            UnitSystem::Inch => "in",
        };
        format!("{}{unit}", v.trim())
    } else {
        v.to_owned()
    }
}

fn open(path: &Path) -> Result<(PartFile, Document), CliError> {
    let Opened {
        file,
        document,
        recovery,
        ..
    } = PartFile::open(path).map_err(cmd_err)?;
    if let Some(r) = recovery {
        return Err(CliError::Command(format!(
            "{} has unsaved work from an interrupted session ({} transaction(s), last: {}). \
             Run `pxp-cli part recover {}` to keep it or `pxp-cli part discard-recovery {}` to drop it.",
            path.display(),
            r.transactions,
            r.last_label,
            path.display(),
            path.display()
        )));
    }
    Ok((file, document))
}

/// Every feature of a part with its stored geometry hash (`None` for a
/// feature without geometry), in creation order: what a journal records.
///
/// # Errors
/// The part cannot be opened.
pub(crate) fn feature_hashes(path: &Path) -> Result<Vec<(String, Option<u64>)>, CliError> {
    let Opened { document, .. } = PartFile::open_read_only(path).map_err(cmd_err)?;
    let snap = document.snapshot();
    let mut features: Vec<&Object> = snap
        .objects()
        .filter(|o| primitives::is_primitive_kind(&o.kind))
        .collect();
    features.sort_by_key(|o| o.id());
    Ok(features
        .into_iter()
        .map(|o| {
            let name = o
                .name
                .clone()
                .unwrap_or_else(|| format!("{} {}", kind_label(&o.kind), o.id()));
            (name, stored_hash(o))
        })
        .collect())
}

fn feature_count(doc: &Document) -> usize {
    doc.snapshot()
        .objects()
        .filter(|o| primitives::is_primitive_kind(&o.kind))
        .count()
}

fn kind_label(kind: &ObjectKind) -> &'static str {
    [
        (DATUM_CSYS, "Datum Coordinate System"),
        (BLOCK, "Block"),
        (CYLINDER, "Cylinder"),
        (CONE, "Cone"),
        (SPHERE, "Sphere"),
    ]
    .into_iter()
    .find(|(k, _)| k == kind)
    .map_or("Object", |(_, label)| label)
}

fn stored_hash(o: &Object) -> Option<u64> {
    match o.prop(GEOMETRY_HASH) {
        Some(Value::Int(h)) => Some(h.cast_unsigned()),
        _ => None,
    }
}

/// Recomputes, refusing to save a part whose new feature failed.
fn recompute(doc: &mut Document, registry: &AppRegistry) -> Result<(), CliError> {
    let report = doc
        .recompute(registry, &CancelToken::new())
        .map_err(cmd_err)?;
    if let Some((id, why)) = report.failed.first() {
        return Err(CliError::Command(format!(
            "regenerating {id} failed: {why} (nothing saved)"
        )));
    }
    Ok(())
}

pub(crate) fn run(
    args: &[String],
    registry: &AppRegistry,
    kernel: &Arc<KernelHost>,
) -> Result<String, CliError> {
    let (sub, rest) = args
        .split_first()
        .ok_or_else(|| usage("part needs a command"))?;
    if matches!(sub.as_str(), "help" | "--help" | "-h") {
        return Ok(format!("{USAGE}\n"));
    }
    let (file, rest) = rest
        .split_first()
        .ok_or_else(|| usage(format!("part {sub} needs a FILE")))?;
    let path = Path::new(file);
    let opts = Opts { rest };
    match sub.as_str() {
        "new" => new_part(path, &opts),
        "block" | "cylinder" | "cone" | "sphere" => add_primitive(sub, path, &opts, registry),
        "info" => {
            opts.only(&[])?;
            info(path, kernel)
        }
        "verify" => {
            opts.only(&[])?;
            verify(path, registry, kernel)
        }
        "recover" => {
            opts.only(&[])?;
            let Opened { mut file, .. } = PartFile::open(path).map_err(cmd_err)?;
            let doc = file.recovered_document().map_err(cmd_err)?;
            let meta = file.meta();
            let r = file.save(&doc, meta).map_err(cmd_err)?;
            file.close().map_err(cmd_err)?;
            Ok(format!(
                "recovered and saved {} ({} objects)\n",
                path.display(),
                r.objects
            ))
        }
        "discard-recovery" => {
            opts.only(&[])?;
            let Opened { mut file, .. } = PartFile::open(path).map_err(cmd_err)?;
            file.discard_recovery().map_err(cmd_err)?;
            file.close().map_err(cmd_err)?;
            Ok(format!("discarded unsaved work of {}\n", path.display()))
        }
        other => Err(usage(format!("unknown part command '{other}'"))),
    }
}

fn new_part(path: &Path, opts: &Opts<'_>) -> Result<String, CliError> {
    opts.only(&["--units"])?;
    let meta = match opts.one("--units")? {
        None | Some("mm") => PartMeta::millimetre(),
        Some("in") => PartMeta::inch(),
        Some(other) => return Err(usage(format!("--units must be mm or in, got {other}"))),
    };
    let mut doc = Document::new();
    doc.transact("New part", |tx| {
        primitives::create_datum_csys(
            tx,
            "Datum Coordinate System(0)",
            Point3::ORIGIN,
            Vec3::X,
            Vec3::Y,
        )
    })
    .map_err(cmd_err)?;
    // The datum needs no kernel; recompute it so the template is up to date.
    let registry = crate::builtin_registry(None)?;
    recompute(&mut doc, &registry)?;
    let file = PartFile::create(path, meta, &doc).map_err(cmd_err)?;
    file.close().map_err(cmd_err)?;
    Ok(format!(
        "created {} ({} part, {} objects)\n",
        path.display(),
        if meta.units == UnitSystem::Inch {
            "inch"
        } else {
            "mm"
        },
        doc.snapshot().len()
    ))
}

fn add_primitive(
    kind: &str,
    path: &Path,
    opts: &Opts<'_>,
    registry: &AppRegistry,
) -> Result<String, CliError> {
    let (mut file, mut doc) = open(path)?;
    let meta = file.meta();
    let len = |v: &str| length_formula(v, meta.units);
    let n = feature_count(&doc);
    let triple =
        |flag: &str, default: [&str; 3], as_length: bool| -> Result<[String; 3], CliError> {
            let v = opts.array::<3>(flag)?.unwrap_or(default);
            Ok(v.map(|s| if as_length { len(s) } else { s.to_owned() }))
        };
    let label = kind_label(&ObjectKind::new(format!("modeling.{kind}")).map_err(cmd_err)?);
    let name = opts
        .one("--name")?
        .map_or_else(|| format!("{label}({n})"), str::to_owned);
    let id = match kind {
        "block" => {
            opts.only(&["--size", "--on-datum", "--name"])?;
            let size: [&str; 3] = opts.required("--size")?;
            let datum = if opts.has("--on-datum") {
                Some(
                    doc.snapshot()
                        .objects()
                        .find(|o| o.kind == DATUM_CSYS)
                        .map(Object::id)
                        .ok_or_else(|| CliError::Command("the part has no datum CSYS".into()))?,
                )
            } else {
                None
            };
            let [x, y, z] = size.map(len);
            doc.transact(&format!("Block {name}"), |tx| {
                primitives::create_block(tx, &name, [&x, &y, &z], datum)
            })
        }
        "cylinder" => {
            opts.only(&["--diameter", "--height", "--base", "--axis", "--name"])?;
            let [d] = opts.required("--diameter")?.map(len);
            let [h] = opts.required("--height")?.map(len);
            let [bx, by, bz] = triple("--base", ["0", "0", "0"], true)?;
            let [ax, ay, az] = triple("--axis", ["0", "0", "1"], false)?;
            doc.transact(&format!("Cylinder {name}"), |tx| {
                primitives::create_cylinder(tx, &name, [&bx, &by, &bz], [&ax, &ay, &az], &d, &h)
            })
        }
        "cone" => {
            opts.only(&["--diameters", "--height", "--base", "--axis", "--name"])?;
            let [d1, d2] = opts.required("--diameters")?.map(len);
            let [h] = opts.required("--height")?.map(len);
            let [bx, by, bz] = triple("--base", ["0", "0", "0"], true)?;
            let [ax, ay, az] = triple("--axis", ["0", "0", "1"], false)?;
            doc.transact(&format!("Cone {name}"), |tx| {
                primitives::create_cone(tx, &name, [&bx, &by, &bz], [&ax, &ay, &az], [&d1, &d2], &h)
            })
        }
        _ => {
            opts.only(&["--diameter", "--center", "--name"])?;
            let [d] = opts.required("--diameter")?.map(len);
            let [cx, cy, cz] = triple("--center", ["0", "0", "0"], true)?;
            doc.transact(&format!("Sphere {name}"), |tx| {
                primitives::create_sphere(tx, &name, [&cx, &cy, &cz], &d)
            })
        }
    }
    .map_err(cmd_err)?;
    recompute(&mut doc, registry)?;
    let snap = doc.snapshot();
    let obj = snap.get(id).ok_or_else(|| cmd_err("feature vanished"))?;
    let hash = stored_hash(obj).ok_or_else(|| cmd_err("feature has no geometry hash"))?;
    file.save(&doc, meta).map_err(cmd_err)?;
    file.close().map_err(cmd_err)?;
    Ok(format!(
        "{name}: created and saved to {} (geometry hash {hash:016x})\n",
        path.display()
    ))
}

fn info(path: &Path, kernel: &KernelHost) -> Result<String, CliError> {
    let Opened { file, document, .. } = PartFile::open_read_only(path).map_err(cmd_err)?;
    let meta = file.meta();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} ({} part)",
        path.display(),
        if meta.units == UnitSystem::Inch {
            "inch"
        } else {
            "mm"
        }
    );
    let snap = document.snapshot();
    let mut features: Vec<&Object> = snap
        .objects()
        .filter(|o| primitives::is_primitive_kind(&o.kind))
        .collect();
    features.sort_by_key(|o| o.id());
    for o in features {
        let name = o.name.as_deref().unwrap_or("?");
        let state = match o.state {
            ObjectState::UpToDate => "up to date",
            ObjectState::OutOfDate => "out of date",
            ObjectState::Failed => "FAILED",
            ObjectState::Suppressed => "suppressed",
        };
        let mut line = format!("  {name:<30} {:<24} {state}", kind_label(&o.kind));
        if let Some(h) = stored_hash(o) {
            let backend = match o.prop(KERNEL) {
                Some(Value::Text(t)) => t.as_str(),
                _ => "?",
            };
            let _ = write!(line, "  kernel {backend}  hash {h:016x}");
            let stored = decode_stored_shape(&o.payload).map_err(cmd_err)?;
            if stored.backend == kernel.backend_id().0 {
                let shape = kernel
                    .call(|k| k.import_native(stored.native))
                    .map_err(cmd_err)?;
                let m = kernel.call(|k| k.mass_properties(shape));
                let _ = kernel.call(|k| {
                    k.release(shape);
                    Ok(())
                });
                let m = m.map_err(cmd_err)?;
                let fmt = |q: pxp_units::Quantity| {
                    meta.units.format(q, 4).unwrap_or_else(|_| format!("{q}"))
                };
                let _ = write!(
                    line,
                    "  volume {}  area {}",
                    fmt(m.volume.into()),
                    fmt(m.area.into())
                );
            }
        }
        let _ = writeln!(out, "{line}");
    }
    Ok(out)
}

fn verify(path: &Path, registry: &AppRegistry, kernel: &KernelHost) -> Result<String, CliError> {
    let Opened { document, .. } = PartFile::open_read_only(path).map_err(cmd_err)?;
    let mut doc = document;
    let before = doc.snapshot();
    let solids: Vec<&Object> = before
        .objects()
        .filter(|o| primitives::is_solid_kind(&o.kind))
        .collect();
    let mut out = String::new();
    let mut bad = 0_usize;

    // 1. Stored B-reps reproduce their stored hashes.
    for o in &solids {
        let name = o.name.as_deref().unwrap_or("?");
        let want = stored_hash(o).ok_or_else(|| cmd_err(format!("{name} has no stored hash")))?;
        let stored = decode_stored_shape(&o.payload).map_err(cmd_err)?;
        if stored.backend != kernel.backend_id().0 {
            let _ = writeln!(
                out,
                "  {name}: stored by kernel '{}', this build runs '{}': skipped",
                stored.backend,
                kernel.backend_id().0
            );
            continue;
        }
        let shape = kernel
            .call(|k| k.import_native(stored.native))
            .map_err(cmd_err)?;
        let got = kernel.call(|k| k.geometry_hash(shape, DEFAULT_HASH_RESOLUTION));
        let _ = kernel.call(|k| {
            k.release(shape);
            Ok(())
        });
        let got = got.map_err(cmd_err)?;
        let ok = got == want;
        bad += usize::from(!ok);
        let _ = writeln!(
            out,
            "  {name}: stored B-rep hash {got:016x} {}",
            if ok {
                "matches"
            } else {
                "DIFFERS from the stored hash"
            }
        );
    }

    // 2. Regenerating every feature reproduces hash and bytes.
    let ids: Vec<_> = before
        .objects()
        .filter(|o| primitives::is_primitive_kind(&o.kind))
        .map(Object::id)
        .collect();
    doc.transact("Verify: regenerate all", |tx| {
        for id in &ids {
            tx.set_state(*id, ObjectState::OutOfDate, None)?;
        }
        Ok(())
    })
    .map_err(cmd_err)?;
    recompute(&mut doc, registry)?;
    let after = doc.snapshot();
    for o in &solids {
        let name = o.name.as_deref().unwrap_or("?");
        let new = after
            .get(o.id())
            .ok_or_else(|| cmd_err("feature vanished"))?;
        let same_hash = stored_hash(new) == stored_hash(o);
        let same_bytes = new.payload == o.payload;
        bad += usize::from(!(same_hash && same_bytes));
        let _ = writeln!(
            out,
            "  {name}: regenerated hash {} {}{}",
            stored_hash(new).map_or_else(|| "-".to_owned(), |h| format!("{h:016x}")),
            if same_hash { "matches" } else { "DIFFERS" },
            if same_bytes {
                ", B-rep byte-identical"
            } else {
                ", B-rep bytes DIFFER"
            }
        );
    }
    if bad == 0 {
        let _ = writeln!(
            out,
            "verified {}: {} solid(s), all identical",
            path.display(),
            solids.len()
        );
        Ok(out)
    } else {
        Err(CliError::Command(format!("{out}{bad} check(s) failed")))
    }
}
