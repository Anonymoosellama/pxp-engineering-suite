// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-topopt: topology optimization
//!
//! SIMP and BESO topology optimization on the pxp-fea solvers, with manufacturing constraints.
//!
//! **Status: not yet implemented.** Scheduled for Stage 5 (sub-stage Stage 5). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Density-based SIMP (filtering, Heaviside projection) and BESO, optimality-criteria and MMA
//! optimisers, manufacturing constraints, and iso-surface extraction to facet or fitted bodies.
//! beso (LGPL-3.0) is a method reference only unless an LGPL-3.0 crate is created for ported
//! code (ADR-0001).
//!
//! ## Data flow
//!
//! Reuses pxp-fea assembly and solvers each iteration; publishes results as Modeling bodies.
//!
//! ## Invariants (once implemented)
//!
//! - Studies are never re-run silently.
//!
//! ## Must never
//!
//! - Copy beso code into this LGPL-2.1-or-later crate.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 5;
