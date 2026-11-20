// SPDX-License-Identifier: LGPL-2.1-or-later
//! Transactions: the only way to change a document.

use crate::expr::{Expression, FORMULA_PROP, VALUE_PROP};
use crate::state::DocState;
use crate::undo::Change;
use crate::{
    AppId, Dep, DocError, IdSource, NewObject, Object, ObjectId, ObjectKind, ObjectState, Value,
};
use pxp_units::Quantity;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Dependency role used for expression-to-expression references.
pub const EXPR_REF_ROLE: &str = "expr-ref";

/// A transaction in progress. Obtained only inside
/// [`crate::Document::transact`].
///
/// Every mutation is applied to a private working copy of the document. If
/// the closure returns `Err` or panics, the copy is dropped and the document
/// is exactly as it was (full rollback). If it returns `Ok`, the copy
/// becomes the new state in one step (full apply), one undo entry is
/// recorded, and one change set is published.
///
/// Reads through the transaction (`get`, `expression`) see the
/// transaction's own uncommitted edits.
pub struct Tx<'d> {
    pub(crate) work: DocState,
    /// Every object written in this transaction (for the undo diff).
    pub(crate) touched: BTreeSet<ObjectId>,
    /// Objects whose recompute state was decided explicitly in this
    /// transaction; commit will not override it.
    pub(crate) state_set: BTreeSet<ObjectId>,
    /// Objects whose content changed, whose suppression flipped, or that
    /// were deleted: their dependants become out of date at commit.
    pub(crate) invalidating: BTreeSet<ObjectId>,
    ids: &'d mut dyn IdSource,
    label: String,
}

impl<'d> Tx<'d> {
    pub(crate) fn new(label: &str, work: DocState, ids: &'d mut dyn IdSource) -> Self {
        Self {
            work,
            touched: BTreeSet::new(),
            state_set: BTreeSet::new(),
            invalidating: BTreeSet::new(),
            ids,
            label: label.to_owned(),
        }
    }

    /// The transaction label (shown in the undo list).
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Reads an object, including uncommitted edits made in this
    /// transaction.
    #[must_use]
    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.work.get(id).map(AsRef::as_ref)
    }

    /// Reads an expression object by name.
    #[must_use]
    pub fn expression(&self, name: &str) -> Option<&Object> {
        self.work.expr_names.get(name).and_then(|id| self.get(*id))
    }

    /// Creates an object and returns its new ID.
    ///
    /// # Errors
    /// [`DocError::DanglingDependency`], [`DocError::InvalidName`] or
    /// [`DocError::DuplicateExpressionName`] if the object is invalid.
    pub fn create(&mut self, spec: NewObject) -> Result<ObjectId, DocError> {
        let id = self.ids.next_id();
        let mut obj = spec.object;
        obj.id = id;
        self.state_set.insert(id); // the creator chose the initial state
        self.put(id, None, obj)?;
        Ok(id)
    }

    /// Edits an object in place. The closure gets a mutable copy; the edit
    /// is validated and stored when the closure returns. An edit that
    /// changes nothing is a no-op.
    ///
    /// Changing content (properties, dependencies, payload, name, layer,
    /// parent) marks the object and everything downstream out of date at
    /// commit, unless the transaction also sets the object's state
    /// explicitly.
    ///
    /// # Errors
    /// [`DocError::NotFound`], or a validation error (dangling dependency,
    /// cycle, bad expression name).
    pub fn update(&mut self, id: ObjectId, f: impl FnOnce(&mut Object)) -> Result<(), DocError> {
        let old = Arc::clone(self.work.get(id).ok_or(DocError::NotFound(id))?);
        let mut new = (*old).clone();
        f(&mut new);
        new.id = id; // IDs are immutable, whatever the closure did
        if new == *old {
            return Ok(());
        }
        if new.state != old.state {
            self.state_set.insert(id);
        }
        self.put(id, Some(&old), new)
    }

    /// Sets an object's recompute state and diagnostic explicitly.
    ///
    /// # Errors
    /// [`DocError::NotFound`].
    pub fn set_state(
        &mut self,
        id: ObjectId,
        state: ObjectState,
        diagnostic: Option<String>,
    ) -> Result<(), DocError> {
        self.update(id, |o| {
            o.state = state;
            o.diagnostic = diagnostic;
        })?;
        self.state_set.insert(id);
        Ok(())
    }

    /// Deletes one object.
    ///
    /// # Errors
    /// [`DocError::NotFound`], or [`DocError::HasDependants`] if other
    /// objects still depend on it (references are never left dangling).
    pub fn delete(&mut self, id: ObjectId) -> Result<(), DocError> {
        if self.work.get(id).is_none() {
            return Err(DocError::NotFound(id));
        }
        if let Some(deps) = self.work.dependants.get(&id)
            && !deps.is_empty()
        {
            return Err(DocError::HasDependants {
                id,
                dependants: deps.iter().copied().collect(),
            });
        }
        self.work.replace_raw(id, None);
        self.touched.insert(id);
        self.invalidating.insert(id);
        Ok(())
    }

    /// Deletes an object and everything that depends on it, dependants
    /// first. Returns every deleted ID (in deletion order).
    ///
    /// # Errors
    /// [`DocError::NotFound`].
    pub fn delete_with_dependants(&mut self, id: ObjectId) -> Result<Vec<ObjectId>, DocError> {
        if self.work.get(id).is_none() {
            return Err(DocError::NotFound(id));
        }
        let mut doomed = self.work.transitive_dependants([id]);
        doomed.insert(id);
        let mut order = Vec::with_capacity(doomed.len());
        // Repeatedly delete members with no remaining dependants. The graph
        // is acyclic, so each pass deletes at least one member.
        while !doomed.is_empty() {
            let leaves: Vec<ObjectId> = doomed
                .iter()
                .copied()
                .filter(|d| self.work.dependants.get(d).is_none_or(BTreeSet::is_empty))
                .collect();
            if leaves.is_empty() {
                // Unreachable for an acyclic graph; refuse rather than loop.
                let first = doomed.iter().next().copied().unwrap_or(id);
                return Err(DocError::Cycle {
                    from: first,
                    to: first,
                });
            }
            for leaf in leaves {
                self.delete(leaf)?;
                doomed.remove(&leaf);
                order.push(leaf);
            }
        }
        Ok(order)
    }

    /// Creates or redefines the named expression (NX: Tools > Expressions).
    ///
    /// The formula is parsed, every name it references must already exist,
    /// it is evaluated immediately with units checked, and dependencies on
    /// the referenced expressions are recorded, so cycles (`a = b`, `b = a`)
    /// are rejected. Downstream objects become out of date at commit.
    ///
    /// # Errors
    /// [`DocError::InvalidName`], [`DocError::DuplicateExpressionName`],
    /// [`DocError::Expression`] (syntax, unknown name, unit error, non-finite
    /// result) or [`DocError::Cycle`].
    pub fn set_expression(&mut self, name: &str, formula: &str) -> Result<ObjectId, DocError> {
        let wrap = |error| DocError::Expression {
            name: name.to_owned(),
            error,
        };
        if !crate::expr::is_valid_name(name) {
            return Err(DocError::InvalidName(name.to_owned()));
        }
        let parsed = Expression::parse(formula).map_err(wrap)?;
        let value = self.evaluate_parsed(&parsed).map_err(wrap)?;
        let mut deps = BTreeSet::new();
        for r in parsed.references() {
            // evaluate_parsed succeeded, so every reference exists.
            if let Some(&target) = self.work.expr_names.get(&r) {
                deps.insert(Dep::new(target, EXPR_REF_ROLE));
            }
        }
        let props = |o: &mut Object| {
            o.props.insert(
                FORMULA_PROP.to_owned(),
                Value::Text(parsed.source().to_owned()),
            );
            o.props
                .insert(VALUE_PROP.to_owned(), Value::Quantity(value));
            o.deps.clone_from(&deps);
            o.state = ObjectState::UpToDate;
            o.diagnostic = None;
        };
        if let Some(&id) = self.work.expr_names.get(name) {
            self.update(id, props)?;
            self.state_set.insert(id);
            Ok(id)
        } else {
            let mut spec = NewObject::new(ObjectKind::EXPRESSION, AppId::CORE).name(name);
            props(&mut spec.object);
            self.create(spec)
        }
    }

    /// Evaluates a formula against the expressions as they stand in this
    /// transaction, without storing anything. Dialogs use this for preview
    /// (`=width/2` typed into a field).
    ///
    /// # Errors
    /// [`DocError::Expression`].
    pub fn evaluate(&self, formula: &str) -> Result<Quantity, DocError> {
        let wrap = |error| DocError::Expression {
            name: formula.to_owned(),
            error,
        };
        let parsed = Expression::parse(formula).map_err(wrap)?;
        self.evaluate_parsed(&parsed).map_err(wrap)
    }

    /// Evaluation uses each referenced expression's cached value. Values are
    /// kept current because `set_expression` evaluates eagerly and recompute
    /// re-evaluates downstream expressions in dependency order.
    fn evaluate_parsed(&self, parsed: &Expression) -> Result<Quantity, crate::expr::ExprError> {
        parsed.evaluate(&mut |n| {
            self.work
                .expression_value(n)
                .ok_or_else(|| crate::expr::ExprError::UnknownName(n.to_owned()))
        })
    }

    /// Validates and stores `new`, recording what changed.
    fn put(
        &mut self,
        id: ObjectId,
        old: Option<&Arc<Object>>,
        new: Object,
    ) -> Result<(), DocError> {
        self.work.validate(&new)?;
        let content_changed = old.is_none_or(|o| !o.content_eq(&new));
        let suppression_flipped = old.is_some_and(|o| {
            (o.state == ObjectState::Suppressed) != (new.state == ObjectState::Suppressed)
        });
        let deps_changed = old.is_none_or(|o| o.deps != new.deps);
        self.work.replace_raw(id, Some(Arc::new(new)));
        if deps_changed {
            self.work.check_acyclic(id)?;
        }
        self.touched.insert(id);
        if content_changed || suppression_flipped {
            self.invalidating.insert(id);
        }
        Ok(())
    }

    /// Ends the transaction: propagates out-of-date states and computes the
    /// before/after diff against `base`.
    ///
    /// Propagation rule (see `docs/design/document-core.md`):
    /// 1. An object whose content changed becomes out of date, unless its
    ///    state was set explicitly in this transaction (a creator's chosen
    ///    initial state, `set_state`, or an expression `set_expression` just
    ///    evaluated) or it is suppressed.
    /// 2. Every transitive dependant of a changed, deleted or
    ///    (un)suppressed object becomes out of date unless it is suppressed.
    ///    An explicit state does **not** protect against rule 2: a value
    ///    computed earlier in the transaction may be stale if something
    ///    upstream changed afterwards, and a stale value must never be
    ///    labelled up to date.
    pub(crate) fn finish(mut self, base: &DocState) -> (DocState, BTreeMap<ObjectId, Change>) {
        let mut mark: BTreeSet<ObjectId> = self
            .invalidating
            .iter()
            .copied()
            .filter(|id| !self.state_set.contains(id))
            .collect();
        mark.extend(
            self.work
                .transitive_dependants(self.invalidating.iter().copied()),
        );
        for id in mark {
            if let Some(obj) = self.work.get(id)
                && matches!(obj.state, ObjectState::UpToDate | ObjectState::Failed)
            {
                let mut o = (**obj).clone();
                o.state = ObjectState::OutOfDate;
                self.work.replace_raw(id, Some(Arc::new(o)));
                self.touched.insert(id);
            }
        }
        let mut changes = BTreeMap::new();
        for id in self.touched {
            let before = base.get(id).cloned();
            let after = self.work.get(id).cloned();
            let same = match (&before, &after) {
                (None, None) => true,
                (Some(b), Some(a)) => Arc::ptr_eq(b, a) || b == a,
                _ => false,
            };
            if !same {
                changes.insert(id, Change { before, after });
            }
        }
        (self.work, changes)
    }
}
