# ADR-0002: UI toolkit

- **Status:** Proposed. The decision waits for the two prototypes below.
- **Date:** 2026-11-20

## Context

The master prompt requires a judgement between egui + `egui_dock`, Slint and
iced on six criteria: docking, ribbon feasibility, text quality, IME and
accessibility, wgpu integration, and binary size. A one-day prototype ribbon
and navigator must be built in the top two candidates before deciding.

Session 1 had no crates.io access, so no prototype could be compiled.

## Candidates and first assessment (desk study, not yet measured)

| Criterion | egui + egui_dock | Slint | iced |
| --- | --- | --- | --- |
| Docking (dockable, undockable resource bar) | egui_dock provides tabbed docking | Must be built | Must be built |
| Ribbon feasibility (data-driven from `CommandSpec`) | Immediate mode makes a data-driven ribbon simple | Declarative `.slint` markup; dynamic models possible | Elm-style; possible |
| Text quality | Good; check hinting at small sizes | Good | Good |
| IME and accessibility | AccessKit integration; IME to verify per OS | AccessKit; IME support | Improving; verify |
| wgpu integration (shared device with `pxp-render`) | Native (egui-wgpu) | Possible through a renderer integration | Native (iced_wgpu) |
| Binary size | Small | Small | Moderate |
| Licence | MIT OR Apache-2.0 | GPLv3, royalty-free or commercial; **must be checked against ADR-0001 (LGPL-2.1-or-later)** before prototyping | MIT |

Slint's licence options must be confirmed first: if no option is compatible
with an LGPL-2.1-or-later application that third parties may embed, Slint
drops out and iced takes its prototype slot.

## Plan

1. Confirm Slint's current licence terms against ADR-0001.
2. Prototype in egui + `egui_dock` and the second candidate: a ribbon built
   from `AppRegistry::commands()` (greyed planned commands with tooltips),
   a dockable Part Navigator, application switching by Ctrl+M / Ctrl+Shift+D
   / Ctrl+Alt+M, a dialog with selection steps and MB2 = OK, and a wgpu
   viewport sharing the device.
3. Measure each criterion and record the numbers here.

## Decision

Pending.
