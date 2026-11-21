// SPDX-License-Identifier: LGPL-2.1-or-later
//! The SQLite schema of a `.pxp` file, and schema migrations.
//!
//! ## Version 1
//!
//! | Table | Holds |
//! | --- | --- |
//! | `meta` | Part-level settings: display units, tolerances, writer, save token |
//! | `objects` | Every object of every application, one row each; properties in the [`crate::codec`] encoding; the kind-specific payload as an opaque blob (or chunked into `blobs`) |
//! | `deps` | The dependency graph, one row per edge, indexed both ways |
//! | `blobs` | Payloads larger than [`crate::store::CHUNK_BYTES`], split into ordered chunks |
//!
//! The master prompt's illustrative schema also lists `topo_names`,
//! `geometry` and `components`. They are added by the migrations that
//! introduce persistent naming, the geometry store and assemblies, not
//! created empty in advance (ADR-0003).
//!
//! ## Identification
//!
//! `PRAGMA application_id` is [`APPLICATION_ID`] (`"PXP1"` in ASCII), so tools
//! such as `file(1)` and this reader can tell a part file from any other
//! SQLite database. `PRAGMA user_version` is the schema version.
//!
//! ## Migrations
//!
//! A newer build opens an older file by copying it into an in-memory
//! database and applying [`MIGRATIONS`] there, one version at a time, each in
//! its own transaction; the file on disk is upgraded only when the user saves.
//! An older build opening a newer file reads what it understands and opens it
//! read-only (the master prompt's rule), so newer data is never discarded.

use crate::FormatError;
use rusqlite::{Connection, Transaction};

/// `PRAGMA application_id` of every `.pxp` file: `"PXP1"` in ASCII.
pub const APPLICATION_ID: i32 = 0x5058_5031;

/// The schema version this build writes.
pub const SCHEMA_VERSION: i32 = 1;

/// DDL for schema version 1.
pub(crate) const CREATE_V1: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value BLOB NOT NULL
);
CREATE TABLE objects (
    id             BLOB PRIMARY KEY NOT NULL CHECK (length(id) = 16),
    kind           TEXT NOT NULL,
    kind_ver       INTEGER NOT NULL,
    owner_app      TEXT NOT NULL,
    parent         BLOB CHECK (parent IS NULL OR length(parent) = 16),
    name           TEXT,
    layer          INTEGER NOT NULL CHECK (layer BETWEEN 1 AND 256),
    state          INTEGER NOT NULL,
    diagnostic     TEXT,
    props          BLOB NOT NULL,
    payload        BLOB NOT NULL,
    payload_chunks INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE deps (
    from_id BLOB NOT NULL,
    to_id   BLOB NOT NULL,
    role    TEXT NOT NULL,
    PRIMARY KEY (from_id, to_id, role)
) WITHOUT ROWID;
CREATE INDEX deps_by_target ON deps (to_id);
CREATE TABLE blobs (
    blob_id BLOB NOT NULL,
    seq     INTEGER NOT NULL,
    data    BLOB NOT NULL,
    PRIMARY KEY (blob_id, seq)
);
";

/// One schema upgrade step, from version `from` to `from + 1`.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// Version this step upgrades from.
    pub from: i32,
    /// What it changes, for the open report.
    pub describe: &'static str,
    /// The upgrade itself, run inside a transaction.
    pub apply: fn(&Transaction<'_>) -> rusqlite::Result<()>,
}

/// Every migration this build knows. Version 1 is the first schema, so the
/// list is empty; the framework is exercised by tests with synthetic steps.
pub const MIGRATIONS: &[Migration] = &[];

/// Applies `migrations` to `conn` from version `found` up to `target`, one
/// transaction per step, and returns the descriptions of the steps applied.
///
/// # Errors
/// [`FormatError::NoMigration`] if a step is missing; database errors.
pub(crate) fn migrate(
    conn: &mut Connection,
    found: i32,
    target: i32,
    migrations: &[Migration],
) -> Result<Vec<&'static str>, FormatError> {
    let mut version = found;
    let mut applied = Vec::new();
    while version < target {
        let step = migrations
            .iter()
            .find(|m| m.from == version)
            .ok_or(FormatError::NoMigration(version))?;
        let tx = conn.transaction().map_err(crate::sql("start migration"))?;
        (step.apply)(&tx).map_err(crate::sql("migrate"))?;
        tx.pragma_update(None, "user_version", version + 1)
            .map_err(crate::sql("migrate"))?;
        tx.commit().map_err(crate::sql("commit migration"))?;
        applied.push(step.describe);
        version += 1;
    }
    Ok(applied)
}
