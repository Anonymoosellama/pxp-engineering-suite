// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-post: layered post-processor engine
//!
//! Post layer stack resolution, event pipeline, buffers, variables, formats; WASM post runtime.
//!
//! **Status: not yet implemented.** Scheduled for Stage 4 (sub-stage Stage 4). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Resolves numbered post layers (higher number wins, explicit unset, handler chaining with
//! `next()`), runs the event-to-block-to-buffer pipeline and hosts compiled posts in a wasmtime
//! sandbox.
//!
//! ## Data flow
//!
//! Manufacturing feeds toolpath events in; NC text comes out; every output word can be traced
//! to the event, block, variable and layer that produced it.
//!
//! ## Invariants (once implemented)
//!
//! - Resolution is deterministic and reports which layer supplied each value.
//!
//! ## Must never
//!
//! - Give a post file or network access beyond the host-provided output stream.
//! - Label NC output as verified.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 4;
