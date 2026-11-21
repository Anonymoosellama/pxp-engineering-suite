// SPDX-License-Identifier: LGPL-2.1-or-later
//! Document state, its indexes, and read-only snapshots.

use crate::expr::{self, VALUE_PROP};
use crate::{DocError, Object, ObjectId, ObjectKind, ObjectState, Value};
use pxp_units::Quantity;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The complete state of a document at one generation.
///
/// Objects are `Arc`-shared, so cloning a `DocState` copies only pointers:
/// that clone is how a transaction gets a private working copy (rollback =
/// drop it) and how a reader gets an immutable snapshot.
///
/// Cost note: a clone is O(n) pointer copies for n objects. That is cheap at
/// Stage 0 scale (100 000 objects ≈ a millisecond) and keeps the code simple
/// and obviously correct. ADR-0006 plans a persistent map (O(log n) per
/// change) once crates.io is reachable; the public API does not change.
#[derive(Debug, Clone, Default)]
pub(crate) struct DocState {
    pub(crate) objects: BTreeMap<ObjectId, Arc<Object>>,
    /// Reverse dependency index: `dependants[x]` = objects whose `deps`
    /// name `x`. Maintained only by [`DocState::replace_raw`].
    pub(crate) dependants: BTreeMap<ObjectId, BTreeSet<ObjectId>>,
    /// Expression name → expression object. Maintained by `replace_raw`.
    pub(crate) expr_names: BTreeMap<String, ObjectId>,
    /// Incremented on every commit, undo and redo.
    pub(crate) generation: u64,
}

impl DocState {
    pub(crate) fn get(&self, id: ObjectId) -> Option<&Arc<Object>> {
        self.objects.get(&id)
    }

    /// Replaces (or inserts, or removes when `new` is `None`) one object and
    /// keeps both indexes consistent. No validation: callers either
    /// validated first (transactions) or are restoring a state that was
    /// valid as a whole (undo/redo, where objects may be restored in any
    /// order).
    pub(crate) fn replace_raw(
        &mut self,
        id: ObjectId,
        new: Option<Arc<Object>>,
    ) -> Option<Arc<Object>> {
        let old = self.objects.remove(&id);
        if let Some(old) = &old {
            for dep in &old.deps {
                if let Some(set) = self.dependants.get_mut(&dep.on) {
                    set.remove(&id);
                    if set.is_empty() {
                        self.dependants.remove(&dep.on);
                    }
                }
            }
            if old.kind == ObjectKind::EXPRESSION
                && let Some(name) = &old.name
                && self.expr_names.get(name) == Some(&id)
            {
                self.expr_names.remove(name);
            }
        }
        if let Some(new) = new {
            for dep in &new.deps {
                self.dependants.entry(dep.on).or_default().insert(id);
            }
            if new.kind == ObjectKind::EXPRESSION
                && let Some(name) = &new.name
            {
                self.expr_names.insert(name.clone(), id);
            }
            self.objects.insert(id, new);
        }
        old
    }

    /// Checks an object against the document before it is stored:
    /// dependencies exist and are not the object itself; expression names
    /// are valid and unique.
    pub(crate) fn validate(&self, obj: &Object) -> Result<(), DocError> {
        for dep in &obj.deps {
            if dep.on == obj.id {
                return Err(DocError::Cycle {
                    from: obj.id,
                    to: obj.id,
                });
            }
            if !self.objects.contains_key(&dep.on) {
                return Err(DocError::DanglingDependency {
                    from: obj.id,
                    to: dep.on,
                });
            }
        }
        if obj.kind == ObjectKind::EXPRESSION {
            let name = obj.name.as_deref().unwrap_or_default();
            if !expr::is_valid_name(name) {
                return Err(DocError::InvalidName(name.to_owned()));
            }
            if let Some(&other) = self.expr_names.get(name)
                && other != obj.id
            {
                return Err(DocError::DuplicateExpressionName(name.to_owned()));
            }
        }
        Ok(())
    }

    /// Fails if `id` now (transitively) depends on itself.
    ///
    /// Method: `id` depends on each `d` in its deps; a cycle exists iff some
    /// such `d` is downstream of `id`, i.e. in `transitive_dependants(id)`.
    /// Cost: O(size of the downstream subgraph of `id`).
    pub(crate) fn check_acyclic(&self, id: ObjectId) -> Result<(), DocError> {
        let Some(obj) = self.objects.get(&id) else {
            return Ok(());
        };
        let downstream = self.transitive_dependants([id]);
        match obj.deps.iter().find(|d| downstream.contains(&d.on)) {
            Some(dep) => Err(DocError::Cycle {
                from: id,
                to: dep.on,
            }),
            None => Ok(()),
        }
    }

    /// Checks a whole loaded state: every dependency target exists, no
    /// object depends on itself, expression names are valid and unique, and
    /// the graph has no cycle anywhere.
    ///
    /// Method: Kahn's algorithm over the full graph; any object never
    /// reaching in-degree zero lies on (or behind) a cycle. O(V + E).
    pub(crate) fn validate_all(&self) -> Result<(), DocError> {
        let mut names = BTreeMap::new();
        for obj in self.objects.values() {
            for dep in &obj.deps {
                if dep.on == obj.id {
                    return Err(DocError::Cycle {
                        from: obj.id,
                        to: obj.id,
                    });
                }
                if !self.objects.contains_key(&dep.on) {
                    return Err(DocError::DanglingDependency {
                        from: obj.id,
                        to: dep.on,
                    });
                }
            }
            if obj.kind == ObjectKind::EXPRESSION {
                let name = obj.name.as_deref().unwrap_or_default();
                if !expr::is_valid_name(name) {
                    return Err(DocError::InvalidName(name.to_owned()));
                }
                if names.insert(name.to_owned(), obj.id).is_some() {
                    return Err(DocError::DuplicateExpressionName(name.to_owned()));
                }
            }
        }
        let mut indegree: BTreeMap<ObjectId, usize> = self
            .objects
            .values()
            .map(|o| {
                (
                    o.id,
                    o.deps.iter().map(|d| d.on).collect::<BTreeSet<_>>().len(),
                )
            })
            .collect();
        let mut ready: Vec<ObjectId> = indegree
            .iter()
            .filter(|&(_, &n)| n == 0)
            .map(|(&id, _)| id)
            .collect();
        let mut seen = 0_usize;
        while let Some(id) = ready.pop() {
            seen += 1;
            for child in self.dependants.get(&id).into_iter().flatten() {
                if let Some(n) = indegree.get_mut(child) {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        ready.push(*child);
                    }
                }
            }
        }
        if seen == self.objects.len() {
            return Ok(());
        }
        let stuck = indegree.iter().find(|&(_, &n)| n > 0).map(|(&id, _)| id);
        match stuck.and_then(|id| self.objects.get(&id)) {
            Some(obj) => {
                let to = obj.deps.iter().next().map_or(obj.id, |d| d.on);
                Err(DocError::Cycle { from: obj.id, to })
            }
            None => Ok(()),
        }
    }

    /// Every object that depends, directly or transitively, on any of
    /// `roots`. Roots are included only if reachable from another root.
    /// Iterative depth-first search; O(visited nodes + edges).
    pub(crate) fn transitive_dependants(
        &self,
        roots: impl IntoIterator<Item = ObjectId>,
    ) -> BTreeSet<ObjectId> {
        let mut seen = BTreeSet::new();
        let mut stack: Vec<ObjectId> = roots.into_iter().collect();
        while let Some(id) = stack.pop() {
            if let Some(children) = self.dependants.get(&id) {
                for &child in children {
                    if seen.insert(child) {
                        stack.push(child);
                    }
                }
            }
        }
        seen
    }

    /// The cached value of the named expression, if it exists and has one.
    pub(crate) fn expression_value(&self, name: &str) -> Option<Quantity> {
        let id = self.expr_names.get(name)?;
        match self.objects.get(id)?.props.get(VALUE_PROP)? {
            Value::Quantity(q) => Some(*q),
            _ => None,
        }
    }
}

/// An immutable, cheaply clonable view of the document at one generation.
///
/// Readers (UI, renderer, recompute workers, CAM and FEA solvers) work on
/// snapshots and never take a lock that could freeze the UI. A snapshot
/// never changes after it is taken, even while the document commits new
/// transactions.
#[derive(Debug, Clone)]
pub struct Snapshot(pub(crate) Arc<DocState>);

impl Snapshot {
    /// The generation this snapshot was taken at (increments on every
    /// commit, undo and redo).
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.0.generation
    }

    /// The object with this ID.
    #[must_use]
    pub fn get(&self, id: ObjectId) -> Option<&Object> {
        self.0.objects.get(&id).map(AsRef::as_ref)
    }

    /// Number of objects.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.objects.len()
    }

    /// True if the document has no objects.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.objects.is_empty()
    }

    /// All objects, in ID order (deterministic).
    pub fn objects(&self) -> impl Iterator<Item = &Object> {
        self.0.objects.values().map(AsRef::as_ref)
    }

    /// Objects that depend directly on `id` (the "children" in the
    /// Dependencies panel), in ID order.
    pub fn dependants(&self, id: ObjectId) -> impl Iterator<Item = ObjectId> + '_ {
        self.0.dependants.get(&id).into_iter().flatten().copied()
    }

    /// The expression object with this name.
    #[must_use]
    pub fn expression(&self, name: &str) -> Option<&Object> {
        self.0.expr_names.get(name).and_then(|id| self.get(*id))
    }

    /// The cached value of the named expression.
    #[must_use]
    pub fn expression_value(&self, name: &str) -> Option<Quantity> {
        self.0.expression_value(name)
    }

    /// Objects currently out of date (what "Update" would recompute, plus
    /// manual-policy objects such as CAM operations needing regeneration).
    pub fn out_of_date(&self) -> impl Iterator<Item = &Object> {
        self.objects().filter(|o| o.state == ObjectState::OutOfDate)
    }
}
