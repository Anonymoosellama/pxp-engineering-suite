// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-io-iges: IGES translator
//!
//! IGES import and export (curves and surfaces).
//!
//! **Status: not yet implemented.** Scheduled for Stage 1 (sub-stage 1E). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! ## Responsibility
//!
//! Reads and writes IGES 5.3 curves, surfaces and trimmed surfaces.
//!
//! ## Data flow
//!
//! Implements pxp-io's traits; geometry through the kernel trait.
//!
//! ## Invariants (once implemented)
//!
//! - The reader is fuzzed and never panics or hangs.
//!
//! ## Must never
//!
//! - Report success for entities it skipped.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 1;
