// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-app-manufacturing: the Manufacturing application (subtractive, additive and Machine Simulation)
//!
//! Operation Navigator with program, tool, geometry and method parent groups; milling, turning, mill-turn and additive operations; toolpath verification; and, from Stage 4, Machine Simulation.
//!
//! **Status:** registered with the application registry so its ribbon
//! tabs, navigators and commands appear (greyed, with the tooltip "Planned
//! for Stage N.") and switching to it works with the part still open. Its
//! first real commands arrive in Stage 3; nothing here is a stub
//! that pretends to work.
//!
//! ## Responsibility
//!
//! Owns the schema of the object kinds in [`KINDS`] and contributes the
//! commands in its ribbon. Like every application it is an editor over the
//! one shared document: switching to it copies or converts nothing.
//!
//! ## Data flow
//!
//! Registered by the shell into `pxp_appkit::AppRegistry`; receives change
//! sets through `on_upstream_change`; regenerates its kinds through
//! `Application::recompute` once implemented.
//!
//! ## Invariants
//!
//! - [`APP_ID`] and every kind name are persisted in files and never change.
//! - Intended update policy once implemented: Manual: toolpaths are marked 'regenerate needed' and never regenerated silently; post output is marked stale. Until then the
//!   safe default (manual, refuse to regenerate) applies.
//!
//! ## Must never
//!
//! - Depend on another application crate (shared logic lives in domain
//!   libraries).
//! - Copy Siemens icons, help text or trademarks.
#![forbid(unsafe_code)]

use pxp_appkit::{
    AppId, Application, ChangeCtx, ChangeSet, CommandSpec, CommandStatus, ObjectKind,
    RibbonPlacement, UiContext,
};

/// Stable application ID persisted in `.pxp` files.
pub const APP_ID: AppId = AppId::from_static("pxp.manufacturing");

/// Object kinds whose schema this application owns.
pub static KINDS: [ObjectKind; 6] = [
    ObjectKind::from_static("cam.program"),
    ObjectKind::from_static("cam.tool"),
    ObjectKind::from_static("cam.geometry"),
    ObjectKind::from_static("cam.method"),
    ObjectKind::from_static("cam.operation"),
    ObjectKind::from_static("cam.ipw"),
];

/// Ribbon tabs shown while this application is active, left to right.
pub const RIBBON_TABS: &[&str] = &["Home", "Operations", "Analysis", "Tools", "Application"];

/// Navigators shown in the resource bar while this application is active.
pub const NAVIGATORS: &[&str] = &["Operation Navigator", "Machine Tool"];

/// Command table: (id, label, tab, group, default shortcut, planned stage,
/// description). Descriptions are PXP's own words.
type CommandRow = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    u8,
    &'static str,
);
const COMMANDS: &[CommandRow] = &[
    (
        "cam.create_program",
        "Create Program",
        "Home",
        "Insert",
        None,
        3,
        "Add a program group.",
    ),
    (
        "cam.create_tool",
        "Create Tool",
        "Home",
        "Insert",
        None,
        3,
        "Add a cutting tool or nozzle.",
    ),
    (
        "cam.create_geometry",
        "Create Geometry",
        "Home",
        "Insert",
        None,
        3,
        "Add an MCS, workpiece or sub-geometry group.",
    ),
    (
        "cam.create_method",
        "Create Method",
        "Home",
        "Insert",
        None,
        3,
        "Add a machining method group.",
    ),
    (
        "cam.create_operation",
        "Create Operation",
        "Home",
        "Insert",
        None,
        3,
        "Add a milling, drilling, turning or additive operation.",
    ),
    (
        "cam.generate",
        "Generate",
        "Operations",
        "Toolpath",
        None,
        3,
        "Generate toolpaths for the selected operations.",
    ),
    (
        "cam.verify",
        "Verify",
        "Operations",
        "Toolpath",
        None,
        3,
        "Replay toolpaths with material removal and gouge checks.",
    ),
    (
        "cam.machine_simulation",
        "Machine Simulation",
        "Analysis",
        "Simulation",
        None,
        4,
        "Simulate posted NC code on a machine kit.",
    ),
    (
        "cam.post_process",
        "Post Process",
        "Operations",
        "Output",
        None,
        4,
        "Post-process programs to NC code.",
    ),
];

/// The Manufacturing application.
#[derive(Debug, Default, Clone, Copy)]
pub struct Manufacturing;

impl Application for Manufacturing {
    fn id(&self) -> AppId {
        APP_ID
    }

    fn name(&self) -> &'static str {
        "Manufacturing"
    }

    fn planned_stage(&self) -> u8 {
        3
    }

    fn owned_kinds(&self) -> &[ObjectKind] {
        &KINDS
    }

    fn commands(&self) -> Vec<CommandSpec> {
        COMMANDS
            .iter()
            .map(
                |&(id, label, tab, group, shortcut, stage, description)| CommandSpec {
                    id: id.into(),
                    label: label.into(),
                    app: APP_ID,
                    placement: RibbonPlacement {
                        tab: tab.into(),
                        group: group.into(),
                    },
                    shortcut: shortcut.map(Into::into),
                    description: description.into(),
                    status: CommandStatus::Planned { stage },
                },
            )
            .collect()
    }

    fn on_upstream_change(&self, _ctx: &mut ChangeCtx, _change: &ChangeSet) {
        // The document already marks every transitive dependant out of date
        // at commit. This application has no extra derived data yet.
    }

    fn activate(&self, ui: &mut UiContext) {
        ui.ribbon_tabs = RIBBON_TABS.to_vec();
        ui.navigators = NAVIGATORS.to_vec();
    }

    fn deactivate(&self, ui: &mut UiContext) {
        ui.ribbon_tabs.clear();
        ui.navigators.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_kinds_are_valid_and_namespaced() {
        assert!(APP_ID.is_valid());
        for k in &KINDS {
            assert!(k.is_valid(), "{k}");
            assert_eq!(k.namespace(), "cam");
        }
    }

    #[test]
    fn every_command_is_honestly_planned() {
        let cmds = Manufacturing.commands();
        assert!(!cmds.is_empty());
        for c in &cmds {
            assert!(c.id.starts_with(concat!("cam", ".")), "{}", c.id);
            assert!(!c.is_available(), "{} must not claim to work yet", c.id);
            assert!(c.tooltip().starts_with("Planned for Stage "));
            assert!(
                RIBBON_TABS.contains(&c.placement.tab.as_ref()),
                "{} on unknown tab",
                c.id
            );
        }
    }

    #[test]
    fn activation_sets_ribbon_and_navigators() {
        let mut ui = UiContext::default();
        Manufacturing.activate(&mut ui);
        assert_eq!(ui.ribbon_tabs, RIBBON_TABS);
        assert_eq!(ui.navigators, NAVIGATORS);
        Manufacturing.deactivate(&mut ui);
        assert!(ui.ribbon_tabs.is_empty());
    }
}
