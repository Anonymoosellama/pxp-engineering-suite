// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-app-modeling: the Modeling application (assemblies included)
//!
//! Sketches, curves, datums, parametric solid and sheet features, synchronous modeling, holes and threads, analysis, assemblies, display and layers.
//!
//! **Status (Stage 0):** the datum CSYS and the four primitives (block,
//! cylinder, cone, sphere) are real features: created in a transaction
//! ([`primitives`]), regenerated through the kernel trait with an immediate
//! update policy, and stored with their exact B-rep and geometry hash. They
//! exist to prove the pipeline end to end; every other command is listed
//! greyed with the tooltip "Planned for Stage N." until it works. Nothing
//! here is a stub that pretends to work: without a connected kernel the
//! primitives report "no geometry kernel is connected" and stay greyed.
//!
//! ## Responsibility
//!
//! Owns the schema of the object kinds in [`KINDS`] and contributes the
//! commands in its ribbon. Like every application it is an editor over the
//! one shared document: switching to it copies or converts nothing.
//!
//! ## Data flow
//!
//! Registered by the shell into `pxp_appkit::AppRegistry`, with the
//! `pxp_kernel::KernelHost` the shell chose ([`Modeling::with_kernel`]);
//! receives change sets through `on_upstream_change`; regenerates primitives
//! through `Application::recompute`, which the registry calls from the
//! document's parallel recompute.
//!
//! ```text
//! create_block (Tx) ─► expressions p0..p2 + modeling.block (out of date)
//! Document::recompute ─► AppRegistry ─► Modeling::recompute ─► KernelHost::op(block)
//!                    ◄── geometry_hash, kernel, payload = stored exact B-rep
//! ```
//!
//! ## Invariants
//!
//! - [`APP_ID`] and every kind name are persisted in files and never change.
//! - Primitives and the datum CSYS have the immediate update policy
//!   (master prompt associativity rules); kinds not implemented yet keep
//!   the safe default (manual, refuse to regenerate).
//! - Recompute is a pure function of the snapshot: kernel shapes are
//!   released before it returns.
//!
//! ## Must never
//!
//! - Depend on another application crate (shared logic lives in domain
//!   libraries).
//! - Copy Siemens icons, help text or trademarks.
#![forbid(unsafe_code)]

pub mod primitives;

use pxp_appkit::{
    AppId, Application, ChangeCtx, ChangeSet, CommandSpec, CommandStatus, ObjectKind,
    RecomputeOutput, RibbonPlacement, UiContext, UpdatePolicy,
};
use pxp_doc::{Object, Snapshot};
use pxp_kernel::KernelHost;
use std::sync::Arc;

/// Stable application ID persisted in `.pxp` files.
pub const APP_ID: AppId = AppId::from_static("pxp.modeling");

/// Object kinds whose schema this application owns.
pub static KINDS: [ObjectKind; 18] = [
    primitives::DATUM_CSYS,
    primitives::BLOCK,
    primitives::CYLINDER,
    primitives::CONE,
    primitives::SPHERE,
    ObjectKind::from_static("modeling.sketch"),
    ObjectKind::from_static("modeling.extrude"),
    ObjectKind::from_static("modeling.revolve"),
    ObjectKind::from_static("modeling.hole"),
    ObjectKind::from_static("modeling.thread"),
    ObjectKind::from_static("modeling.edge_blend"),
    ObjectKind::from_static("modeling.chamfer"),
    ObjectKind::from_static("modeling.boolean"),
    ObjectKind::from_static("modeling.shell"),
    ObjectKind::from_static("modeling.draft"),
    ObjectKind::from_static("modeling.pattern"),
    ObjectKind::from_static("modeling.mirror"),
    ObjectKind::from_static("modeling.component"),
];

/// Ribbon tabs shown while this application is active, left to right.
pub const RIBBON_TABS: &[&str] = &[
    "Home",
    "Curve",
    "Surface",
    "Assemblies",
    "Analysis",
    "View",
    "Selection",
    "Render",
    "Tools",
    "Application",
];

/// Navigators shown in the resource bar while this application is active.
pub const NAVIGATORS: &[&str] = &[
    "Part Navigator",
    "Assembly Navigator",
    "Constraint Navigator",
    "Reuse Library",
    "History",
    "Roles",
];

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
        "modeling.datum_csys",
        "Datum CSYS",
        "Home",
        "Feature",
        None,
        0,
        "Create a datum coordinate system.",
    ),
    (
        "modeling.block",
        "Block",
        "Home",
        "Feature",
        None,
        0,
        "Create a rectangular block primitive.",
    ),
    (
        "modeling.cylinder",
        "Cylinder",
        "Home",
        "Feature",
        None,
        0,
        "Create a cylinder primitive.",
    ),
    (
        "modeling.cone",
        "Cone",
        "Home",
        "Feature",
        None,
        0,
        "Create a cone or truncated-cone primitive.",
    ),
    (
        "modeling.sphere",
        "Sphere",
        "Home",
        "Feature",
        None,
        0,
        "Create a sphere primitive.",
    ),
    (
        "modeling.expressions",
        "Expressions",
        "Tools",
        "Utilities",
        Some("Ctrl+E"),
        0,
        "Create and edit named, unit-aware expressions.",
    ),
    (
        "modeling.sketch",
        "Sketch",
        "Home",
        "Direct Sketch",
        None,
        1,
        "Sketch constrained curves on a plane or path.",
    ),
    (
        "modeling.extrude",
        "Extrude",
        "Home",
        "Feature",
        Some("X"),
        1,
        "Extrude a section to create a solid or sheet.",
    ),
    (
        "modeling.revolve",
        "Revolve",
        "Home",
        "Feature",
        None,
        1,
        "Revolve a section about an axis.",
    ),
    (
        "modeling.hole",
        "Hole",
        "Home",
        "Feature",
        None,
        1,
        "Create standard simple, counterbored, countersunk or threaded holes.",
    ),
    (
        "modeling.thread",
        "Thread",
        "Home",
        "Feature",
        None,
        1,
        "Add symbolic or modeled threads.",
    ),
    (
        "modeling.edge_blend",
        "Edge Blend",
        "Home",
        "Feature",
        None,
        1,
        "Round edges with constant or variable radius.",
    ),
    (
        "modeling.chamfer",
        "Chamfer",
        "Home",
        "Feature",
        None,
        1,
        "Bevel edges.",
    ),
    (
        "modeling.unite",
        "Unite",
        "Home",
        "Feature",
        None,
        1,
        "Combine bodies.",
    ),
    (
        "modeling.subtract",
        "Subtract",
        "Home",
        "Feature",
        None,
        1,
        "Remove tool bodies from a target.",
    ),
    (
        "modeling.intersect",
        "Intersect",
        "Home",
        "Feature",
        None,
        1,
        "Keep the common volume of bodies.",
    ),
    (
        "modeling.shell",
        "Shell",
        "Home",
        "Feature",
        None,
        1,
        "Hollow a solid with a wall thickness.",
    ),
    (
        "modeling.draft",
        "Draft",
        "Home",
        "Feature",
        None,
        1,
        "Taper faces for moulding.",
    ),
    (
        "modeling.pattern_feature",
        "Pattern Feature",
        "Home",
        "Feature",
        None,
        1,
        "Repeat features linearly, circularly or along a curve.",
    ),
    (
        "modeling.mirror_feature",
        "Mirror Feature",
        "Home",
        "Feature",
        None,
        1,
        "Mirror features across a plane.",
    ),
    (
        "modeling.move_face",
        "Move Face",
        "Home",
        "Synchronous Modeling",
        None,
        1,
        "Move faces of a body without history.",
    ),
    (
        "modeling.studio_spline",
        "Studio Spline",
        "Curve",
        "Curve",
        None,
        1,
        "Create a spline through points or poles.",
    ),
    (
        "modeling.through_curves",
        "Through Curves",
        "Surface",
        "Surface",
        None,
        1,
        "Loft a body through section curves.",
    ),
    (
        "modeling.add_component",
        "Add Component",
        "Assemblies",
        "Component",
        None,
        1,
        "Add a part as an assembly component.",
    ),
    (
        "modeling.assembly_constraints",
        "Assembly Constraints",
        "Assemblies",
        "Component Position",
        None,
        1,
        "Position components with constraints.",
    ),
    (
        "modeling.measure",
        "Measure",
        "Analysis",
        "Measure",
        None,
        1,
        "Measure distance, angle, area, volume and mass.",
    ),
    (
        "modeling.edit_object_display",
        "Edit Object Display",
        "View",
        "Visualization",
        Some("Ctrl+J"),
        1,
        "Change colour, translucency, layer and line style of objects or faces.",
    ),
    (
        "modeling.layer_settings",
        "Layer Settings",
        "View",
        "Layer",
        Some("Ctrl+L"),
        1,
        "Set the work layer and layer visibility.",
    ),
];

/// Commands that work once a geometry kernel is connected.
const PRIMITIVE_COMMANDS: [&str; 5] = [
    "modeling.datum_csys",
    "modeling.block",
    "modeling.cylinder",
    "modeling.cone",
    "modeling.sphere",
];

/// The Modeling application.
#[derive(Debug, Default, Clone)]
pub struct Modeling {
    kernel: Option<Arc<KernelHost>>,
}

impl Modeling {
    /// Modeling without a geometry kernel: everything is listed, nothing
    /// that needs geometry is available.
    #[must_use]
    pub const fn new() -> Self {
        Self { kernel: None }
    }

    /// Modeling regenerating geometry through `kernel` (the shell chooses
    /// the backend; this crate never names one).
    #[must_use]
    pub fn with_kernel(kernel: Arc<KernelHost>) -> Self {
        Self {
            kernel: Some(kernel),
        }
    }

    /// The connected kernel, if any.
    #[must_use]
    pub fn kernel(&self) -> Option<&KernelHost> {
        self.kernel.as_deref()
    }
}

impl Application for Modeling {
    fn id(&self) -> AppId {
        APP_ID
    }

    fn name(&self) -> &'static str {
        "Modeling"
    }

    fn planned_stage(&self) -> u8 {
        0
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
                    status: if self.kernel.is_some() && PRIMITIVE_COMMANDS.contains(&id) {
                        CommandStatus::Available
                    } else {
                        CommandStatus::Planned { stage }
                    },
                },
            )
            .collect()
    }

    fn update_policy(&self, kind: &ObjectKind) -> UpdatePolicy {
        if primitives::is_primitive_kind(kind) {
            UpdatePolicy::Immediate
        } else {
            UpdatePolicy::Manual
        }
    }

    fn recompute(&self, inputs: &Snapshot, object: &Object) -> Result<RecomputeOutput, String> {
        if primitives::is_primitive_kind(&object.kind) {
            primitives::recompute(self.kernel(), inputs, object)
        } else {
            Err(format!(
                "'{}' is planned for a later stage and cannot be regenerated yet",
                object.kind
            ))
        }
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
            assert_eq!(k.namespace(), "modeling");
        }
    }

    #[test]
    fn every_command_is_honestly_planned_without_a_kernel() {
        let cmds = Modeling::new().commands();
        assert!(!cmds.is_empty());
        for c in &cmds {
            assert!(c.id.starts_with(concat!("modeling", ".")), "{}", c.id);
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
        Modeling::new().activate(&mut ui);
        assert_eq!(ui.ribbon_tabs, RIBBON_TABS);
        assert_eq!(ui.navigators, NAVIGATORS);
        Modeling::new().deactivate(&mut ui);
        assert!(ui.ribbon_tabs.is_empty());
    }
}
