// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-tess: tessellation with level of detail
//!
//! Turns exact B-rep faces into display and export meshes at controlled chord and angle
//! tolerances.
//!
//! **Status: not yet implemented.** Scheduled for Stage 0 (sub-stage 0G/0H). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Watertight tessellation of kernel shapes for display (several levels of detail), STL/3MF
//! export and the disposable tessellation cache stored in .pxp files.
//!
//! ## Data flow
//!
//! Asks the kernel trait for face geometry, produces indexed triangle meshes tagged with face
//! ids for GPU picking, and hands them to pxp-render or the exporters.
//!
//! ## Invariants (once implemented)
//!
//! - Meshes of closed solids are watertight.
//! - Every triangle carries the persistent id of the face it came from.
//!
//! ## Must never
//!
//! - Be the source of truth for geometry; the exact B-rep always is.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 0;
