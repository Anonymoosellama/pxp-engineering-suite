// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-pointcloud: point-cloud data and processing
//!
//! Large point clouds: chunked storage, octree level of detail, clean-up and registration.
//!
//! **Status: not yet implemented.** Scheduled for Stage 5 (sub-stage Stage 5). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! Point-cloud containers with octree LOD for hundreds of millions of points, outlier removal,
//! decimation, normal estimation, and registration (RANSAC global, point-to-plane ICP). E57 I/O
//! via the MIT-licensed e57 crate.
//!
//! ## Data flow
//!
//! Read by pxp-scan and pxp-render; stored as chunked blobs by pxp-format.
//!
//! ## Invariants (once implemented)
//!
//! - Very large clouds stream; they are never required to fit in memory at once.
//!
//! ## Must never
//!
//! - Silently decimate data the user asked to keep.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 5;
