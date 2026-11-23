// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-format: the unified `.pxp` part file
//!
//! One `.pxp` file is one SQLite database holding every object of every
//! application in a part (ADR-0003). This crate creates, opens, saves and
//! recovers those files.
//!
//! ## Responsibility
//!
//! - [`PartFile::create`], [`PartFile::open`], [`PartFile::save`],
//!   [`PartFile::save_as`]: atomic, durable file operations.
//! - **Recovery journal**: [`PartFile::record`] after every committed document
//!   transaction; [`Opened::recovery`] and [`PartFile::recovered_document`] on
//!   the next open after a crash.
//! - **Rolling backups** of the previous saves (`part.pxp.bak1` newest).
//! - **Advisory lock** (`part.pxp.lock`) so two sessions never write one part.
//! - **Schema versions and migrations** ([`schema`]); newer files open
//!   read-only.
//! - **The explicit binary codec** ([`codec`]) for properties; payloads of
//!   unknown kinds are preserved byte for byte.
//!
//! ## Files on disk
//!
//! ```text
//! part.pxp           the part (SQLite, application_id "PXP1")
//! part.pxp.lock      advisory lock while a session has it open
//! part.pxp.journal   recovery journal: unsaved committed transactions
//! part.pxp.bak1..3   previous saves, newest first
//! ```
//!
//! ## Data flow
//!
//! ```text
//! open:  file ─► quick_check ─► (migrate in memory) ─► rows ─► Document::from_objects (validates graph)
//! edit:  Document::transact ─► ChangeSet ─► PartFile::record ─► journal (fsync)
//! save:  snapshot ─► new SQLite file (one transaction) ─► rotate backups ─► atomic rename ─► journal reset
//! ```
//!
//! ## Invariants
//!
//! - A save is atomic: after a crash, `part.pxp` is the previous save or the
//!   new one, never a mixture (temporary file + fsync + rename).
//! - Every committed transaction is in the journal before `record` returns.
//! - A journal is replayed only onto the save it extends (save token).
//! - Reading never panics on malformed input; the codec is fuzzed.
//!
//! ## Must never
//!
//! - Drop objects it does not understand.
//! - Write a file it opened read-only, or one whose lock it does not hold.
//! - Silently downgrade a file written by a newer version.
#![forbid(unsafe_code)]

pub mod codec;
mod journal;
pub mod schema;
mod store;

pub use codec::CodecError;
pub use journal::RecoverySummary;
pub use schema::{APPLICATION_ID, Migration, SCHEMA_VERSION};
pub use store::CHUNK_BYTES;

use pxp_doc::{ChangeSet, DocError, Document, IdSource, ObjectId, Snapshot, UuidV7Source};
use pxp_math::Tolerances;
use pxp_platform::{FileLock, PlatformError};
use pxp_units::{Angle, Length, UnitSystem};
use rusqlite::{Connection, OpenFlags};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Stage of the master prompt this crate belongs to (shown by
/// `pxp-cli status`).
pub const PLANNED_STAGE: u8 = 0;

/// How many previous saves are kept as backups.
pub const BACKUPS_KEPT: usize = 3;

/// Errors from file operations. The shell turns these into user messages.
#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    /// The file is not a `.pxp` part (not SQLite, or another application's
    /// database).
    #[error("{0} is not a PXP part file")]
    NotAPxpFile(PathBuf),
    /// `create` was asked to overwrite an existing file.
    #[error("{0} already exists")]
    AlreadyExists(PathBuf),
    /// The file is damaged.
    #[error("the part file is damaged: {0}")]
    Corrupt(String),
    /// The objects in the file do not form a valid document.
    #[error("the part's contents are inconsistent: {0}")]
    InvalidContent(#[source] DocError),
    /// One object's encoded data is invalid.
    #[error("object {id} could not be decoded: {source}")]
    Codec {
        /// The object.
        id: String,
        /// Why.
        #[source]
        source: CodecError,
    },
    /// The file was opened read-only (written by a newer version, or opened
    /// for viewing) and cannot be written.
    #[error("the part is open read-only: {0}")]
    ReadOnly(String),
    /// No migration exists from this schema version.
    #[error("no migration from schema version {0} exists in this build")]
    NoMigration(i32),
    /// A SQLite error.
    #[error("database error while trying to {action}: {source}")]
    Sqlite {
        /// What was being done.
        action: &'static str,
        /// The SQLite error.
        #[source]
        source: rusqlite::Error,
    },
    /// Locking or durable-write failure.
    #[error(transparent)]
    Platform(#[from] PlatformError),
    /// Another I/O failure.
    #[error("could not {action} {}: {source}", path.display())]
    Io {
        /// What was being done.
        action: &'static str,
        /// The path.
        path: PathBuf,
        /// The error.
        #[source]
        source: std::io::Error,
    },
}

/// Wraps a SQLite error with what was being attempted.
pub(crate) fn sql(action: &'static str) -> impl Fn(rusqlite::Error) -> FormatError {
    move |source| FormatError::Sqlite { action, source }
}

/// Part-level settings stored in the file (part preferences).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartMeta {
    /// Display unit system (mm or inch part).
    pub units: UnitSystem,
    /// Modeling distance and angle tolerances.
    pub tolerances: Tolerances,
}

impl PartMeta {
    /// Settings for a new millimetre part (the mm template).
    #[must_use]
    pub fn millimetre() -> Self {
        Self {
            units: UnitSystem::Millimetre,
            tolerances: Tolerances::default(),
        }
    }

    /// Settings for a new inch part (the inch template).
    #[must_use]
    pub fn inch() -> Self {
        Self {
            units: UnitSystem::Inch,
            tolerances: Tolerances::default(),
        }
    }
}

/// How the file's schema compared with this build's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaStatus {
    /// Same version.
    Current,
    /// Older; migrated in memory with these steps. Saving writes the new
    /// schema.
    Migrated {
        /// Version found in the file.
        from: i32,
        /// Steps applied.
        steps: Vec<&'static str>,
    },
    /// Newer than this build: opened read-only.
    Newer {
        /// Version found in the file.
        found: i32,
    },
}

/// The result of opening a part.
#[derive(Debug)]
pub struct Opened {
    /// The open file (holds the lock unless opened read-only).
    pub file: PartFile,
    /// The document as last saved.
    pub document: Document,
    /// Unsaved work from a crashed session, if any. Offer it to the user,
    /// then call [`PartFile::recovered_document`] or
    /// [`PartFile::discard_recovery`].
    pub recovery: Option<RecoverySummary>,
    /// How the schema compared.
    pub schema: SchemaStatus,
    /// A complete autosaved copy of unsaved work made on top of exactly
    /// this save (`<part>.pxp.autosave`). The recovery journal, which holds
    /// every committed transaction, comes first; offer the autosave when the
    /// journal has nothing (it was lost or damaged) or the user declines it,
    /// then call [`PartFile::autosaved_document`] or
    /// [`PartFile::discard_autosave`].
    pub autosave: Option<PathBuf>,
}

/// What a save did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    /// Objects written.
    pub objects: usize,
    /// Size of the saved file in bytes.
    pub bytes: u64,
    /// Backups that now exist, newest first.
    pub backups: Vec<PathBuf>,
}

/// An open part file.
#[derive(Debug)]
pub struct PartFile {
    path: PathBuf,
    meta: PartMeta,
    token: [u8; 16],
    lock: Option<FileLock>,
    journal: Option<journal::Journal>,
    read_only: Option<String>,
}

/// `part.pxp` + `suffix` (for example `part.pxp.lock`).
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(suffix);
    path.with_file_name(name)
}

fn new_token() -> [u8; 16] {
    UuidV7Source::new().next_id().to_bytes()
}

fn io<'a>(action: &'static str, path: &'a Path) -> impl FnOnce(std::io::Error) -> FormatError + 'a {
    move |source| FormatError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

impl PartFile {
    /// Creates a new part file containing `doc` (usually empty), takes its
    /// lock and starts its journal. Refuses to overwrite an existing file.
    ///
    /// # Errors
    /// [`FormatError::AlreadyExists`], lock and I/O errors.
    pub fn create(path: &Path, meta: PartMeta, doc: &Document) -> Result<Self, FormatError> {
        if path.exists() {
            return Err(FormatError::AlreadyExists(path.to_path_buf()));
        }
        let lock = FileLock::try_acquire(&sidecar(path, ".lock"))?;
        let token = new_token();
        write_file(path, meta, token, &doc.snapshot())?;
        let mut journal = journal::Journal::open(&sidecar(path, ".journal"))?;
        journal.reset(&token)?;
        Ok(Self {
            path: path.to_path_buf(),
            meta,
            token,
            lock: Some(lock),
            journal: Some(journal),
            read_only: None,
        })
    }

    /// Opens a part for editing: takes its lock, reads and validates it, and
    /// reports any recoverable work from a crashed session.
    ///
    /// # Errors
    /// [`FormatError::NotAPxpFile`], [`FormatError::Corrupt`],
    /// [`FormatError::InvalidContent`], [`PlatformError::Locked`] (via
    /// [`FormatError::Platform`]) and I/O errors.
    pub fn open(path: &Path) -> Result<Opened, FormatError> {
        open_with(path, schema::MIGRATIONS, false)
    }

    /// Opens a part for viewing only: no lock, no journal, no saving.
    ///
    /// # Errors
    /// As [`PartFile::open`], except locking.
    pub fn open_read_only(path: &Path) -> Result<Opened, FormatError> {
        open_with(path, schema::MIGRATIONS, true)
    }

    /// The file's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Part settings as last saved.
    #[must_use]
    pub fn meta(&self) -> PartMeta {
        self.meta
    }

    /// `Some(reason)` if the file cannot be written.
    #[must_use]
    pub fn read_only_reason(&self) -> Option<&str> {
        self.read_only.as_deref()
    }

    /// Journals one committed document transaction. Call right after every
    /// [`Document::transact`], undo, redo and recompute (the shell's document
    /// thread does this synchronously).
    ///
    /// # Errors
    /// [`FormatError::ReadOnly`] or database errors.
    pub fn record(&mut self, snap: &Snapshot, change: &ChangeSet) -> Result<(), FormatError> {
        self.writable()?;
        match &mut self.journal {
            Some(j) => j.record(snap, change),
            None => Err(FormatError::ReadOnly("no recovery journal is open".into())),
        }
    }

    /// The document as saved plus every journalled transaction: what the
    /// crashed session had committed.
    ///
    /// # Errors
    /// Read, decode and validation errors.
    pub fn recovered_document(&self) -> Result<Document, FormatError> {
        let (_, _, mut objects, _) = read_file(&self.path, schema::MIGRATIONS)?;
        if let Some(j) = &self.journal {
            j.replay_onto(&mut objects)?;
        }
        Document::from_objects(objects.into_values(), Box::new(UuidV7Source::new()))
            .map_err(FormatError::InvalidContent)
    }

    /// Throws away the crashed session's unsaved work.
    ///
    /// # Errors
    /// [`FormatError::ReadOnly`] or database errors.
    pub fn discard_recovery(&mut self) -> Result<(), FormatError> {
        self.writable()?;
        match &mut self.journal {
            Some(j) => j.reset(&self.token),
            None => Ok(()),
        }
    }

    /// Saves `doc` with `meta`: writes a complete new file, keeps the
    /// previous save as `.bak1` (rotating older backups), atomically replaces
    /// the part, and empties the journal.
    ///
    /// # Errors
    /// [`FormatError::ReadOnly`], database and I/O errors. On error the
    /// previous save is untouched.
    pub fn save(&mut self, doc: &Document, meta: PartMeta) -> Result<SaveReport, FormatError> {
        self.writable()?;
        let token = new_token();
        let temp = sidecar(&self.path, &format!(".tmp-{}", std::process::id()));
        let objects = write_file(&temp, meta, token, &doc.snapshot())?;
        let backups = rotate_backups(&self.path)?;
        pxp_platform::replace_file(&temp, &self.path)?;
        self.token = token;
        self.meta = meta;
        if let Some(j) = &mut self.journal {
            j.reset(&token)?;
        }
        // Everything is saved: an autosave of earlier unsaved work is stale.
        self.discard_autosave()?;
        let bytes = std::fs::metadata(&self.path)
            .map_err(io("read size of", &self.path))?
            .len();
        Ok(SaveReport {
            objects,
            bytes,
            backups,
        })
    }

    /// Writes a complete copy of `doc` to `<part>.pxp.autosave`, atomically,
    /// tagged with this file's save token so a later open knows it extends
    /// exactly this save. The part file, its lock and its journal are not
    /// touched. A save or a clean close removes the autosave.
    ///
    /// # Errors
    /// [`FormatError::ReadOnly`] for a read-only file; I/O or SQLite errors.
    pub fn autosave(&self, doc: &Document) -> Result<PathBuf, FormatError> {
        self.writable()?;
        let target = sidecar(&self.path, ".autosave");
        let temp = sidecar(&self.path, &format!(".autosave-tmp-{}", std::process::id()));
        write_file(&temp, self.meta, self.token, &doc.snapshot())?;
        pxp_platform::replace_file(&temp, &target)?;
        Ok(target)
    }

    /// The document as autosaved (see [`Opened::autosave`]).
    ///
    /// # Errors
    /// [`FormatError::Corrupt`] if there is no autosave for this save, or
    /// the errors of reading a part file.
    pub fn autosaved_document(&self) -> Result<Document, FormatError> {
        let path = sidecar(&self.path, ".autosave");
        let (_, base, objects, _) = read_file(&path, &[])?;
        if base != self.token {
            return Err(FormatError::Corrupt(
                "the autosave belongs to a different save of this part".into(),
            ));
        }
        Document::from_objects(objects.into_values(), Box::new(UuidV7Source::new()))
            .map_err(FormatError::InvalidContent)
    }

    /// Deletes this part's autosave, if any.
    ///
    /// # Errors
    /// I/O errors deleting it.
    pub fn discard_autosave(&self) -> Result<(), FormatError> {
        let path = sidecar(&self.path, ".autosave");
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io("delete autosave", &path)(e)),
        }
    }

    /// Saves to a new path and continues working there ("Save As").
    ///
    /// The new path's lock is taken first, so another session's file is
    /// never overwritten. An existing unlocked file at `path` is replaced,
    /// with a backup, as Save As does everywhere. On success the old path's
    /// lock and journal are released; on failure the part stays open at the
    /// old path exactly as before.
    ///
    /// # Errors
    /// As [`PartFile::save`], plus [`PlatformError::Locked`] (via
    /// [`FormatError::Platform`]) if another session holds `path`.
    pub fn save_as(
        &mut self,
        path: &Path,
        doc: &Document,
        meta: PartMeta,
    ) -> Result<SaveReport, FormatError> {
        self.writable()?;
        let lock = FileLock::try_acquire(&sidecar(path, ".lock"))?;
        let journal = journal::Journal::open(&sidecar(path, ".journal"))?;
        let old_path = std::mem::replace(&mut self.path, path.to_path_buf());
        let old_lock = self.lock.replace(lock);
        let old_journal = self.journal.replace(journal);
        match self.save(doc, meta) {
            Ok(report) => {
                drop(old_lock);
                if let Some(j) = old_journal {
                    j.remove()?;
                }
                Ok(report)
            }
            Err(e) => {
                self.path = old_path;
                self.lock = old_lock;
                self.journal = old_journal;
                Err(e)
            }
        }
    }

    /// Closes the file. A journal holding no unsaved transactions is
    /// deleted, so a cleanly closed part leaves only the `.pxp`, its
    /// backups and its lock file (`.lock`, kept: deleting it safely would
    /// need every platform to re-check the file identity after locking).
    /// Unsaved transactions stay in the journal for recovery on next open.
    /// Dropping a `PartFile` without `close` is safe too: the journal is
    /// simply kept.
    ///
    /// # Errors
    /// I/O or SQLite errors reading or deleting the journal.
    pub fn close(mut self) -> Result<(), FormatError> {
        if let Some(journal) = self.journal.take()
            && journal.summary()?.is_none()
        {
            journal.remove()?;
            // Nothing unsaved: an autosave can only be stale.
            self.discard_autosave()?;
        }
        Ok(())
    }

    /// Existing backup files, newest first.
    #[must_use]
    pub fn backups(&self) -> Vec<PathBuf> {
        (1..=BACKUPS_KEPT)
            .map(|n| sidecar(&self.path, &format!(".bak{n}")))
            .filter(|p| p.exists())
            .collect()
    }

    fn writable(&self) -> Result<(), FormatError> {
        match &self.read_only {
            Some(reason) => Err(FormatError::ReadOnly(reason.clone())),
            None => Ok(()),
        }
    }
}

/// Shifts `.bak1 → .bak2 → …` and copies the current file to `.bak1`.
/// Copy, not rename: the part must exist until the atomic replace.
fn rotate_backups(path: &Path) -> Result<Vec<PathBuf>, FormatError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    for n in (1..BACKUPS_KEPT).rev() {
        let from = sidecar(path, &format!(".bak{n}"));
        if from.exists() {
            let to = sidecar(path, &format!(".bak{}", n + 1));
            std::fs::rename(&from, &to).map_err(io("rotate backup", &from))?;
        }
    }
    let newest = sidecar(path, ".bak1");
    std::fs::copy(path, &newest).map_err(io("back up", path))?;
    Ok((1..=BACKUPS_KEPT)
        .map(|n| sidecar(path, &format!(".bak{n}")))
        .filter(|p| p.exists())
        .collect())
}

/// Writes a complete part file at `path` (replacing any file there).
fn write_file(
    path: &Path,
    meta: PartMeta,
    token: [u8; 16],
    snap: &Snapshot,
) -> Result<usize, FormatError> {
    if path.exists() {
        std::fs::remove_file(path).map_err(io("remove stale temporary file", path))?;
    }
    let mut conn = Connection::open(path).map_err(sql("create part file"))?;
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(sql("configure part file"))?;
    conn.pragma_update(None, "application_id", APPLICATION_ID)
        .map_err(sql("configure part file"))?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(sql("configure part file"))?;
    let tx = conn.transaction().map_err(sql("start writing part"))?;
    tx.execute_batch(schema::CREATE_V1)
        .map_err(sql("create schema"))?;
    store::put_meta(
        &tx,
        "units",
        match meta.units {
            UnitSystem::Millimetre => b"mm",
            UnitSystem::Inch => b"in",
        },
    )?;
    store::put_meta(
        &tx,
        "distance_tolerance_mm",
        &meta.tolerances.distance().mm().to_le_bytes(),
    )?;
    store::put_meta(
        &tx,
        "angle_tolerance_rad",
        &meta.tolerances.angle().rad().to_le_bytes(),
    )?;
    store::put_meta(
        &tx,
        "writer",
        format!("pxp-format {}", env!("CARGO_PKG_VERSION")).as_bytes(),
    )?;
    store::put_meta(&tx, "save_token", &token)?;
    let n = store::write_objects(&tx, snap)?;
    tx.commit().map_err(sql("commit part"))?;
    conn.close().map_err(|(_, e)| sql("close part file")(e))?;
    Ok(n)
}

type Loaded = (
    PartMeta,
    [u8; 16],
    std::collections::BTreeMap<ObjectId, pxp_doc::Object>,
    SchemaStatus,
);

/// Reads a part file: identification, integrity, schema, settings, objects.
fn read_file(path: &Path, migrations: &[Migration]) -> Result<Loaded, FormatError> {
    if !path.exists() {
        return Err(FormatError::Io {
            action: "open",
            path: path.to_path_buf(),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        });
    }
    let not_pxp = |_| FormatError::NotAPxpFile(path.to_path_buf());
    let src = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(not_pxp)?;
    let app_id: i32 = src
        .pragma_query_value(None, "application_id", |r| r.get(0))
        .map_err(not_pxp)?;
    if app_id != APPLICATION_ID {
        return Err(FormatError::NotAPxpFile(path.to_path_buf()));
    }
    let check: String = src
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(sql("check integrity"))?;
    if check != "ok" {
        return Err(FormatError::Corrupt(format!("integrity check: {check}")));
    }
    let version: i32 = src
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(sql("read schema version"))?;
    let (conn, status) = match version.cmp(&SCHEMA_VERSION) {
        std::cmp::Ordering::Equal => (src, SchemaStatus::Current),
        std::cmp::Ordering::Greater => (src, SchemaStatus::Newer { found: version }),
        std::cmp::Ordering::Less => {
            let mut mem = Connection::open_in_memory().map_err(sql("prepare migration"))?;
            {
                let backup = rusqlite::backup::Backup::new(&src, &mut mem)
                    .map_err(sql("copy for migration"))?;
                backup
                    .run_to_completion(256, Duration::ZERO, None)
                    .map_err(sql("copy for migration"))?;
            }
            let steps = schema::migrate(&mut mem, version, SCHEMA_VERSION, migrations)?;
            (
                mem,
                SchemaStatus::Migrated {
                    from: version,
                    steps,
                },
            )
        }
    };
    let meta = read_meta(&conn)?;
    let token: [u8; 16] = store::get_meta(&conn, "save_token")?
        .and_then(|t| t.try_into().ok())
        .ok_or_else(|| FormatError::Corrupt("missing save token".into()))?;
    let objects = store::read_objects(&conn)?;
    Ok((meta, token, objects, status))
}

fn read_meta(conn: &Connection) -> Result<PartMeta, FormatError> {
    let f64_meta = |key: &str| -> Result<f64, FormatError> {
        let bytes = store::get_meta(conn, key)?
            .ok_or_else(|| FormatError::Corrupt(format!("missing setting {key}")))?;
        let arr: [u8; 8] = bytes
            .try_into()
            .map_err(|_| FormatError::Corrupt(format!("setting {key} is malformed")))?;
        Ok(f64::from_le_bytes(arr))
    };
    let units = match store::get_meta(conn, "units")?.as_deref() {
        Some(b"mm") => UnitSystem::Millimetre,
        Some(b"in") => UnitSystem::Inch,
        _ => {
            return Err(FormatError::Corrupt(
                "missing or unknown display units".into(),
            ));
        }
    };
    let tolerances = Tolerances::new(
        Length::from_mm(f64_meta("distance_tolerance_mm")?),
        Angle::from_rad(f64_meta("angle_tolerance_rad")?),
    )
    .map_err(|e| FormatError::Corrupt(e.to_string()))?;
    Ok(PartMeta { units, tolerances })
}

fn open_with(
    path: &Path,
    migrations: &[Migration],
    view_only: bool,
) -> Result<Opened, FormatError> {
    let lock = if view_only {
        None
    } else {
        Some(FileLock::try_acquire(&sidecar(path, ".lock"))?)
    };
    let (meta, token, objects, schema) = read_file(path, migrations)?;
    let document = Document::from_objects(objects.into_values(), Box::new(UuidV7Source::new()))
        .map_err(FormatError::InvalidContent)?;
    let read_only = match (&schema, view_only) {
        (_, true) => Some("opened for viewing".to_owned()),
        (SchemaStatus::Newer { found }, _) => Some(format!(
            "written by a newer version (schema {found}; this build writes {SCHEMA_VERSION})"
        )),
        _ => None,
    };
    let journal_path = sidecar(path, ".journal");
    let (journal, recovery) = if read_only.is_some() {
        (None, None)
    } else {
        let mut journal = journal::Journal::open(&journal_path)?;
        let recovery = if journal.base_token()?.as_deref() == Some(&token[..]) {
            journal.summary()?
        } else {
            // A journal for some other save of this path is meaningless here.
            journal.reset(&token)?;
            None
        };
        (Some(journal), recovery)
    };
    // An autosave counts only if it was made on top of this exact save
    // (its token is the part's); one left by another save is stale.
    let autosave_path = sidecar(path, ".autosave");
    let autosave = if read_only.is_none() && autosave_path.exists() {
        match read_file(&autosave_path, migrations) {
            Ok((_, base, _, _)) if base == token => Some(autosave_path),
            _ => {
                let _ = std::fs::remove_file(&autosave_path);
                None
            }
        }
    } else {
        None
    };
    Ok(Opened {
        file: PartFile {
            path: path.to_path_buf(),
            meta,
            token,
            lock,
            journal,
            read_only,
        },
        document,
        recovery,
        schema,
        autosave,
    })
}

/// Writes an autosave copy of the open part when it is due: the document
/// changed since the last save or autosave, and `interval` has passed since
/// the last autosave. The application calls [`Autosaver::tick`] from its
/// idle loop and [`Autosaver::saved`] after every save.
///
/// The recovery journal already keeps every committed transaction; the
/// autosave is a second, independent copy for when the journal is lost or
/// damaged (a full disk, a deleted sidecar), the way NX autosaves parts.
#[derive(Debug, Clone)]
pub struct Autosaver {
    interval: Duration,
    generation: u64,
    last: std::time::Instant,
}

impl Autosaver {
    /// Starts tracking a document at `generation` (just opened or saved).
    #[must_use]
    pub fn new(interval: Duration, generation: u64, now: std::time::Instant) -> Self {
        Self {
            interval,
            generation,
            last: now,
        }
    }

    /// Records a save: nothing is unsaved at `generation`.
    pub fn saved(&mut self, generation: u64, now: std::time::Instant) {
        self.generation = generation;
        self.last = now;
    }

    /// Autosaves if due; returns the autosave's path when it wrote one.
    ///
    /// # Errors
    /// As [`PartFile::autosave`].
    pub fn tick(
        &mut self,
        file: &PartFile,
        doc: &Document,
        now: std::time::Instant,
    ) -> Result<Option<PathBuf>, FormatError> {
        if doc.generation() == self.generation
            || now.saturating_duration_since(self.last) < self.interval
        {
            return Ok(None);
        }
        let path = file.autosave(doc)?;
        self.generation = doc.generation();
        self.last = now;
        Ok(Some(path))
    }
}

#[cfg(test)]
mod tests;
