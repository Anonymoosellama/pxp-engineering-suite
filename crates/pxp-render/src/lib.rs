// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-render: wgpu scene renderer
//!
//! wgpu viewer: shaded and wireframe display, GPU ID-buffer picking, edges, silhouettes,
//! section clipping.
//!
//! **Status: not yet implemented.** Scheduled for Stage 0 (sub-stage 0H). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! Blocked in session 1: wgpu needs crates.io access.
//!
//! ## Responsibility
//!
//! The graphics window: rendering styles, GPU ID-buffer picking, edges and silhouettes, order-
//! independent transparency, section planes, view triad, named and standard views, instancing
//! for assemblies and an annotation overlay. Falls back to the OS software rasteriser (WARP,
//! llvmpipe) with a warning when no usable GPU exists.
//!
//! ## Data flow
//!
//! Reads immutable document snapshots and tessellations; never writes to the document. Picking
//! results go back to the UI as object ids plus persistent topology names.
//!
//! ## Invariants (once implemented)
//!
//! - Rendering never blocks the document thread or the UI thread.
//! - Picking returns persistent ids, never transient triangle indices.
//!
//! ## Must never
//!
//! - Mutate the document.
//! - Silently render nothing when the GPU is unavailable; it falls back and warns.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 0;
