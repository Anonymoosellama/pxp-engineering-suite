// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-app-drafting: the Drafting application
//!
//! Multi-sheet drawings with associative views, dimensions, GD&T, annotations and parts lists, inside the part or in a master-model part.
//!
//! **Status:** registered with the application registry so its ribbon
//! tabs, navigators and commands appear (greyed, with the tooltip "Planned
//! for Stage N.") and switching to it works with the part still open. Its
//! first real commands arrive in Stage 2; nothing here is a stub
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
//! - Intended update policy once implemented: Immediate by default (views and dimensions update automatically), switchable to on-demand for large drawings. Until then the
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
pub const APP_ID: AppId = AppId::from_static("pxp.drafting");

/// Object kinds whose schema this application owns.
pub static KINDS: [ObjectKind; 5] = [
    ObjectKind::from_static("drafting.sheet"),
    ObjectKind::from_static("drafting.view"),
    ObjectKind::from_static("drafting.dimension"),
    ObjectKind::from_static("drafting.annotation"),
    ObjectKind::from_static("drafting.parts_list"),
];

/// Ribbon tabs shown while this application is active, left to right.
pub const RIBBON_TABS: &[&str] = &["Home", "Annotation", "Tools", "View", "Application"];

/// Navigators shown in the resource bar while this application is active.
pub const NAVIGATORS: &[&str] = &["Part Navigator"];

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
        "drafting.new_sheet",
        "New Sheet",
        "Home",
        "Sheet",
        None,
        2,
        "Add a drawing sheet from a template.",
    ),
    (
        "drafting.base_view",
        "Base View",
        "Home",
        "View",
        None,
        2,
        "Place a view of the model at a chosen scale.",
    ),
    (
        "drafting.projected_view",
        "Projected View",
        "Home",
        "View",
        None,
        2,
        "Project an orthographic view from a parent view.",
    ),
    (
        "drafting.section_view",
        "Section View",
        "Home",
        "View",
        None,
        2,
        "Create full, half, offset or aligned section views.",
    ),
    (
        "drafting.detail_view",
        "Detail View",
        "Home",
        "View",
        None,
        2,
        "Magnify a region of a view.",
    ),
    (
        "drafting.rapid_dimension",
        "Rapid Dimension",
        "Home",
        "Dimension",
        None,
        2,
        "Infer and place a dimension.",
    ),
    (
        "drafting.ordinate_dimension",
        "Ordinate Dimension",
        "Home",
        "Dimension",
        None,
        2,
        "Dimension from an ordinate origin and baselines.",
    ),
    (
        "drafting.feature_control_frame",
        "Feature Control Frame",
        "Annotation",
        "GD&T",
        None,
        2,
        "Add a geometric tolerance frame.",
    ),
    (
        "drafting.datum_feature_symbol",
        "Datum Feature Symbol",
        "Annotation",
        "GD&T",
        None,
        2,
        "Identify a datum feature.",
    ),
    (
        "drafting.note",
        "Note",
        "Annotation",
        "Annotation",
        None,
        2,
        "Add text with live attribute references.",
    ),
    (
        "drafting.parts_list",
        "Parts List",
        "Home",
        "Table",
        None,
        2,
        "Insert a bill of materials from the assembly.",
    ),
    (
        "drafting.auto_balloon",
        "Auto Balloon",
        "Home",
        "Table",
        None,
        2,
        "Balloon components and link them to parts-list rows.",
    ),
];

/// The Drafting application.
#[derive(Debug, Default, Clone, Copy)]
pub struct Drafting;

impl Application for Drafting {
    fn id(&self) -> AppId {
        APP_ID
    }

    fn name(&self) -> &'static str {
        "Drafting"
    }

    fn planned_stage(&self) -> u8 {
        2
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
            assert_eq!(k.namespace(), "drafting");
        }
    }

    #[test]
    fn every_command_is_honestly_planned() {
        let cmds = Drafting.commands();
        assert!(!cmds.is_empty());
        for c in &cmds {
            assert!(c.id.starts_with(concat!("drafting", ".")), "{}", c.id);
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
        Drafting.activate(&mut ui);
        assert_eq!(ui.ribbon_tabs, RIBBON_TABS);
        assert_eq!(ui.navigators, NAVIGATORS);
        Drafting.deactivate(&mut ui);
        assert!(ui.ribbon_tabs.is_empty());
    }
}
