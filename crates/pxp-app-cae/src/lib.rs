// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-app-cae: the CAE application (FEA, topology optimization, scan to CAD)
//!
//! Simulation Navigator with FEM, Simulation, Optimization and Scan objects: linear static, modal and harmonic FEA; SIMP and BESO topology optimization; point-cloud and mesh to surfaces and solids.
//!
//! **Status:** registered with the application registry so its ribbon
//! tabs, navigators and commands appear (greyed, with the tooltip "Planned
//! for Stage N.") and switching to it works with the part still open. Its
//! first real commands arrive in Stage 5; nothing here is a stub
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
//! - Intended update policy once implemented: Manual: meshes, results and studies are marked out of date and never re-run silently. Until then the
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
pub const APP_ID: AppId = AppId::from_static("pxp.cae");

/// Object kinds whose schema this application owns.
pub static KINDS: [ObjectKind; 4] = [
    ObjectKind::from_static("cae.fem"),
    ObjectKind::from_static("cae.simulation"),
    ObjectKind::from_static("cae.optimization"),
    ObjectKind::from_static("cae.scan"),
];

/// Ribbon tabs shown while this application is active, left to right.
pub const RIBBON_TABS: &[&str] = &["Home", "Results", "Optimization", "Scan", "Application"];

/// Navigators shown in the resource bar while this application is active.
pub const NAVIGATORS: &[&str] = &["Simulation Navigator"];

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
        "cae.new_fem",
        "New FEM",
        "Home",
        "Model",
        None,
        5,
        "Create an idealised model for meshing.",
    ),
    (
        "cae.mesh",
        "Mesh",
        "Home",
        "Model",
        None,
        5,
        "Mesh bodies with tetrahedral or shell elements.",
    ),
    (
        "cae.material",
        "Assign Material",
        "Home",
        "Model",
        None,
        5,
        "Assign a material to bodies.",
    ),
    (
        "cae.load",
        "Load",
        "Home",
        "Simulation",
        None,
        5,
        "Apply forces, pressures, moments and accelerations.",
    ),
    (
        "cae.constraint",
        "Constraint",
        "Home",
        "Simulation",
        None,
        5,
        "Fix, pin or support geometry.",
    ),
    (
        "cae.solve",
        "Solve",
        "Home",
        "Simulation",
        None,
        5,
        "Run a static, modal or harmonic solution.",
    ),
    (
        "cae.results",
        "Results",
        "Results",
        "Post-processing",
        None,
        5,
        "Show contour plots, deformed shapes and probes.",
    ),
    (
        "cae.topology_optimization",
        "Topology Optimization",
        "Optimization",
        "Study",
        None,
        5,
        "Optimise a design space for stiffness or mass.",
    ),
    (
        "cae.import_scan",
        "Import Scan",
        "Scan",
        "Data",
        None,
        5,
        "Import point clouds or scan meshes.",
    ),
    (
        "cae.register_scans",
        "Register",
        "Scan",
        "Data",
        None,
        5,
        "Align scans to each other or to CAD.",
    ),
    (
        "cae.fit_surfaces",
        "Fit Surfaces",
        "Scan",
        "Reconstruction",
        None,
        5,
        "Fit primitives and NURBS surfaces to scan regions.",
    ),
    (
        "cae.deviation",
        "Deviation",
        "Scan",
        "Inspection",
        None,
        5,
        "Colour-map deviation between scan and CAD.",
    ),
];

/// The CAE application.
#[derive(Debug, Default, Clone, Copy)]
pub struct Cae;

impl Application for Cae {
    fn id(&self) -> AppId {
        APP_ID
    }

    fn name(&self) -> &'static str {
        "CAE"
    }

    fn planned_stage(&self) -> u8 {
        5
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
            assert_eq!(k.namespace(), "cae");
        }
    }

    #[test]
    fn every_command_is_honestly_planned() {
        let cmds = Cae.commands();
        assert!(!cmds.is_empty());
        for c in &cmds {
            assert!(c.id.starts_with(concat!("cae", ".")), "{}", c.id);
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
        Cae.activate(&mut ui);
        assert_eq!(ui.ribbon_tabs, RIBBON_TABS);
        assert_eq!(ui.navigators, NAVIGATORS);
        Cae.deactivate(&mut ui);
        assert!(ui.ribbon_tabs.is_empty());
    }
}
