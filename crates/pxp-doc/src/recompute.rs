// SPDX-License-Identifier: LGPL-2.1-or-later
//! Recompute scheduling: which out-of-date objects to regenerate, in what
//! order, and on how many threads.
//!
//! ## Algorithm
//!
//! 1. **Candidates**: objects that are out of date and whose kind has the
//!    [`UpdatePolicy::Immediate`] policy (expressions always do). Objects
//!    with a manual policy, such as CAM toolpaths, are never regenerated
//!    here: they stay "regenerate needed" until a person asks.
//! 2. **Blocked**: a candidate is blocked if any input outside the
//!    candidate set is not usable (out of date with a manual policy, or
//!    failed). Blocking propagates to its downstream candidates. Blocked
//!    objects stay out of date and are listed in the report, never computed
//!    from stale input.
//! 3. **Levels**: Kahn's topological sort (Kahn 1962, "Topological sorting
//!    of large networks", CACM 5(11)) restricted to the candidates, taken
//!    one *wavefront* at a time. Every object in a level depends only on
//!    objects in earlier levels or already up to date, so a whole level runs
//!    in parallel. Levels are sorted by ID, and the parallel map keeps input
//!    order, so the result is deterministic.
//!
//! Complexity: O(V + E) over the candidate subgraph to plan; execution
//! clones the working state once per level (O(levels × objects) pointer
//! copies, see `DocState`'s cost note).
//!
//! The commit rule is the master prompt's: results from all levels are
//! committed together in one transaction, or, if anything fails, **no
//! results** are committed and only the failure flags are recorded.

use crate::expr::{ExprError, Expression, FORMULA_PROP, VALUE_PROP};
use crate::state::DocState;
use crate::{Object, ObjectId, ObjectKind, ObjectState, Snapshot, Value};
use std::collections::{BTreeMap, BTreeSet};

/// How an object kind reacts to its inputs going out of date (configurable
/// per application in preferences, as in NX).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpdatePolicy {
    /// Recompute as soon as possible (Modeling features, expressions,
    /// Drafting views by default).
    Immediate,
    /// Mark out of date and wait for an explicit request (CAM toolpaths,
    /// CAE solutions, large drawings).
    Manual,
}

/// What a recompute handler produced for one object. Properties are merged
/// into the object's existing properties; the payload, if given, replaces
/// the old one; a warning is shown as a badge (never dropped silently).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecomputeOutput {
    /// Properties to set.
    pub props: BTreeMap<String, Value>,
    /// New payload, or `None` to keep the old one.
    pub payload: Option<Vec<u8>>,
    /// Non-fatal warning (a tolerance relaxed, a tiny edge merged...).
    pub warning: Option<String>,
}

/// Regenerates objects of the kinds an application owns. The shell builds
/// one handler from the application registry; tests supply their own.
///
/// `recompute` runs on worker threads against an immutable snapshot in
/// which all of the object's inputs are up to date. It must be a pure
/// function of that snapshot (determinism rule).
pub trait RecomputeHandler: Sync {
    /// Update policy for objects of `kind`.
    fn policy(&self, kind: &ObjectKind) -> UpdatePolicy;

    /// Computes new results for `object`.
    ///
    /// # Errors
    /// A human-readable reason, shown in the navigator. Panics are caught
    /// and reported the same way.
    fn recompute(&self, inputs: &Snapshot, object: &Object) -> Result<RecomputeOutput, String>;
}

/// A handler that knows no application kinds: only expressions are
/// recomputed and everything else is left for its owner. Used by the CLI
/// and by tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExpressionsOnly;

impl RecomputeHandler for ExpressionsOnly {
    fn policy(&self, _kind: &ObjectKind) -> UpdatePolicy {
        UpdatePolicy::Manual
    }

    fn recompute(&self, _inputs: &Snapshot, object: &Object) -> Result<RecomputeOutput, String> {
        Err(format!("no recompute handler for kind '{}'", object.kind))
    }
}

/// The outcome of [`crate::Document::recompute`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecomputeReport {
    /// Number of parallel levels executed.
    pub levels: usize,
    /// Objects regenerated, in commit order (empty if anything failed).
    pub recomputed: Vec<ObjectId>,
    /// Objects that failed, with reasons.
    pub failed: Vec<(ObjectId, String)>,
    /// Objects left out of date because an input was unusable.
    pub blocked: Vec<ObjectId>,
}

impl RecomputeReport {
    /// True if nothing failed.
    #[must_use]
    pub fn succeeded(&self) -> bool {
        self.failed.is_empty()
    }
}

pub(crate) struct Plan {
    pub(crate) levels: Vec<Vec<ObjectId>>,
    pub(crate) blocked: Vec<ObjectId>,
}

/// Builds the level plan described in the module docs.
pub(crate) fn plan(state: &DocState, handler: &dyn RecomputeHandler) -> Plan {
    let candidates: BTreeSet<ObjectId> = state
        .objects
        .values()
        .filter(|o| {
            o.state == ObjectState::OutOfDate
                && (o.kind == ObjectKind::EXPRESSION
                    || handler.policy(&o.kind) == UpdatePolicy::Immediate)
        })
        .map(|o| o.id)
        .collect();

    let mut indegree: BTreeMap<ObjectId, usize> = BTreeMap::new();
    let mut blocked: BTreeSet<ObjectId> = BTreeSet::new();
    for &c in &candidates {
        let mut n = 0;
        if let Some(obj) = state.get(c) {
            for dep in &obj.deps {
                if candidates.contains(&dep.on) {
                    n += 1;
                } else {
                    let usable = matches!(
                        state.get(dep.on).map(|o| o.state),
                        Some(ObjectState::UpToDate | ObjectState::Suppressed)
                    );
                    if !usable {
                        blocked.insert(c);
                    }
                }
            }
        }
        indegree.insert(c, n);
    }

    let mut levels = Vec::new();
    let mut ready: Vec<ObjectId> = indegree
        .iter()
        .filter(|&(_, &n)| n == 0)
        .map(|(&id, _)| id)
        .collect();
    while !ready.is_empty() {
        let mut level = Vec::new();
        let mut next = BTreeSet::new();
        for id in ready {
            let is_blocked = blocked.contains(&id);
            if !is_blocked {
                level.push(id);
            }
            for child in state.dependants.get(&id).into_iter().flatten() {
                let Some(n) = indegree.get_mut(child) else {
                    continue;
                };
                if is_blocked {
                    blocked.insert(*child);
                }
                *n = n.saturating_sub(1);
                if *n == 0 {
                    next.insert(*child);
                }
            }
        }
        if !level.is_empty() {
            levels.push(level);
        }
        ready = next.into_iter().collect();
    }
    // Unreachable for an acyclic graph, but never silently drop anything.
    for (id, n) in indegree {
        if n > 0 {
            blocked.insert(id);
        }
    }
    Plan {
        levels,
        blocked: blocked.into_iter().collect(),
    }
}

/// Built-in recompute for expression objects: re-evaluate the formula from
/// the (up-to-date) values of the expressions it references.
pub(crate) fn evaluate_expression(
    snap: &Snapshot,
    obj: &Object,
) -> Result<RecomputeOutput, String> {
    let Some(Value::Text(formula)) = obj.props.get(FORMULA_PROP) else {
        return Err("expression has no formula".to_owned());
    };
    let parsed = Expression::parse(formula).map_err(|e| e.to_string())?;
    let value = parsed
        .evaluate(&mut |n| {
            snap.expression_value(n)
                .ok_or_else(|| ExprError::UnknownName(n.to_owned()))
        })
        .map_err(|e| e.to_string())?;
    let mut props = BTreeMap::new();
    props.insert(VALUE_PROP.to_owned(), Value::Quantity(value));
    Ok(RecomputeOutput {
        props,
        payload: None,
        warning: None,
    })
}
