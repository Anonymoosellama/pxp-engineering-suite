// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-app-postgen: the Post Processor Generation application
//!
//! Layered post processors (machine, controller, capability and customisation layers; higher number wins) edited in a GUI and compiled to sandboxed WebAssembly.
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
//! - Intended update policy once implemented: Manual: posts rebuild when saved; output previews refresh on edit. Until then the
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
pub const APP_ID: AppId = AppId::from_static("pxp.postgen");

/// Object kinds whose schema this application owns.
pub static KINDS: [ObjectKind; 1] = [ObjectKind::from_static("postgen.post_ref")];

/// Ribbon tabs shown while this application is active, left to right.
pub const RIBBON_TABS: &[&str] = &["Home", "Application"];

/// Navigators shown in the resource bar while this application is active.
pub const NAVIGATORS: &[&str] = &["Layer Navigator"];

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
        "postgen.new_post",
        "New Post",
        "Home",
        "Post",
        None,
        4,
        "Start a post from a machine kit and a controller.",
    ),
    (
        "postgen.add_layer",
        "Add Layer",
        "Home",
        "Layers",
        None,
        4,
        "Add a capability or customisation layer.",
    ),
    (
        "postgen.edit_event",
        "Edit Event",
        "Home",
        "Events",
        None,
        4,
        "Edit the blocks an event produces.",
    ),
    (
        "postgen.preview",
        "Preview Output",
        "Home",
        "Output",
        None,
        4,
        "Preview NC output and see which layer produced each line.",
    ),
    (
        "postgen.build",
        "Build Post",
        "Home",
        "Output",
        None,
        4,
        "Compile the post to WebAssembly.",
    ),
];

/// The Post Processor Generation application.
#[derive(Debug, Default, Clone, Copy)]
pub struct PostGeneration;

impl Application for PostGeneration {
    fn id(&self) -> AppId {
        APP_ID
    }

    fn name(&self) -> &'static str {
        "Post Processor Generation"
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
            assert_eq!(k.namespace(), "postgen");
        }
    }

    #[test]
    fn every_command_is_honestly_planned() {
        let cmds = PostGeneration.commands();
        assert!(!cmds.is_empty());
        for c in &cmds {
            assert!(c.id.starts_with(concat!("postgen", ".")), "{}", c.id);
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
        PostGeneration.activate(&mut ui);
        assert_eq!(ui.ribbon_tabs, RIBBON_TABS);
        assert_eq!(ui.navigators, NAVIGATORS);
        PostGeneration.deactivate(&mut ui);
        assert!(ui.ribbon_tabs.is_empty());
    }
}
