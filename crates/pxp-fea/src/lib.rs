// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-fea: finite element analysis
//!
//! Linear static, modal and harmonic FEA: meshing interface, assembly, solvers, results.
//!
//! **Status: not yet implemented.** Scheduled for Stage 5 (sub-stage Stage 5). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Element library, parallel assembly, boundary conditions, sparse direct (faer) and iterative
//! (PCG with AMG, GPU SpMV with CPU reference) solvers, modal (shift-invert Lanczos or LOBPCG)
//! and harmonic response, and results.
//!
//! ## Data flow
//!
//! Reads idealised geometry and loads keyed by persistent names; writes chunked result blobs.
//! Solver units are one consistent system (mm, N, MPa, tonne, s), converted at the boundary
//! through pxp-units.
//!
//! ## Invariants (once implemented)
//!
//! - Parallel reductions are deterministic.
//! - Every GPU path has a CPU reference and an agreement test.
//!
//! ## Must never
//!
//! - Link CalculiX or Gmsh (GPL); they run only as external processes for cross-validation.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 5;
