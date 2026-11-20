// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-io-pdf: vector PDF writer
//!
//! Vector PDF output for drawing sheets (one page per sheet, fonts embedded).
//!
//! **Status: not yet implemented.** Scheduled for Stage 2 (sub-stage Stage 2). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Writes drawing sheets as vector PDF with embedded fonts, true line weights, layers as
//! optional content and a PDF/A option.
//!
//! ## Data flow
//!
//! Consumes Drafting's exact (never rasterised) hidden-line output.
//!
//! ## Invariants (once implemented)
//!
//! - Output is vector geometry, never a raster image of the sheet.
//!
//! ## Must never
//!
//! - Substitute fonts silently.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 2;
