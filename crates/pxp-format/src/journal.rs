// SPDX-License-Identifier: LGPL-2.1-or-later
//! The write-ahead recovery journal (`part.pxp.journal`).
//!
//! ## How recovery works
//!
//! ```text
//! save ─► part.pxp (save token T) ─► journal reset: base = T, no entries
//! each committed transaction ─► journal: one row per created/modified/deleted object (full object bytes)
//! crash ─► next open: journal base == file's token and entries exist ─► "recover?"
//!          recover = file objects, then journal rows applied in order (last row per object wins)
//! ```
//!
//! The base token ties a journal to the exact saved file it extends: a
//! journal left behind by a different save of the same path (or a copied
//! file) is never applied to the wrong base.
//!
//! Each [`Journal::record`] is one SQLite transaction with
//! `synchronous = FULL`, so once a document transaction has been journalled
//! it survives a crash or power cut. Rows store complete objects (not diffs),
//! so replay needs no knowledge of the edits themselves.

use crate::store::journal_bytes;
use crate::{FormatError, codec, sql};
use pxp_doc::{ChangeSet, Object, ObjectId, Snapshot};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CREATE: &str = "
CREATE TABLE IF NOT EXISTS journal_meta (key TEXT PRIMARY KEY NOT NULL, value BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS entries (
    seq        INTEGER PRIMARY KEY AUTOINCREMENT,
    generation INTEGER NOT NULL,
    label      TEXT NOT NULL,
    object_id  BLOB NOT NULL,
    object     BLOB
);
";

/// What a recovery would restore, for the "recover?" prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverySummary {
    /// Committed transactions not yet saved.
    pub transactions: usize,
    /// Label of the last one (the user's last action).
    pub last_label: String,
}

#[derive(Debug)]
pub(crate) struct Journal {
    conn: Connection,
    path: PathBuf,
}

impl Journal {
    /// Opens (creating if needed) the journal at `path`.
    pub(crate) fn open(path: &Path) -> Result<Self, FormatError> {
        let conn = Connection::open(path).map_err(sql("open recovery journal"))?;
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(sql("configure recovery journal"))?;
        conn.execute_batch(CREATE)
            .map_err(sql("create recovery journal"))?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// The save token this journal extends.
    pub(crate) fn base_token(&self) -> Result<Option<Vec<u8>>, FormatError> {
        self.conn
            .query_row(
                "SELECT value FROM journal_meta WHERE key = 'base'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql("read recovery journal"))
    }

    /// Empties the journal and binds it to the save with `token`.
    pub(crate) fn reset(&mut self, token: &[u8]) -> Result<(), FormatError> {
        let tx = self
            .conn
            .transaction()
            .map_err(sql("reset recovery journal"))?;
        tx.execute("DELETE FROM entries", [])
            .map_err(sql("reset recovery journal"))?;
        tx.execute(
            "INSERT OR REPLACE INTO journal_meta (key, value) VALUES ('base', ?1)",
            params![token],
        )
        .map_err(sql("reset recovery journal"))?;
        tx.commit().map_err(sql("reset recovery journal"))
    }

    /// Appends one committed document transaction.
    pub(crate) fn record(
        &mut self,
        snap: &Snapshot,
        change: &ChangeSet,
    ) -> Result<(), FormatError> {
        if change.is_empty() {
            return Ok(());
        }
        let generation = i64::try_from(change.generation).unwrap_or(i64::MAX);
        let tx = self
            .conn
            .transaction()
            .map_err(sql("write recovery journal"))?;
        {
            let mut stmt = tx
                .prepare("INSERT INTO entries (generation, label, object_id, object) VALUES (?1, ?2, ?3, ?4)")
                .map_err(sql("write recovery journal"))?;
            let touched = change
                .created
                .iter()
                .chain(&change.modified)
                .chain(&change.deleted);
            for id in touched {
                // The snapshot is the state right after this change; an object
                // absent from it was deleted.
                let bytes = snap.get(*id).map(journal_bytes);
                stmt.execute(params![generation, change.label, id.to_bytes(), bytes])
                    .map_err(sql("write recovery journal"))?;
            }
        }
        tx.commit().map_err(sql("write recovery journal"))
    }

    /// Summary of what recovery would restore, or `None` if nothing.
    pub(crate) fn summary(&self) -> Result<Option<RecoverySummary>, FormatError> {
        let transactions: i64 = self
            .conn
            .query_row("SELECT COUNT(DISTINCT generation) FROM entries", [], |r| {
                r.get(0)
            })
            .map_err(sql("read recovery journal"))?;
        if transactions == 0 {
            return Ok(None);
        }
        let last_label: String = self
            .conn
            .query_row(
                "SELECT label FROM entries ORDER BY seq DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .map_err(sql("read recovery journal"))?;
        Ok(Some(RecoverySummary {
            transactions: usize::try_from(transactions).unwrap_or(usize::MAX),
            last_label,
        }))
    }

    /// Applies every entry, in order, onto `objects`.
    pub(crate) fn replay_onto(
        &self,
        objects: &mut BTreeMap<ObjectId, Object>,
    ) -> Result<(), FormatError> {
        let mut stmt = self
            .conn
            .prepare("SELECT object_id, object FROM entries ORDER BY seq")
            .map_err(sql("read recovery journal"))?;
        let mut rows = stmt.query([]).map_err(sql("read recovery journal"))?;
        while let Some(row) = rows.next().map_err(sql("read recovery journal"))? {
            let raw: Vec<u8> = row.get(0).map_err(sql("read recovery journal"))?;
            let id =
                ObjectId::from_bytes(raw.as_slice().try_into().map_err(|_| {
                    FormatError::Corrupt("journal object id is not 16 bytes".into())
                })?);
            let bytes: Option<Vec<u8>> = row.get(1).map_err(sql("read recovery journal"))?;
            match bytes {
                None => {
                    objects.remove(&id);
                }
                Some(b) => {
                    let obj = codec::decode_object(&b).map_err(|source| FormatError::Codec {
                        id: id.to_string(),
                        source,
                    })?;
                    if obj.id() != id {
                        return Err(FormatError::Corrupt(format!(
                            "journal row for {id} holds {}",
                            obj.id()
                        )));
                    }
                    objects.insert(id, obj);
                }
            }
        }
        Ok(())
    }

    /// Deletes the journal file (after save-as moved the part elsewhere).
    pub(crate) fn remove(self) -> Result<(), FormatError> {
        let path = self.path.clone();
        drop(self.conn);
        std::fs::remove_file(&path).map_err(|source| FormatError::Io {
            action: "remove",
            path,
            source,
        })
    }
}
