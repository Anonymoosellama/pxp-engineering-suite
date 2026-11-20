// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-platform: OS abstraction
//!
//! The only crate allowed to contain OS-specific code.
//!
//! **Status: not yet implemented.** Scheduled for Stage 0 (sub-stage 0E/0J). The crate exists
//! now so the workspace layering, dependency checks and documentation are in place from day
//! one. It deliberately exports no functions: a caller cannot reach a stub that silently does
//! nothing.
//!
//! ## Responsibility
//!
//! All operating-system-specific behaviour: locations of the layered customer-defaults files
//! (site, group, user), the per-user data and crash-report directories, advisory file locks,
//! durable writes (write, fsync, atomic rename), and high-DPI and IME hooks the UI needs from
//! the OS.
//!
//! ## Data flow
//!
//! Called by pxp-format (locks, durable writes), pxp-diag (crash-report directory), pxp-ui
//! (defaults, DPI). It calls only the standard library and, later, small audited OS crates.
//!
//! ## Invariants (once implemented)
//!
//! - Every `cfg(target_os = ...)` in the workspace lives here.
//! - Durable-write helpers either leave the old file intact or the new file complete, never a
//!   torn file.
//! - Missing directories are reported as typed errors, never assumed.
//!
//! ## Must never
//!
//! - Contain domain logic (geometry, documents, CAM).
//! - Panic because a path, permission or environment variable is missing.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 0;
