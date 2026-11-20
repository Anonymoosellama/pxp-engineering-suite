// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-doc: the document core
//!
//! One [`Document`] holds every object of every application in an open part:
//! sketches, features, drawings, CAM operations, FEA studies, scans. The core
//! knows nothing about geometry or user interfaces; it guarantees that every
//! change is transactional, undoable, deterministic and visible to
//! dependants.
//!
//! ## Responsibility
//!
//! - **Object store** with stable UUIDv7 IDs ([`ObjectId`]), namespaced kinds
//!   ([`ObjectKind`]), owning applications ([`AppId`]), 256 layers
//!   ([`Layer`]), typed unit-aware properties ([`Value`]) and opaque
//!   versioned payloads preserved byte for byte.
//! - **Expressions**: NX-style named, unit-aware formulas ([`expr`]).
//! - **Dependency graph** across all applications, with cycle rejection and
//!   automatic out-of-date propagation.
//! - **Transactions** ([`Tx`]): all-or-nothing, panic-safe, the only way to
//!   change anything.
//! - **Undo/redo**: unlimited steps within a memory cap.
//! - **Change bus**: subscribers get a [`ChangeSet`] after every commit.
//! - **Snapshots** ([`Snapshot`]): immutable views for lock-free readers.
//! - **Recompute** ([`Document::recompute`]): parallel, level-by-level,
//!   all-or-nothing.
//!
//! ## Data flow
//!
//! ```text
//! command ──transact(label, |tx| …)──► working copy ──commit──► new state (generation+1)
//!                                          │ Err / panic              ├─► undo entry
//!                                          ▼                          ├─► ChangeSet ─► subscribers
//!                                       dropped (rollback)            └─► dependants marked OutOfDate
//! recompute ──plan levels──► par_map per level on snapshots ──► commit all results (or only failure flags)
//! ```
//!
//! See `docs/design/document-core.md` for the full design note and diagrams.
//!
//! ## Invariants
//!
//! - The document changes only inside [`Document::transact`],
//!   [`Document::undo`], [`Document::redo`] and [`Document::recompute`].
//! - A failed or panicking transaction leaves the state bit-for-bit
//!   unchanged.
//! - Every dependency points at an existing object; the graph is acyclic.
//! - Expression names are unique; expression values are never NaN or
//!   infinite.
//! - An object marked up to date never has an out-of-date input among the
//!   objects recompute manages.
//! - Snapshots never change after they are taken.
//!
//! ## Must never
//!
//! - Depend on geometry, rendering, UI or file-format crates.
//! - Re-bind a reference to a different object silently.
//! - Panic: every failure is a [`DocError`].
#![forbid(unsafe_code)]

mod change;
mod error;
pub mod expr;
mod id;
mod object;
mod recompute;
mod state;
mod txn;
mod undo;

pub use change::{ChangeOrigin, ChangeSet};
pub use error::DocError;
pub use id::{IdParseError, IdSource, ObjectId, SequentialIdSource, UuidV7Source};
pub use object::{AppId, Dep, Layer, NewObject, Object, ObjectKind, ObjectState, Value};
pub use recompute::{
    ExpressionsOnly, RecomputeHandler, RecomputeOutput, RecomputeReport, UpdatePolicy,
};
pub use state::Snapshot;
pub use txn::{EXPR_REF_ROLE, Tx};
pub use undo::DEFAULT_UNDO_CAP_BYTES;

use pxp_compute::CancelToken;
use pxp_units::Quantity;
use state::DocState;
use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use undo::{Change, UndoEntry, UndoStack};

/// An open part: every object of every application, plus history.
///
/// The document has a single writer (whoever holds `&mut Document`, in the
/// application the dedicated document thread) and any number of readers
/// holding [`Snapshot`]s.
///
/// ```
/// use pxp_doc::{Document, ExpressionsOnly};
/// use pxp_compute::CancelToken;
///
/// let mut doc = Document::new();
/// doc.transact("Set width", |tx| tx.set_expression("width", "100mm")).unwrap();
/// doc.transact("Set p1", |tx| tx.set_expression("p1", "width * 2")).unwrap();
/// doc.transact("Edit width", |tx| tx.set_expression("width", "4in")).unwrap();
/// doc.recompute(&ExpressionsOnly, &CancelToken::new()).unwrap();
/// let p1 = doc.snapshot().expression_value("p1").unwrap();
/// assert!((p1.value() - 203.2).abs() < 1e-9); // 2 × 4 in, in mm
/// doc.undo().unwrap(); // the edit and its recompute undo together
/// let p1 = doc.snapshot().expression_value("p1").unwrap();
/// assert!((p1.value() - 200.0).abs() < 1e-9);
/// ```
pub struct Document {
    state: Arc<DocState>,
    ids: Box<dyn IdSource>,
    undo: UndoStack,
    bus: change::ChangeBus,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("generation", &self.state.generation)
            .field("objects", &self.state.objects.len())
            .field("undo_steps", &self.undo.undo_labels().len())
            .finish_non_exhaustive()
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

/// How a commit enters the undo history.
#[derive(Clone, Copy)]
enum UndoMode {
    /// A new undo step.
    Record,
    /// Fold into the last step if nothing happened since; else a new step.
    MergeOrRecord,
}

impl Document {
    /// An empty document with time-ordered UUIDv7 IDs.
    #[must_use]
    pub fn new() -> Self {
        Self::with_id_source(Box::new(UuidV7Source::new()))
    }

    /// An empty document with the given ID source. Use
    /// [`SequentialIdSource`] for deterministic tests and journal replay.
    #[must_use]
    pub fn with_id_source(ids: Box<dyn IdSource>) -> Self {
        Self {
            state: Arc::new(DocState::default()),
            ids,
            undo: UndoStack::default(),
            bus: change::ChangeBus::default(),
        }
    }

    /// An immutable view of the current state. O(1); the snapshot stays
    /// valid and unchanged while the document moves on.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Snapshot(Arc::clone(&self.state))
    }

    /// The current generation (increments on every commit, undo, redo and
    /// recompute commit).
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.state.generation
    }

    /// Subscribes to change sets. Drop the receiver to unsubscribe.
    pub fn subscribe(&mut self) -> Receiver<Arc<ChangeSet>> {
        self.bus.subscribe()
    }

    /// Runs `f` as one transaction labelled `label` (the label appears in the
    /// undo list).
    ///
    /// All-or-nothing: if `f` returns `Ok`, every edit is applied at once,
    /// one undo step is recorded and one [`ChangeSet`] is published; if `f`
    /// returns `Err` or panics, nothing changes. A transaction that changes
    /// nothing records nothing.
    ///
    /// # Errors
    /// Whatever `f` returns, or [`DocError::Panicked`] if it panicked (the
    /// panic is contained here, at the command boundary).
    pub fn transact<T>(
        &mut self,
        label: &str,
        f: impl FnOnce(&mut Tx<'_>) -> Result<T, DocError>,
    ) -> Result<T, DocError> {
        let base = Arc::clone(&self.state);
        let mut tx = Tx::new(label, (*base).clone(), self.ids.as_mut());
        let outcome = catch_unwind(AssertUnwindSafe(|| f(&mut tx)));
        let value = match outcome {
            Ok(Ok(v)) => v,
            Ok(Err(e)) => return Err(e),
            Err(payload) => {
                return Err(DocError::Panicked {
                    label: label.to_owned(),
                    message: pxp_diag::panic_message(payload.as_ref()),
                });
            }
        };
        let (work, changes) = tx.finish(&base);
        self.commit(
            label,
            work,
            &changes,
            ChangeOrigin::Commit,
            UndoMode::Record,
        );
        Ok(value)
    }

    /// Evaluates a formula against the current expressions without changing
    /// anything (dialog preview, CLI `eval`).
    ///
    /// # Errors
    /// [`DocError::Expression`].
    pub fn evaluate(&self, formula: &str) -> Result<Quantity, DocError> {
        let wrap = |error| DocError::Expression {
            name: formula.to_owned(),
            error,
        };
        let parsed = expr::Expression::parse(formula).map_err(wrap)?;
        parsed
            .evaluate(&mut |n| {
                self.state
                    .expression_value(n)
                    .ok_or_else(|| expr::ExprError::UnknownName(n.to_owned()))
            })
            .map_err(wrap)
    }

    /// Undoes the most recent step and returns its label.
    ///
    /// # Errors
    /// [`DocError::NothingToUndo`].
    pub fn undo(&mut self) -> Result<String, DocError> {
        let mut entry = self.undo.pop_done().ok_or(DocError::NothingToUndo)?;
        let mut work = (*self.state).clone();
        for (id, change) in &entry.changes {
            work.replace_raw(*id, change.before.clone());
        }
        work.generation += 1;
        let generation = work.generation;
        self.state = Arc::new(work);
        let cs = ChangeSet::from_changes(
            generation,
            &entry.label,
            ChangeOrigin::Undo,
            &entry.changes,
            true,
        );
        let label = entry.label.clone();
        entry.after_generation = 0; // stale until redone
        self.undo.push_undone(entry);
        self.bus.publish(cs);
        Ok(label)
    }

    /// Redoes the most recently undone step and returns its label.
    ///
    /// # Errors
    /// [`DocError::NothingToRedo`].
    pub fn redo(&mut self) -> Result<String, DocError> {
        let mut entry = self.undo.pop_undone().ok_or(DocError::NothingToRedo)?;
        let mut work = (*self.state).clone();
        for (id, change) in &entry.changes {
            work.replace_raw(*id, change.after.clone());
        }
        work.generation += 1;
        let generation = work.generation;
        self.state = Arc::new(work);
        let cs = ChangeSet::from_changes(
            generation,
            &entry.label,
            ChangeOrigin::Redo,
            &entry.changes,
            false,
        );
        let label = entry.label.clone();
        entry.after_generation = generation;
        self.undo.push_redone(entry);
        self.bus.publish(cs);
        Ok(label)
    }

    /// True if [`Document::undo`] would do something.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.undo.can_undo()
    }

    /// True if [`Document::redo`] would do something.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.undo.can_redo()
    }

    /// Undo step labels, most recent first (the Quick Access undo list).
    #[must_use]
    pub fn undo_labels(&self) -> Vec<String> {
        self.undo.undo_labels()
    }

    /// Redo step labels, next-to-redo first.
    #[must_use]
    pub fn redo_labels(&self) -> Vec<String> {
        self.undo.redo_labels()
    }

    /// Sets the undo memory cap in bytes (default
    /// [`DEFAULT_UNDO_CAP_BYTES`]). The oldest steps are dropped to fit; the
    /// newest step is always kept.
    pub fn set_undo_memory_cap(&mut self, bytes: usize) {
        self.undo.set_cap(bytes);
    }

    /// Estimated bytes held by the undo history.
    #[must_use]
    pub fn undo_memory_used(&self) -> usize {
        self.undo.used_bytes()
    }

    /// Regenerates every out-of-date object whose kind has an immediate
    /// update policy (see [`recompute`] module docs for the algorithm).
    ///
    /// On success all results are committed in one step, merged into the
    /// undo step of the edit that caused them (so one undo reverts both). If
    /// any object fails, **no results** are committed: only the failing
    /// objects are flagged [`ObjectState::Failed`] with their reasons, and
    /// everything downstream stays out of date.
    ///
    /// # Errors
    /// [`DocError::Cancelled`] if `cancel` fires (nothing is committed).
    pub fn recompute(
        &mut self,
        handler: &dyn RecomputeHandler,
        cancel: &CancelToken,
    ) -> Result<RecomputeReport, DocError> {
        let recompute::Plan { levels, blocked } = recompute::plan(&self.state, handler);
        let mut report = RecomputeReport {
            blocked,
            ..RecomputeReport::default()
        };
        if levels.is_empty() {
            return Ok(report);
        }
        let mut work = (*self.state).clone();
        for level in &levels {
            cancel.check().map_err(|_| DocError::Cancelled)?;
            let snap = Snapshot(Arc::new(work.clone()));
            let results = pxp_compute::par_map(level, |&id| {
                let Some(obj) = snap.get(id) else {
                    return Err(format!("object {id} vanished during recompute"));
                };
                catch_unwind(AssertUnwindSafe(|| {
                    if obj.kind == ObjectKind::EXPRESSION {
                        recompute::evaluate_expression(&snap, obj)
                    } else {
                        handler.recompute(&snap, obj)
                    }
                }))
                .unwrap_or_else(|p| {
                    Err(format!("panicked: {}", pxp_diag::panic_message(p.as_ref())))
                })
            })
            .map_err(|e| DocError::Panicked {
                label: "recompute".into(),
                message: e.to_string(),
            })?;
            report.levels += 1;
            let failures: Vec<(ObjectId, String)> = level
                .iter()
                .zip(&results)
                .filter_map(|(id, r)| r.as_ref().err().map(|e| (*id, e.clone())))
                .collect();
            if !failures.is_empty() {
                self.commit_failures(&failures);
                report.failed = failures;
                return Ok(report);
            }
            for (id, result) in level.iter().zip(results) {
                if let (Some(old), Ok(out)) = (work.get(*id), result) {
                    let mut obj = (**old).clone();
                    obj.props.extend(out.props);
                    if let Some(payload) = out.payload {
                        obj.payload = payload;
                    }
                    obj.state = ObjectState::UpToDate;
                    obj.diagnostic = out.warning;
                    work.replace_raw(*id, Some(Arc::new(obj)));
                    report.recomputed.push(*id);
                }
            }
        }
        let changes = diff(&self.state, &work, report.recomputed.iter().copied());
        self.commit(
            "Update",
            work,
            &changes,
            ChangeOrigin::Recompute,
            UndoMode::MergeOrRecord,
        );
        Ok(report)
    }

    /// Records failure flags (and nothing else) after a failed recompute.
    fn commit_failures(&mut self, failures: &[(ObjectId, String)]) {
        let mut work = (*self.state).clone();
        for (id, reason) in failures {
            if let Some(old) = work.get(*id) {
                let mut obj = (**old).clone();
                obj.state = ObjectState::Failed;
                obj.diagnostic = Some(reason.clone());
                work.replace_raw(*id, Some(Arc::new(obj)));
            }
        }
        let changes = diff(&self.state, &work, failures.iter().map(|(id, _)| *id));
        self.commit(
            "Update (failed)",
            work,
            &changes,
            ChangeOrigin::Recompute,
            UndoMode::MergeOrRecord,
        );
    }

    fn commit(
        &mut self,
        label: &str,
        mut work: DocState,
        changes: &BTreeMap<ObjectId, Change>,
        origin: ChangeOrigin,
        mode: UndoMode,
    ) {
        if changes.is_empty() {
            return;
        }
        let previous = self.state.generation;
        work.generation = previous + 1;
        let generation = work.generation;
        self.state = Arc::new(work);
        let merged = matches!(mode, UndoMode::MergeOrRecord)
            && self.undo.merge_into_last(changes, previous, generation);
        if !merged {
            self.undo
                .push(UndoEntry::new(label, changes.clone(), generation));
        }
        self.bus.publish(ChangeSet::from_changes(
            generation, label, origin, changes, false,
        ));
    }
}

/// Before/after pairs for `ids` between two states, skipping no-ops.
fn diff(
    before: &DocState,
    after: &DocState,
    ids: impl Iterator<Item = ObjectId>,
) -> BTreeMap<ObjectId, Change> {
    let mut out = BTreeMap::new();
    for id in ids {
        let (b, a) = (before.get(id).cloned(), after.get(id).cloned());
        let same = match (&b, &a) {
            (None, None) => true,
            (Some(x), Some(y)) => Arc::ptr_eq(x, y) || x == y,
            _ => false,
        };
        if !same {
            out.insert(
                id,
                Change {
                    before: b,
                    after: a,
                },
            );
        }
    }
    out
}
