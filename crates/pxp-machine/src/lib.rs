// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-machine: machine kit model and kinematics
//!
//! Machine kit folder model (machine.toml, graphics, setup), kinematic chains and limits.
//!
//! **Status: not yet implemented.** Scheduled for Stage 3 (sub-stage Stage 3 reader / Stage 4
//! editor). The crate exists now so the workspace layering, dependency checks and documentation
//! are in place from day one. It deliberately exports no functions: a caller cannot reach a
//! stub that silently does nothing.
//!
//! ## Responsibility
//!
//! The machine-kit data model and folder reader/writer, forward and inverse kinematics, travel
//! limits and tool changers. Stage 3 builds the reader and ships read-only generic kits; Stage
//! 4's Machine Kit Builder edits kits through this crate.
//!
//! ## Data flow
//!
//! Read by pxp-cam (limits, pockets), pxp-post (layer 1) and pxp-vnc (kinematics during
//! simulation).
//!
//! ## Invariants (once implemented)
//!
//! - A kit change marks dependent posts for rebuild and clears verified flags.
//!
//! ## Must never
//!
//! - Mark any NC output as verified.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 3;
