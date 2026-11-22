// SPDX-License-Identifier: LGPL-2.1-or-later
//! An open part as the desktop shell works with it: the file, its document,
//! the write-ahead journal, autosave, and the display meshes of its solids.
//!
//! ## Life of a session
//!
//! ```text
//! create(path, units) ─┐                     ┌─► save() ─► (journal reset, autosave removed)
//! open(path) ──────────┴─► PartSession ──────┼─► add_block(...) ─► transact ─► recompute ─► journal.record
//!   └─ OpenNotes: recovery? autosave?        ├─► undo()/redo()  ─► recompute ─► journal.record
//!                                            ├─► tick(now) ─► Autosaver writes <part>.pxp.autosave when due
//!                                            └─► close() (clean: journal and autosave removed)
//! ```
//!
//! Every change goes through [`PartSession::commit`], which recomputes and
//! records every published change set in the part's recovery journal
//! before returning, so a crash right after a command loses nothing
//! (pxp-format's write-ahead rule).
//!
//! ## Must never
//!
//! - Keep geometry anywhere but the document: display meshes are derived,
//!   rebuilt from the stored B-rep, and never saved.

use crate::primitives::{self, BLOCK, CONE, CYLINDER, DATUM_CSYS, KERNEL, SPHERE};
use pxp_appkit::AppRegistry;
use pxp_compute::CancelToken;
use pxp_doc::{ChangeSet, DocError, Document, Object, ObjectId, ObjectKind, ObjectState, Value};
use pxp_format::{Autosaver, FormatError, Opened, PartFile, PartMeta, RecoverySummary};
use pxp_kernel::{FacetMesh, KernelHost, MeshTolerance, decode_stored_shape};
use pxp_math::{Point3, Vec3};
use pxp_units::{Length, UnitSystem};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// Why a session operation failed, in words for the status line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionError(pub String);

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SessionError {}

impl From<FormatError> for SessionError {
    fn from(e: FormatError) -> Self {
        Self(e.to_string())
    }
}

impl From<DocError> for SessionError {
    fn from(e: DocError) -> Self {
        Self(e.to_string())
    }
}

/// What opening a part found besides the saved document.
#[derive(Debug, Clone, Default)]
pub struct OpenNotes {
    /// Unsaved transactions in the recovery journal (offer to restore).
    pub recovery: Option<RecoverySummary>,
    /// An autosave made on top of this save (offer when there is no
    /// journal to recover from).
    pub autosave: Option<PathBuf>,
    /// The file is open read-only, and why (locked by another session).
    pub read_only: Option<String>,
}

/// One feature row of the Part Navigator's Model History.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    /// The feature.
    pub id: ObjectId,
    /// Its name, as shown.
    pub name: String,
    /// Feature type ("Block", "Datum Coordinate System", …).
    pub kind: &'static str,
    /// Regeneration state.
    pub state: ObjectState,
    /// Why it failed, if it did.
    pub error: Option<String>,
}

/// The primitives a dialog can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primitive {
    /// Block: origin and three edge lengths.
    Block,
    /// Cylinder: base point, axis, diameter, height.
    Cylinder,
    /// Cone: base point, axis, base and top diameters, height.
    Cone,
    /// Sphere: centre and diameter.
    Sphere,
}

impl Primitive {
    /// Feature type name.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Block => "Block",
            Self::Cylinder => "Cylinder",
            Self::Cone => "Cone",
            Self::Sphere => "Sphere",
        }
    }

    /// The dialog fields in order: (label, is a length). Labels follow NX's
    /// dialogs (XC/YC/ZC are work coordinate system axes).
    #[must_use]
    pub const fn fields(self) -> &'static [(&'static str, bool)] {
        match self {
            Self::Block => &[
                ("Origin XC", true),
                ("Origin YC", true),
                ("Origin ZC", true),
                ("Length (XC)", true),
                ("Width (YC)", true),
                ("Height (ZC)", true),
            ],
            Self::Cylinder => &[
                ("Base XC", true),
                ("Base YC", true),
                ("Base ZC", true),
                ("Axis I", false),
                ("Axis J", false),
                ("Axis K", false),
                ("Diameter", true),
                ("Height", true),
            ],
            Self::Cone => &[
                ("Base XC", true),
                ("Base YC", true),
                ("Base ZC", true),
                ("Axis I", false),
                ("Axis J", false),
                ("Axis K", false),
                ("Base Diameter", true),
                ("Top Diameter", true),
                ("Height", true),
            ],
            Self::Sphere => &[
                ("Center XC", true),
                ("Center YC", true),
                ("Center ZC", true),
                ("Diameter", true),
            ],
        }
    }

    /// Default dialog values (millimetre parts; the shell scales them for
    /// inch parts).
    #[must_use]
    pub const fn defaults(self) -> &'static [&'static str] {
        match self {
            Self::Block => &["0", "0", "0", "100", "100", "100"],
            Self::Cylinder => &["0", "0", "0", "0", "0", "1", "50", "100"],
            Self::Cone => &["0", "0", "0", "0", "0", "1", "50", "0", "25"],
            Self::Sphere => &["0", "0", "0", "50"],
        }
    }
}

/// A solid's display mesh, ready for the viewport.
#[derive(Debug, Clone)]
pub struct DisplayBody {
    /// The feature that made it.
    pub feature: ObjectId,
    /// The feature's name.
    pub name: String,
    /// The mesh.
    pub mesh: FacetMesh,
}

/// An open part (see the module docs).
pub struct PartSession {
    file: PartFile,
    doc: Document,
    changes: Receiver<Arc<ChangeSet>>,
    autosaver: Autosaver,
    /// Generation at the last save (for the "modified" marker).
    saved_generation: u64,
}

impl std::fmt::Debug for PartSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PartSession")
            .field("path", &self.file.path())
            .field("generation", &self.doc.generation())
            .finish_non_exhaustive()
    }
}

/// Autosave interval used when the customer defaults give none: 5 minutes,
/// NX's default.
pub const DEFAULT_AUTOSAVE: Duration = Duration::from_mins(5);

/// A bare number typed in a length field means the part's unit.
#[must_use]
pub fn length_formula(v: &str, units: UnitSystem) -> String {
    if v.trim().parse::<f64>().is_ok() {
        let unit = match units {
            UnitSystem::Millimetre => "mm",
            UnitSystem::Inch => "in",
        };
        format!("{}{unit}", v.trim())
    } else {
        v.trim().to_owned()
    }
}

/// Display name of a Stage 0 feature kind.
#[must_use]
pub fn kind_label(kind: &ObjectKind) -> &'static str {
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

impl PartSession {
    /// Creates a new part from the template (a datum CSYS at the absolute
    /// origin), mm or inch, and opens it.
    ///
    /// # Errors
    /// File or document errors.
    pub fn create(
        path: &Path,
        units: UnitSystem,
        registry: &AppRegistry,
    ) -> Result<Self, SessionError> {
        let meta = match units {
            UnitSystem::Millimetre => PartMeta::millimetre(),
            UnitSystem::Inch => PartMeta::inch(),
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
        })?;
        doc.recompute(registry, &CancelToken::new())?;
        // The template is the starting point, not an action to undo.
        doc.clear_undo();
        let file = PartFile::create(path, meta, &doc)?;
        Ok(Self::wrap(file, doc))
    }

    /// Opens a part. Unsaved work found beside it is reported in
    /// [`OpenNotes`], not applied: the shell asks the user first.
    ///
    /// # Errors
    /// File errors (missing, not a part, newer schema).
    pub fn open(path: &Path) -> Result<(Self, OpenNotes), SessionError> {
        let Opened {
            file,
            document,
            recovery,
            autosave,
            ..
        } = PartFile::open(path)?;
        let notes = OpenNotes {
            recovery,
            autosave,
            read_only: file.read_only_reason().map(str::to_owned),
        };
        Ok((Self::wrap(file, document), notes))
    }

    fn wrap(file: PartFile, mut doc: Document) -> Self {
        let changes = doc.subscribe();
        let generation = doc.generation();
        Self {
            file,
            doc,
            changes,
            autosaver: Autosaver::new(DEFAULT_AUTOSAVE, generation, Instant::now()),
            saved_generation: generation,
        }
    }

    /// Replaces the document with the journal's unsaved work.
    ///
    /// # Errors
    /// File errors reading the journal.
    pub fn restore_recovery(&mut self) -> Result<(), SessionError> {
        let doc = self.file.recovered_document()?;
        self.replace_document(doc);
        Ok(())
    }

    /// Replaces the document with the autosaved copy.
    ///
    /// # Errors
    /// File errors reading the autosave.
    pub fn restore_autosave(&mut self) -> Result<(), SessionError> {
        let doc = self.file.autosaved_document()?;
        self.replace_document(doc);
        Ok(())
    }

    /// Throws away unsaved work found at open (journal and autosave).
    ///
    /// # Errors
    /// File errors.
    pub fn discard_unsaved(&mut self) -> Result<(), SessionError> {
        self.file.discard_recovery()?;
        self.file.discard_autosave()?;
        Ok(())
    }

    fn replace_document(&mut self, mut doc: Document) {
        self.changes = doc.subscribe();
        self.doc = doc;
        // Restored work is unsaved: mark it modified.
        self.saved_generation = u64::MAX;
    }

    /// Sets how often autosave runs (customer default).
    pub fn set_autosave_interval(&mut self, interval: Duration) {
        self.autosaver = Autosaver::new(
            interval,
            self.saved_generation.min(self.doc.generation()),
            Instant::now(),
        );
    }

    /// The part's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.file.path()
    }

    /// The part's units and tolerances.
    #[must_use]
    pub fn meta(&self) -> PartMeta {
        self.file.meta()
    }

    /// The document (read-only; change it through the session).
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Unsaved changes exist.
    #[must_use]
    pub fn is_modified(&self) -> bool {
        self.doc.generation() != self.saved_generation
    }

    /// Runs a transaction, recomputes, and journals the result. A failed
    /// recompute is not an error here: the feature shows as failed in the
    /// Part Navigator, as in NX, and its reason is returned.
    ///
    /// # Errors
    /// The transaction's error (nothing changed), or a journal write error.
    pub fn commit<T>(
        &mut self,
        registry: &AppRegistry,
        label: &str,
        f: impl FnOnce(&mut pxp_doc::Tx<'_>) -> Result<T, DocError>,
    ) -> Result<(T, Vec<(ObjectId, String)>), SessionError> {
        let value = self.doc.transact(label, f)?;
        let failed = self.recompute(registry)?;
        Ok((value, failed))
    }

    /// Recomputes what is out of date and journals every change.
    ///
    /// # Errors
    /// Document or journal errors.
    pub fn recompute(
        &mut self,
        registry: &AppRegistry,
    ) -> Result<Vec<(ObjectId, String)>, SessionError> {
        let report = self.doc.recompute(registry, &CancelToken::new())?;
        self.journal()?;
        Ok(report.failed)
    }

    fn journal(&mut self) -> Result<(), SessionError> {
        let snap = self.doc.snapshot();
        while let Ok(change) = self.changes.try_recv() {
            self.file.record(&snap, &change)?;
        }
        Ok(())
    }

    /// Undo; returns the undone step's label.
    ///
    /// # Errors
    /// Nothing to undo, or journal errors.
    pub fn undo(&mut self, registry: &AppRegistry) -> Result<String, SessionError> {
        let label = self.doc.undo()?;
        self.recompute(registry)?;
        Ok(label)
    }

    /// Redo; returns the redone step's label.
    ///
    /// # Errors
    /// Nothing to redo, or journal errors.
    pub fn redo(&mut self, registry: &AppRegistry) -> Result<String, SessionError> {
        let label = self.doc.redo()?;
        self.recompute(registry)?;
        Ok(label)
    }

    /// The Block dialog's "Origin and Edge Lengths": origin and lengths are
    /// formulas (a bare number means the part's unit). Returns the new
    /// feature and any regeneration failure.
    ///
    /// # Errors
    /// A bad formula, unit error, or journal error (then nothing changed).
    pub fn add_block(
        &mut self,
        registry: &AppRegistry,
        name: Option<&str>,
        origin: [&str; 3],
        lengths: [&str; 3],
    ) -> Result<(ObjectId, Option<String>), SessionError> {
        let units = self.meta().units;
        // NX numbers features by their place in the history (the datum CSYS
        // is 0), so the next feature's number is the current count.
        let n = self
            .doc
            .snapshot()
            .objects()
            .filter(|o| primitives::is_primitive_kind(&o.kind))
            .count();
        let name = name.map_or_else(|| format!("Block({n})"), str::to_owned);
        let origin = origin.map(|v| length_formula(v, units));
        let lengths = lengths.map(|v| length_formula(v, units));
        let o: [&str; 3] = [&origin[0], &origin[1], &origin[2]];
        let l: [&str; 3] = [&lengths[0], &lengths[1], &lengths[2]];
        let (id, failed) = self.commit(registry, &format!("Block {name}"), |tx| {
            primitives::create_block_at(tx, &name, o, l, None)
        })?;
        let why = failed.into_iter().find(|(f, _)| *f == id).map(|(_, w)| w);
        Ok((id, why))
    }

    /// Creates a cylinder, cone or sphere from dialog values (formulas; a
    /// bare number in a length field means the part's unit, axis
    /// components are plain numbers). `values` follow [`Primitive::fields`].
    ///
    /// # Errors
    /// A bad formula, wrong number of values, or a journal error.
    pub fn add_primitive(
        &mut self,
        registry: &AppRegistry,
        kind: Primitive,
        name: Option<&str>,
        values: &[String],
    ) -> Result<(ObjectId, Option<String>), SessionError> {
        let fields = kind.fields();
        if values.len() != fields.len() {
            return Err(SessionError(format!(
                "{} needs {} values, got {}",
                kind.label(),
                fields.len(),
                values.len()
            )));
        }
        let units = self.meta().units;
        // Lengths get the part unit; plain numbers (axis components) stay.
        let v: Vec<String> = values
            .iter()
            .zip(fields)
            .map(|(v, (_, is_length))| {
                if *is_length {
                    length_formula(v, units)
                } else {
                    v.trim().to_owned()
                }
            })
            .collect();
        // NX numbers features by their place in the history (the datum CSYS
        // is 0), so the next feature's number is the current count.
        let n = self
            .doc
            .snapshot()
            .objects()
            .filter(|o| primitives::is_primitive_kind(&o.kind))
            .count();
        let name = name.map_or_else(|| format!("{}({n})", kind.label()), str::to_owned);
        let a = |i: usize| v.get(i).map_or("", String::as_str);
        let label = format!("{} {name}", kind.label());
        let (id, failed) = match kind {
            Primitive::Block => self.commit(registry, &label, |tx| {
                primitives::create_block_at(tx, &name, [a(0), a(1), a(2)], [a(3), a(4), a(5)], None)
            })?,
            Primitive::Cylinder => self.commit(registry, &label, |tx| {
                primitives::create_cylinder(
                    tx,
                    &name,
                    [a(0), a(1), a(2)],
                    [a(3), a(4), a(5)],
                    a(6),
                    a(7),
                )
            })?,
            Primitive::Cone => self.commit(registry, &label, |tx| {
                primitives::create_cone(
                    tx,
                    &name,
                    [a(0), a(1), a(2)],
                    [a(3), a(4), a(5)],
                    [a(6), a(7)],
                    a(8),
                )
            })?,
            Primitive::Sphere => self.commit(registry, &label, |tx| {
                primitives::create_sphere(tx, &name, [a(0), a(1), a(2)], a(3))
            })?,
        };
        let why = failed.into_iter().find(|(f, _)| *f == id).map(|(_, w)| w);
        Ok((id, why))
    }

    /// Creates a datum CSYS at `origin` (mm) aligned with the absolute axes
    /// (NX's Datum CSYS at the WCS, Stage 0 having only the absolute WCS
    /// orientation).
    ///
    /// # Errors
    /// Document or journal errors.
    pub fn add_datum_csys(
        &mut self,
        registry: &AppRegistry,
        origin: [f64; 3],
    ) -> Result<ObjectId, SessionError> {
        let n = self
            .doc
            .snapshot()
            .objects()
            .filter(|o| o.kind == DATUM_CSYS)
            .count();
        let name = format!("Datum Coordinate System({n})");
        let (id, _) = self.commit(registry, &format!("Datum CSYS {name}"), |tx| {
            primitives::create_datum_csys(
                tx,
                &name,
                Point3::new(origin[0], origin[1], origin[2]),
                Vec3::X,
                Vec3::Y,
            )
        })?;
        Ok(id)
    }

    /// Origins and unit axes of every datum CSYS, for display.
    #[must_use]
    pub fn datum_frames(&self) -> Vec<([f64; 3], [[f64; 3]; 3])> {
        let snap = self.doc.snapshot();
        snap.objects()
            .filter(|o| o.kind == DATUM_CSYS)
            .filter_map(|o| {
                let len = |k: &str| match o.prop(k) {
                    Some(Value::Quantity(q)) => Some(q.value()),
                    _ => None,
                };
                let o3 = [len("origin_x")?, len("origin_y")?, len("origin_z")?];
                let x = [len("x_axis_x")?, len("x_axis_y")?, len("x_axis_z")?];
                let y = [len("y_axis_x")?, len("y_axis_y")?, len("y_axis_z")?];
                let c = pxp_math::Csys::new(
                    Point3::new(o3[0], o3[1], o3[2]),
                    Vec3::new(x[0], x[1], x[2]),
                    Vec3::new(y[0], y[1], y[2]),
                )
                .ok()?;
                let v = |u: pxp_math::UnitVec3| {
                    let g = u.get();
                    [g.x, g.y, g.z]
                };
                Some((o3, [v(c.x_axis()), v(c.y_axis()), v(c.z_axis())]))
            })
            .collect()
    }

    /// Mass properties (unit density) of a solid feature's stored B-rep.
    ///
    /// # Errors
    /// The feature has no geometry from this kernel, or the kernel failed.
    pub fn mass_properties(
        &self,
        feature: ObjectId,
        kernel: &KernelHost,
    ) -> Result<pxp_kernel::MassProperties, SessionError> {
        let snap = self.doc.snapshot();
        let o = snap
            .get(feature)
            .ok_or_else(|| SessionError(format!("no feature {feature}")))?;
        let stored = decode_stored_shape(&o.payload).map_err(|e| SessionError(e.to_string()))?;
        if stored.backend != kernel.backend_id().0 {
            return Err(SessionError(format!(
                "computed by {}; recompute with {} first",
                stored.backend,
                kernel.backend_id().0
            )));
        }
        let shape = kernel
            .call(|k| k.import_native(stored.native))
            .map_err(|e| SessionError(e.to_string()))?;
        let m = kernel
            .call(|k| k.mass_properties(shape))
            .map_err(|e| SessionError(e.to_string()));
        let _ = kernel.call(|k| {
            k.release(shape);
            Ok(())
        });
        m
    }

    /// Model History rows, in creation order.
    #[must_use]
    pub fn history(&self) -> Vec<HistoryRow> {
        let snap = self.doc.snapshot();
        let mut rows: Vec<&Object> = snap
            .objects()
            .filter(|o| o.kind == DATUM_CSYS || primitives::is_primitive_kind(&o.kind))
            .collect();
        rows.sort_by_key(|o| o.id());
        rows.into_iter()
            .map(|o| HistoryRow {
                id: o.id(),
                name: o
                    .name
                    .clone()
                    .unwrap_or_else(|| kind_label(&o.kind).to_owned()),
                kind: kind_label(&o.kind),
                state: o.state,
                error: o.diagnostic.clone(),
            })
            .collect()
    }

    /// Saves. Clears the journal and any autosave.
    ///
    /// # Errors
    /// File errors (read-only, disk full: the file on disk is untouched).
    pub fn save(&mut self) -> Result<(), SessionError> {
        let meta = self.meta();
        self.file.save(&self.doc, meta)?;
        self.saved_generation = self.doc.generation();
        self.autosaver.saved(self.saved_generation, Instant::now());
        Ok(())
    }

    /// Autosaves when due (call from the idle loop). Returns the path
    /// written, if any.
    ///
    /// # Errors
    /// File errors writing the autosave.
    pub fn tick(&mut self, now: Instant) -> Result<Option<PathBuf>, SessionError> {
        if self.file.read_only_reason().is_some() {
            return Ok(None);
        }
        Ok(self.autosaver.tick(&self.file, &self.doc, now)?)
    }

    /// Closes the part. With unsaved work the journal stays, so the next
    /// open offers it back.
    ///
    /// # Errors
    /// File errors.
    pub fn close(self) -> Result<(), SessionError> {
        Ok(self.file.close()?)
    }

    /// Display meshes of every up-to-date solid, rebuilt from the stored
    /// B-rep through `kernel`. Solids computed by another backend are
    /// skipped and named in the second list (they display once recomputed
    /// with this one).
    #[must_use]
    pub fn display_bodies(&self, kernel: &KernelHost) -> (Vec<DisplayBody>, Vec<String>) {
        let snap = self.doc.snapshot();
        let mut bodies = Vec::new();
        let mut problems = Vec::new();
        let mut solids: Vec<&Object> = snap
            .objects()
            .filter(|o| primitives::is_solid_kind(&o.kind) && o.state == ObjectState::UpToDate)
            .collect();
        solids.sort_by_key(|o| o.id());
        for o in solids {
            let name = o
                .name
                .clone()
                .unwrap_or_else(|| kind_label(&o.kind).to_owned());
            match Self::mesh_of(o, kernel) {
                Ok(mesh) => bodies.push(DisplayBody {
                    feature: o.id(),
                    name,
                    mesh,
                }),
                Err(why) => problems.push(format!("{name}: {why}")),
            }
        }
        (bodies, problems)
    }

    fn mesh_of(o: &Object, kernel: &KernelHost) -> Result<FacetMesh, String> {
        if o.payload.is_empty() {
            return Err("no stored geometry".into());
        }
        let stored = decode_stored_shape(&o.payload).map_err(|e| e.to_string())?;
        if stored.backend != kernel.backend_id().0 {
            let by = match o.prop(KERNEL) {
                Some(Value::Text(t)) => t.clone(),
                _ => stored.backend.to_owned(),
            };
            return Err(format!(
                "computed by {by}; recompute to display with {}",
                kernel.backend_id().0
            ));
        }
        let shape = kernel
            .call(|k| k.import_native(stored.native))
            .map_err(|e| e.to_string())?;
        let mesh = kernel
            .call(|k| {
                let b = k.bounding_box(shape)?;
                let size = match (b.min(), b.max()) {
                    (Some(lo), Some(hi)) => (hi - lo).length(),
                    _ => 100.0,
                };
                k.tessellate(shape, &MeshTolerance::display(Length::from_mm(size)))
            })
            .map_err(|e| e.to_string());
        let _ = kernel.call(|k| {
            k.release(shape);
            Ok(())
        });
        mesh
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::Modeling;

    fn registry() -> AppRegistry {
        let mut r = AppRegistry::new();
        r.register(Box::new(Modeling::new())).unwrap();
        r
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pxp-session-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("part.pxp")
    }

    #[test]
    fn bare_numbers_take_the_part_unit() {
        assert_eq!(length_formula(" 25 ", UnitSystem::Inch), "25in");
        assert_eq!(length_formula("p0 * 2", UnitSystem::Millimetre), "p0 * 2");
    }

    #[test]
    fn a_block_without_a_kernel_fails_visibly_and_the_journal_keeps_it() {
        let path = scratch("journal");
        let reg = registry();
        let mut s = PartSession::create(&path, UnitSystem::Millimetre, &reg).unwrap();
        assert!(!s.is_modified());
        let (id, why) = s
            .add_block(&reg, None, ["0", "0", "0"], ["20", "30", "10"])
            .unwrap();
        assert!(
            why.unwrap().contains("no geometry kernel"),
            "the reason is shown, not hidden"
        );
        assert!(s.is_modified());
        let rows = s.history();
        assert_eq!(
            rows.iter().map(|r| r.kind).collect::<Vec<_>>(),
            ["Datum Coordinate System", "Block"]
        );
        assert_eq!(rows[1].id, id);
        assert_eq!(
            rows[1].name, "Block(1)",
            "NX numbering: Datum CSYS(0), Block(1)"
        );
        assert_eq!(rows[1].state, ObjectState::Failed);
        // A crash now: the journal holds the unsaved block.
        drop(s);
        let (mut s, notes) = PartSession::open(&path).unwrap();
        assert_eq!(
            notes.recovery.as_ref().map(|r| r.transactions > 0),
            Some(true)
        );
        assert_eq!(s.history().len(), 1, "the saved state comes first");
        s.restore_recovery().unwrap();
        assert_eq!(s.history().len(), 2);
        s.save().unwrap();
        s.close().unwrap();
        let (s, notes) = PartSession::open(&path).unwrap();
        assert!(notes.recovery.is_none() && notes.autosave.is_none());
        assert_eq!(s.history().len(), 2);
    }

    #[test]
    fn a_hundred_undo_and_redo_steps_restore_every_state() {
        let path = scratch("hundred");
        let reg = registry();
        let mut s = PartSession::create(&path, UnitSystem::Millimetre, &reg).unwrap();
        for i in 0..100 {
            let len = format!("{}", i + 1);
            s.add_block(&reg, None, ["0", "0", "0"], [&len, "1", "1"])
                .unwrap();
        }
        let full: Vec<String> = s.history().into_iter().map(|r| r.name).collect();
        assert_eq!(full.len(), 101);
        for _ in 0..100 {
            s.undo(&reg).unwrap();
        }
        assert_eq!(s.history().len(), 1, "back to the template");
        assert!(s.undo(&reg).is_err(), "nothing more to undo");
        for _ in 0..100 {
            s.redo(&reg).unwrap();
        }
        let again: Vec<String> = s.history().into_iter().map(|r| r.name).collect();
        assert_eq!(again, full, "redo restores every feature, in order");
        // And the journal saw all of it: a crash now recovers the full part.
        drop(s);
        let (mut s, notes) = PartSession::open(&path).unwrap();
        assert!(notes.recovery.is_some());
        s.restore_recovery().unwrap();
        assert_eq!(s.history().len(), 101);
    }

    #[test]
    fn undo_and_redo_go_through_the_journal() {
        let path = scratch("undo");
        let reg = registry();
        let mut s = PartSession::create(&path, UnitSystem::Inch, &reg).unwrap();
        s.add_block(&reg, Some("B"), ["1", "1", "1"], ["2", "2", "2"])
            .unwrap();
        assert_eq!(s.undo(&reg).unwrap(), "Block B");
        assert_eq!(s.history().len(), 1);
        assert_eq!(s.redo(&reg).unwrap(), "Block B");
        assert_eq!(s.history()[1].name, "B");
    }
}
