// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-io-stl: STL translator
//!
//! STL import (as facet bodies) and export.
//!
//! **Status: not yet implemented.** Scheduled for Stage 1 (sub-stage 1E). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! ## Responsibility
//!
//! Reads ASCII and binary STL into facet bodies, and writes tessellated bodies.
//!
//! ## Data flow
//!
//! Implements pxp-io's traits; export tessellates through pxp-tess.
//!
//! ## Invariants (once implemented)
//!
//! - The reader is fuzzed and never panics or hangs.
//!
//! ## Must never
//!
//! - Claim an STL import is an exact B-rep.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 1;
