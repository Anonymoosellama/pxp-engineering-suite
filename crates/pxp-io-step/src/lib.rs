// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-io-step: STEP AP203/AP214/AP242 translator
//!
//! STEP import and export (default export for solid and sheet bodies: AP242).
//!
//! **Status: not yet implemented.** Scheduled for Stage 1 (sub-stage 1E). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! ## Responsibility
//!
//! Reads and writes ISO 10303 STEP files: AP203, AP214 and AP242 with colours, names, layers
//! and, from Stage 2, semantic PMI.
//!
//! ## Data flow
//!
//! Implements pxp-io's `Importer`/`Exporter`; geometry goes through the kernel trait; the OCCT
//! worker may serve as a translator and as a round-trip oracle.
//!
//! ## Invariants (once implemented)
//!
//! - The reader is fuzzed and never panics or hangs.
//! - Round-trips preserve volume, area and centroid within the Stage 1 gate tolerance (1e-6
//!   relative).
//!
//! ## Must never
//!
//! - Report success for entities it skipped.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 1;
