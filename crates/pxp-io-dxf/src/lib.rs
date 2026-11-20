// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-io-dxf: DXF translator
//!
//! DXF import and export (default export for sketches, curves and drawing sheets).
//!
//! **Status: not yet implemented.** Scheduled for Stage 1 (sub-stage 1E). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! ## Responsibility
//!
//! Reads and writes DXF: 2D geometry with layers mapped, and from Stage 2 drawing sheets with
//! blocks, text kept as text and correct scale.
//!
//! ## Data flow
//!
//! Implements pxp-io's traits.
//!
//! ## Invariants (once implemented)
//!
//! - The reader is fuzzed and never panics or hangs.
//!
//! ## Must never
//!
//! - Explode text into lines unless the user asks.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 1;
