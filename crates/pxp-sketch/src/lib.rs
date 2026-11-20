// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-sketch: 2D and 3D geometric constraint solver
//!
//! Sketch constraint solver (starting from SolidCraft's solver, taken under MIT).
//!
//! **Status: not yet implemented.** Scheduled for Stage 1 (sub-stage 1A). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! ## Responsibility
//!
//! Solves sketch constraints (coincident, tangent, parallel, dimensions and the rest), reports
//! degrees of freedom and names conflicting constraints. Method per the master prompt:
//! decompose the constraint graph into independent clusters, solve each with Newton-Raphson
//! plus Levenberg-Marquardt damping, and prefer the solution closest to the previous state.
//!
//! ## Data flow
//!
//! Modeling sketches hand their curves, constraints and expression-driven dimension values to
//! the solver; solved positions come back into the sketch object inside the same document
//! transaction.
//!
//! ## Invariants (once implemented)
//!
//! - Solutions are deterministic for identical inputs.
//! - Over-constrained input yields a diagnostic naming the conflicting constraints, never a
//!   silently ignored constraint.
//!
//! ## Must never
//!
//! - Flip a sketch to a distant solution branch when a nearby one exists.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 1;
