// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-kernel-occt: Open CASCADE backend in a worker process
//!
//! Runs OCCT (LGPL-2.1) in a separate worker process behind the pxp-kernel trait.
//!
//! **Status: not yet implemented.** Scheduled for Stage 0 (sub-stage 0F). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! Blocked in session 1: needs OCCT bindings (cadrum or opencascade-rs) from crates.io.
//!
//! ## Responsibility
//!
//! Hosts Open CASCADE in a child process and speaks a versioned request/response protocol with
//! the in-process `KernelHost`, so a native crash kills only the worker. Used for STEP/IGES
//! exchange, shape healing, and as a cross-check oracle in tests.
//!
//! ## Data flow
//!
//! `KernelHost` serialises a request, the worker executes it with OCCT, and the reply carries
//! an `OpResult` with history. If the worker dies, the host restarts it and the document
//! transaction replays or rolls back.
//!
//! ## Invariants (once implemented)
//!
//! - A worker crash never corrupts the document: the in-flight transaction is replayed or
//!   rolled back.
//! - The future pxp-kernel-occt-sys FFI crate is the only crate allowed `unsafe`, with SAFETY
//!   comments, Miri where possible and ASan in nightly CI.
//!
//! ## Must never
//!
//! - Link OCCT into the main process by default.
//! - Be called directly by application crates.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 0;
