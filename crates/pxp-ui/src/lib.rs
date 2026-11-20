// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-ui: NX-style user interface shell
//!
//! Ribbon, resource bar, navigators, top border bar, dialog framework, cue and status lines.
//!
//! **Status: not yet implemented.** Scheduled for Stage 0 (sub-stage 0H). The crate exists now
//! so the workspace layering, dependency checks and documentation are in place from day one. It
//! deliberately exports no functions: a caller cannot reach a stub that silently does nothing.
//!
//! Blocked in session 1: egui/wgpu need crates.io access.
//!
//! ## Responsibility
//!
//! The NX-style shell: data-driven ribbon built from `CommandSpec`, dockable resource bar and
//! navigators, top border bar (selection filter, snaps, curve and face rules), dialog framework
//! (selection steps, OK/Apply/Cancel, MB2 = OK), cue and status lines, Information window,
//! Command Finder, roles, layered customer defaults, remappable shortcuts and light/dark
//! themes. Toolkit chosen in ADR-0002 (egui + `egui_dock` is the leading candidate).
//!
//! ## Data flow
//!
//! Builds every ribbon from pxp-appkit's command registry; runs commands as document
//! transactions; never computes geometry itself.
//!
//! ## Invariants (once implemented)
//!
//! - The UI thread never blocks on long work.
//! - Unimplemented commands appear greyed with a 'Planned for Stage N' tooltip.
//!
//! ## Must never
//!
//! - Copy Siemens icons, help text, colour themes or trademarks.
#![forbid(unsafe_code)]

/// Stage of the master prompt in which this crate's first real functionality
/// is scheduled. Shown by `pxp-cli status` so documentation never overstates
/// what exists.
pub const PLANNED_STAGE: u8 = 0;
