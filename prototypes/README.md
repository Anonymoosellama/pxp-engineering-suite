<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# UI toolkit prototypes (ADR-0002)

The master prompt asks for a one-day prototype ribbon and navigator in the
top two UI toolkits before ADR-0002 is decided. These two programs are that
prototype, built on the same data: the ribbon comes from
`pxp_appkit::AppRegistry::commands()` (six applications, planned commands
greyed with their "Planned for Stage N" tooltip), with a Part Navigator,
application switching by Ctrl+M / Ctrl+Shift+D / Ctrl+Alt+M, a cue line and a
status line.

| Prototype | Toolkit | Docking |
| --- | --- | --- |
| `ui-egui` | egui + eframe (wgpu) + `egui_dock` | dockable, tabbed, undockable panes |
| `ui-iced` | iced (wgpu) | `pane_grid` split panes (no tab docking) |

Slint did not get a prototype: its licence options (GPL-3.0-only, a
royalty-free licence with attribution and an API-exposure restriction, or a
commercial licence) do not fit ADR-0001; see ADR-0002.

Each prototype is its own Cargo workspace (outside the main one, so CI does
not build them). Build and screenshot:

```sh
cargo build --release --manifest-path prototypes/ui-egui/Cargo.toml
PXP_PROTO_SCREENSHOT=egui.png xvfb-run -s "-screen 0 1600x900x24" prototypes/ui-egui/target/release/pxp-proto-egui
```

`PXP_PROTO_SCREENSHOT` makes the prototype switch to the Modeling
application, open the Planned tooltip once, save a screenshot and exit; the
measured results are recorded in `docs/adr/ADR-0002-ui-toolkit.md`.
