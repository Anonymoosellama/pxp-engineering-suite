// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-additive: additive manufacturing (slicing and deposition)
//!
//! Solid-driven slicing and custom additive operations, written from published algorithms.
//!
//! **Status: not yet implemented.** Scheduled for Stage 3 (sub-stage Stage 3). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Exact B-rep slicing, perimeters, infill, supports, seams and the custom planar, non-planar
//! and multi-axis deposition operations. Under ADR-0001 (LGPL-2.1-or-later) no PrusaSlicer code
//! may enter this crate; PrusaSlicer is only a behaviour reference and a comparison oracle run
//! as an external program.
//!
//! ## Data flow
//!
//! Slices geometry via the kernel trait; produces additive operations stored by Manufacturing.
//!
//! ## Invariants (once implemented)
//!
//! - Layers are cut from the exact B-rep; tessellation is a fallback only.
//!
//! ## Must never
//!
//! - Contain code copied, translated or paraphrased from PrusaSlicer (AGPL-3.0).
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 3;
