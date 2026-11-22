// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-ui: the NX-style desktop shell
//!
//! egui + `egui_dock` on wgpu (ADR-0002). Everything shown comes from data:
//! the ribbon from the application registry's `CommandSpec`s, the Part
//! Navigator from the document, the graphics window from `pxp-render`.
//!
//! | Module | What it holds | Tested without a window |
//! | --- | --- | --- |
//! | [`defaults`] | layered customer defaults (shipped < site < user, site locks) | yes |
//! | [`keys`] | key chords and the remappable shortcut table, with conflict reports | yes |
//! | [`ribbon`] | the data-driven ribbon and roles | yes |
//! | [`dialog`] | the dialog framework (fields, OK/Apply/Cancel, MB2 = OK) | yes |
//! | [`viewport`] | the graphics window: texture display, mouse grammar, overlays | GPU tests in `pxp-render` |
//! | [`shell`] | the eframe application tying them together | scripted screenshots (`pxp --screenshot`) |
//!
//! ## Must never
//!
//! - Hold model data: the document (through `PartSession`) owns it.
//! - Hide a command: planned commands are shown greyed with "Planned for
//!   Stage N." as their tooltip.
//! - Do nothing silently: every refused action says why on the status line.
#![forbid(unsafe_code)]

pub mod defaults;
pub mod dialog;
pub mod keys;
pub mod ribbon;
pub mod shell;
pub mod viewport;

pub use shell::{Autostart, Shell, ShellConfig};

/// Re-export so `pxp-app` uses the same eframe.
pub use eframe;
