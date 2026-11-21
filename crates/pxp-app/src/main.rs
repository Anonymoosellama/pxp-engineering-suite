// SPDX-License-Identifier: LGPL-2.1-or-later
//! # pxp-app: the desktop application binary (`pxp`)
//!
//! **Status:** the graphical shell is not implemented yet. It needs the UI
//! toolkit chosen in ADR-0002 (egui + `egui_dock` versus Slint, decided by a
//! prototype) and wgpu, both of which need crates.io access this session did
//! not have. Until sub-stage 0H the binary assembles the application registry,
//! proving the wiring, reports what it found, and says plainly that the
//! window is not available. It does not open an empty window that looks like
//! a product.
//!
//! ## Responsibility (once implemented)
//!
//! Process start-up: choose and host the geometry kernel backend (the only
//! layer allowed to name one), register the built-in applications, start the
//! document thread, run crash recovery, then hand control to `pxp-ui`.
//!
//! ## Must never
//!
//! - Contain domain logic: it wires crates together and nothing more.
#![forbid(unsafe_code)]

use pxp_appkit::{AppRegistry, Application};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut registry = AppRegistry::new();
    let apps: [Box<dyn Application>; 6] = [
        Box::new(pxp_app_modeling::Modeling::new()),
        Box::new(pxp_app_drafting::Drafting),
        Box::new(pxp_app_manufacturing::Manufacturing),
        Box::new(pxp_app_postgen::PostGeneration),
        Box::new(pxp_app_machine_kit::MachineKitBuilder),
        Box::new(pxp_app_cae::Cae),
    ];
    for app in apps {
        if let Err(e) = registry.register(app) {
            eprintln!("pxp: application registry error: {e}");
            return ExitCode::FAILURE;
        }
    }
    println!(
        "PXP Engineering Suite {} (pre-alpha, Stage 0)",
        env!("CARGO_PKG_VERSION")
    );
    println!(
        "Registered {} applications with {} commands.",
        registry.apps().count(),
        registry.commands().len()
    );
    println!(
        "The graphical shell is not implemented yet (sub-stage 0H; pxp-ui planned for Stage {}).",
        pxp_ui::PLANNED_STAGE
    );
    println!(
        "Use `pxp-cli` to explore what exists: pxp-cli status | apps | commands | find | eval | part | bakeoff."
    );
    ExitCode::from(2)
}
