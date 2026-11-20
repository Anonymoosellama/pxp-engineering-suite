// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-kernel-native: in-process pure-Rust geometry kernel backend
//!
//! Adapter that runs a pure-Rust B-rep kernel (bake-off candidate: SolidCore) behind the pxp-
//! kernel trait.
//!
//! **Status: not yet implemented.** Scheduled for Stage 0 (sub-stage 0F). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! Blocked in session 1: kernel crates need crates.io / git access. Linking SolidCore
//! (Apache-2.0 only) means binaries are conveyed under LGPL-3.0 (ADR-0001).
//!
//! ## Responsibility
//!
//! Implements `pxp_kernel::GeometryKernel` on top of a pure-Rust B-rep kernel so it can run in-
//! process. SolidCore (an Apache-2.0 safe-Rust port of NVIDIA SMLib) is the leading candidate;
//! the Stage 0 bake-off (ADR-0004) decides.
//!
//! ## Data flow
//!
//! pxp-kernel's `KernelHost` forwards operations here; results come back as `OpResult` values
//! with a complete `TopoHistory`, from which pxp-doc builds persistent names.
//!
//! ## Invariants (once implemented)
//!
//! - Every topological operation returns a complete history map.
//! - Identical inputs give identical topology order and geometry hash on every OS.
//!
//! ## Must never
//!
//! - Be called directly by application crates (only shell crates wire backends).
//! - Return plausible-looking geometry for an operation it cannot perform; it returns a typed
//!   refusal instead.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 0;
