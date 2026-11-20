// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-app-machine-kit: the Machine Kit Builder application
//!
//! Machine kits: kinematic chains, graphics, tool changers, junctions, limits and controller binding, written to a portable machine folder.
//!
//! **Status:** registered with the application registry so its ribbon
//! tabs, navigators and commands appear (greyed, with the tooltip "Planned
//! for Stage N.") and switching to it works with the part still open. Its
//! first real commands arrive in Stage 4; nothing here is a stub
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
//! - Intended update policy once implemented: Manual: a kit change marks dependent posts for rebuild and clears verified flags. Until then the
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
pub const APP_ID: AppId = AppId::from_static("pxp.machine-kit");

/// Object kinds whose schema this application owns.
pub static KINDS: [ObjectKind; 1] = [ObjectKind::from_static("machine.kit_ref")];

/// Ribbon tabs shown while this application is active, left to right.
pub const RIBBON_TABS: &[&str] = &["Home", "Application"];

/// Navigators shown in the resource bar while this application is active.
pub const NAVIGATORS: &[&str] = &["Kinematics Navigator"];

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
        "machine.new_kit",
        "New Kit",
        "Home",
        "Kit",
        None,
        4,
        "Start a machine kit from a template.",
    ),
    (
        "machine.add_axis",
        "Add Axis",
        "Home",
        "Kinematics",
        None,
        4,
        "Add a linear or rotary axis to the chain.",
    ),
    (
        "machine.import_graphics",
        "Import Graphics",
        "Home",
        "Graphics",
        None,
        4,
        "Attach STEP, glTF or STL graphics to a component.",
    ),
    (
        "machine.tool_changer",
        "Tool Changer",
        "Home",
        "Tooling",
        None,
        4,
        "Define a carousel, chain or turret.",
    ),
    (
        "machine.jog",
        "Jog",
        "Home",
        "Test",
        None,
        4,
        "Move axes to test motion and limits.",
    ),
    (
        "machine.check_kit",
        "Check Kit",
        "Home",
        "Test",
        None,
        4,
        "Find unconnected components, missing limits and inverted axes.",
    ),
];

/// The Machine Kit Builder application.
#[derive(Debug, Default, Clone, Copy)]
pub struct MachineKitBuilder;

impl Application for MachineKitBuilder {
    fn id(&self) -> AppId {
        APP_ID
    }

    fn name(&self) -> &'static str {
        "Machine Kit Builder"
    }

    fn planned_stage(&self) -> u8 {
        4
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
            assert_eq!(k.namespace(), "machine");
        }
    }

    #[test]
    fn every_command_is_honestly_planned() {
        let cmds = MachineKitBuilder.commands();
        assert!(!cmds.is_empty());
        for c in &cmds {
            assert!(c.id.starts_with(concat!("machine", ".")), "{}", c.id);
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
        MachineKitBuilder.activate(&mut ui);
        assert_eq!(ui.ribbon_tabs, RIBBON_TABS);
        assert_eq!(ui.navigators, NAVIGATORS);
        MachineKitBuilder.deactivate(&mut ui);
        assert!(ui.ribbon_tabs.is_empty());
    }
}
