// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-cam: subtractive CAM toolpath engines
//!
//! Milling, hole-making, turning and mill-turn toolpath generation and verification.
//!
//! **Status: not yet implemented.** Scheduled for Stage 3 (sub-stage Stage 3). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Toolpath engines (2.5D, hole making, 3-axis, stock-recognition and peel milling, 3+2,
//! 5-axis, rotary, C-axis substitution, turning, mill-turn), the shared cut-level and non-
//! cutting-move engine, the in-process workpiece (IPW) and toolpath verification. Adaptive and
//! constant-engagement methods are implemented from published papers (patent caution; see
//! docs/CITATIONS.md).
//!
//! ## Data flow
//!
//! Reads document snapshots and geometry through the kernel trait; produces toolpath objects
//! that Manufacturing stores; IPW computation has a GPU path with a CPU reference.
//!
//! ## Invariants (once implemented)
//!
//! - Toolpaths are never regenerated silently after a geometry change; they are marked
//!   'regenerate needed'.
//! - Every GPU path has a CPU reference and an agreement test.
//!
//! ## Must never
//!
//! - Emit a toolpath that gouges the part without a gouge-check result attached.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 3;
