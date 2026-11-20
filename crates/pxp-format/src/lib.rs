// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-format: the unified .pxp file format
//!
//! Reads and writes .pxp part files (one SQLite database per part).
//!
//! **Status: not yet implemented.** Scheduled for Stage 0 (sub-stage 0E). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! Blocked in session 1: the SQLite binding (rusqlite) needs crates.io access.
//!
//! ## Responsibility
//!
//! New, open, save and save-as of .pxp files; the SQLite schema (`meta`, `objects`, `deps`,
//! `topo_names`, `expressions`, `geometry`, `components`, `blobs`); schema migrations; lazy
//! loading of large sections; the write-ahead recovery journal; rolling backups; the advisory
//! lock; byte-for-byte preservation of objects whose kind this build does not understand.
//!
//! ## Data flow
//!
//! Serialises a pxp-doc snapshot into SQLite inside one SQLite transaction (atomic save) and
//! rebuilds a document from it on open. The recovery journal is appended after every committed
//! document transaction and replayed on the next launch after a crash.
//!
//! ## Invariants (once implemented)
//!
//! - A save is atomic: after a crash the file holds either the previous save or the new one.
//! - Unknown object kinds and unknown payload versions round-trip unchanged.
//! - Geometry blobs are hash-checked on load; a mismatch is reported, never ignored.
//! - Every reader is fuzzed and never panics or hangs on malformed input.
//!
//! ## Must never
//!
//! - Write to a .pxp file outside a SQLite transaction.
//! - Drop data it cannot interpret.
//! - Open a file for writing while another session holds its lock.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 0;
