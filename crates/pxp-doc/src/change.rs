// SPDX-License-Identifier: LGPL-2.1-or-later
//! Change sets and the change-notification bus.

use crate::ObjectId;
use crate::undo::Change;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

/// What produced a change set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeOrigin {
    /// A user transaction committed.
    Commit,
    /// Recompute committed results (or failure flags).
    Recompute,
    /// Undo restored earlier object versions.
    Undo,
    /// Redo re-applied undone versions.
    Redo,
}

/// The objects one commit, undo, redo or recompute created, modified and
/// deleted. Applications receive these (via `on_upstream_change`) to update
/// navigators and to mark their own derived data stale; the renderer uses
/// them to re-tessellate only what changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSet {
    /// Document generation after the change.
    pub generation: u64,
    /// Transaction label (or the undone/redone label).
    pub label: String,
    /// What produced the change.
    pub origin: ChangeOrigin,
    /// Objects that now exist and did not before.
    pub created: BTreeSet<ObjectId>,
    /// Objects that existed before and after, with different contents or
    /// state.
    pub modified: BTreeSet<ObjectId>,
    /// Objects that existed before and no longer do.
    pub deleted: BTreeSet<ObjectId>,
}

impl ChangeSet {
    /// Builds a change set from before/after pairs. With `reverse` (undo),
    /// "after" and "before" swap roles.
    pub(crate) fn from_changes(
        generation: u64,
        label: &str,
        origin: ChangeOrigin,
        changes: &BTreeMap<ObjectId, Change>,
        reverse: bool,
    ) -> Self {
        let mut cs = Self {
            generation,
            label: label.to_owned(),
            origin,
            created: BTreeSet::new(),
            modified: BTreeSet::new(),
            deleted: BTreeSet::new(),
        };
        for (id, c) in changes {
            let (from, to) = if reverse {
                (&c.after, &c.before)
            } else {
                (&c.before, &c.after)
            };
            match (from.is_some(), to.is_some()) {
                (false, true) => cs.created.insert(*id),
                (true, false) => cs.deleted.insert(*id),
                (true, true) => cs.modified.insert(*id),
                (false, false) => false,
            };
        }
        cs
    }

    /// True if nothing changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.modified.is_empty() && self.deleted.is_empty()
    }

    /// True if `id` was created, modified or deleted.
    #[must_use]
    pub fn touches(&self, id: ObjectId) -> bool {
        self.created.contains(&id) || self.modified.contains(&id) || self.deleted.contains(&id)
    }
}

/// Fan-out of change sets to any number of subscribers over channels.
/// Dropped receivers are pruned on the next publish, so subscribers never
/// need to unsubscribe explicitly.
#[derive(Debug, Default)]
pub(crate) struct ChangeBus {
    subscribers: Vec<Sender<Arc<ChangeSet>>>,
}

impl ChangeBus {
    pub(crate) fn subscribe(&mut self) -> Receiver<Arc<ChangeSet>> {
        let (tx, rx) = channel();
        self.subscribers.push(tx);
        rx
    }

    pub(crate) fn publish(&mut self, cs: ChangeSet) {
        let cs = Arc::new(cs);
        self.subscribers.retain(|s| s.send(Arc::clone(&cs)).is_ok());
    }
}
