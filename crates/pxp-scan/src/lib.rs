// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-scan: scan to CAD
//!
//! Mesh reconstruction, segmentation, primitive and NURBS fitting, deviation analysis.
//!
//! **Status: not yet implemented.** Scheduled for Stage 5 (sub-stage Stage 5). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Screened Poisson and ball-pivoting reconstruction, repair, curvature-based segmentation,
//! RANSAC plus least-squares primitive fitting, NURBS patch fitting, automatic surfacing and
//! deviation maps.
//!
//! ## Data flow
//!
//! Consumes pxp-pointcloud data; publishes fitted surfaces and features into Modeling through
//! document transactions.
//!
//! ## Invariants (once implemented)
//!
//! - Every fit reports RMS and maximum error.
//!
//! ## Must never
//!
//! - Present a fitted surface without its deviation statistics.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 5;
