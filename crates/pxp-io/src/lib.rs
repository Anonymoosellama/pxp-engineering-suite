// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-io: data-exchange traits and translation reports
//!
//! Importer and Exporter traits, the format registry and the translation report every
//! translator returns.
//!
//! **Status: not yet implemented.** Scheduled for Stage 1 (sub-stage 1E). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! ## Responsibility
//!
//! Defines the `Importer`/`Exporter` traits (also exposed to WASM plugins), the registry that
//! picks a default export format from the active application and selection, and the report
//! listing exactly what was and was not translated.
//!
//! ## Data flow
//!
//! pxp-io-* crates implement the traits; the shell asks the registry which translators exist;
//! imported bodies enter the document through a transaction.
//!
//! ## Invariants (once implemented)
//!
//! - Every translation returns a report; nothing is dropped silently.
//!
//! ## Must never
//!
//! - Reverse-engineer a proprietary format in breach of a licence agreement.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 1;
