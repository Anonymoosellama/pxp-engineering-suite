// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-vnc: virtual NC controller
//!
//! G-code interpreters per controller dialect for Machine Simulation.
//!
//! **Status: not yet implemented.** Scheduled for Stage 4 (sub-stage Stage 4). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Interprets NC code per dialect (ISO, Fanuc, Siemens, Okuma, Mazak EIA/ISO, Haas): modal
//! groups, cycles, offsets, compensation, TCPM, polar and cylindrical interpolation, macros and
//! subprograms.
//!
//! ## Data flow
//!
//! Reads posted NC text, drives pxp-machine kinematics, and reports tool-tip motion for
//! deviation checks against the CAM path.
//!
//! ## Invariants (once implemented)
//!
//! - Unsupported codes stop simulation with a clear message; they are never skipped silently.
//! - The interpreter is fuzzed.
//!
//! ## Must never
//!
//! - Claim a simulated program is safe to run.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 4;
