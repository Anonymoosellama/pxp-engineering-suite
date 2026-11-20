// SPDX-License-Identifier: LGPL-2.1-or-later
//! Undo and redo history with a memory cap.
//!
//! An entry stores, for every object a transaction touched, the shared
//! (`Arc`) object before and after. Undo writes the "before" versions back;
//! redo writes the "after" versions. Because objects are shared, an entry
//! costs memory proportional to what the transaction changed, not to the
//! model size.
//!
//! The history is unlimited in count and limited in (estimated) bytes, as
//! the master prompt asks: when the cap is exceeded, the oldest entries are
//! dropped, but the most recent entry is always kept.

use crate::{Object, ObjectId};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

/// One object's before/after versions (`None` = did not exist).
#[derive(Debug, Clone)]
pub(crate) struct Change {
    pub(crate) before: Option<Arc<Object>>,
    pub(crate) after: Option<Arc<Object>>,
}

/// One undoable step.
#[derive(Debug, Clone)]
pub(crate) struct UndoEntry {
    pub(crate) label: String,
    pub(crate) changes: BTreeMap<ObjectId, Change>,
    /// Document generation right after this entry was (re)applied. Used to
    /// decide whether a recompute may merge into this entry.
    pub(crate) after_generation: u64,
    bytes: usize,
}

impl UndoEntry {
    pub(crate) fn new(
        label: &str,
        changes: BTreeMap<ObjectId, Change>,
        after_generation: u64,
    ) -> Self {
        let mut e = Self {
            label: label.to_owned(),
            changes,
            after_generation,
            bytes: 0,
        };
        e.bytes = e.estimate();
        e
    }

    fn estimate(&self) -> usize {
        self.changes
            .values()
            .map(|c| {
                64 + c.before.as_ref().map_or(0, |o| o.approx_size())
                    + c.after.as_ref().map_or(0, |o| o.approx_size())
            })
            .sum()
    }
}

/// Default memory cap for the undo history: 256 MiB.
pub const DEFAULT_UNDO_CAP_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct UndoStack {
    done: VecDeque<UndoEntry>,
    undone: Vec<UndoEntry>,
    cap: usize,
    used: usize,
}

impl Default for UndoStack {
    fn default() -> Self {
        Self {
            done: VecDeque::new(),
            undone: Vec::new(),
            cap: DEFAULT_UNDO_CAP_BYTES,
            used: 0,
        }
    }
}

impl UndoStack {
    /// Records a new step. A new step invalidates the redo history.
    pub(crate) fn push(&mut self, entry: UndoEntry) {
        self.undone.clear();
        self.used = self.done.iter().map(|e| e.bytes).sum::<usize>() + entry.bytes;
        self.done.push_back(entry);
        self.enforce_cap();
    }

    /// Folds `changes` into the most recent entry if the document has not
    /// changed since it was applied (`current_generation` matches). Returns
    /// false if no merge happened. Used so that a recompute and the edit
    /// that triggered it undo as one step.
    pub(crate) fn merge_into_last(
        &mut self,
        changes: &BTreeMap<ObjectId, Change>,
        current_generation: u64,
        new_generation: u64,
    ) -> bool {
        let Some(last) = self.done.back_mut() else {
            return false;
        };
        if last.after_generation != current_generation || !self.undone.is_empty() {
            return false;
        }
        for (id, change) in changes {
            match last.changes.get_mut(id) {
                Some(existing) => existing.after.clone_from(&change.after),
                None => {
                    last.changes.insert(*id, change.clone());
                }
            }
        }
        // Drop entries whose net effect is nothing.
        last.changes.retain(|_, c| match (&c.before, &c.after) {
            (None, None) => false,
            (Some(b), Some(a)) => !(Arc::ptr_eq(b, a) || b == a),
            _ => true,
        });
        last.after_generation = new_generation;
        last.bytes = last.estimate();
        self.used = self.done.iter().map(|e| e.bytes).sum();
        self.enforce_cap();
        true
    }

    pub(crate) fn pop_done(&mut self) -> Option<UndoEntry> {
        let e = self.done.pop_back()?;
        self.used = self.used.saturating_sub(e.bytes);
        Some(e)
    }

    pub(crate) fn push_undone(&mut self, e: UndoEntry) {
        self.undone.push(e);
    }

    pub(crate) fn pop_undone(&mut self) -> Option<UndoEntry> {
        self.undone.pop()
    }

    /// Re-records a redone step without clearing the rest of the redo stack.
    pub(crate) fn push_redone(&mut self, e: UndoEntry) {
        self.used += e.bytes;
        self.done.push_back(e);
        self.enforce_cap();
    }

    pub(crate) fn set_cap(&mut self, bytes: usize) {
        self.cap = bytes;
        self.enforce_cap();
    }

    pub(crate) fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub(crate) fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub(crate) fn undo_labels(&self) -> Vec<String> {
        self.done.iter().rev().map(|e| e.label.clone()).collect()
    }

    pub(crate) fn redo_labels(&self) -> Vec<String> {
        self.undone.iter().rev().map(|e| e.label.clone()).collect()
    }

    pub(crate) fn used_bytes(&self) -> usize {
        self.used
    }

    fn enforce_cap(&mut self) {
        while self.used > self.cap && self.done.len() > 1 {
            if let Some(old) = self.done.pop_front() {
                self.used = self.used.saturating_sub(old.bytes);
            }
        }
    }
}
