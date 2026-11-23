<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# ADR-0002: UI toolkit

- **Status:** Decided by the prototypes: egui + `egui_dock` on wgpu.
  Becomes Accepted when the Stage 0 gate is approved.
- **Date:** 2026-11-20; prototypes and decision 2026-11-21

## Context

The master prompt asks for a choice between egui + `egui_dock`, Slint and
iced, judged on six criteria: docking, ribbon feasibility, text quality,
IME and accessibility, wgpu integration, and binary size. It also asks
for a one-day prototype ribbon and navigator in the top two candidates
before deciding.

## Slint's licence (checked first)

Slint offers three licences: GPL-3.0-only, the royalty-free licence 2.0,
and a commercial licence (see the `LICENSE.md` in Slint's repository).

- **GPL-3.0-only** would make every binary that links `pxp-ui` GPL-3.0.
  That defeats ADR-0001's aim of LGPL libraries that third parties can
  embed.
- **The royalty-free licence** requires an attribution widget or badge.
  It excludes embedded systems, which rules out machine-tool HMIs, a
  plausible Stage 3 target. It also does not permit distributing an
  application that exposes Slint's APIs. It is not an OSI licence, so
  distributions such as Debian and Flathub would treat the application as
  non-free.
- **The commercial licence** is paid and not open source.

None of these fits ADR-0001, so Slint dropped out and iced took the
second prototype slot.

## Prototypes (`prototypes/ui-egui`, `prototypes/ui-iced`)

Both prototypes are built from the same data: a ribbon from
`AppRegistry::commands()`, with planned commands greyed and given their
"Planned for Stage N." tooltip, a Part Navigator, application switching
by Ctrl+M, Ctrl+Shift+D and Ctrl+Alt+M through `AppRegistry::by_shortcut`,
cue and status lines, a stand-in viewport, and a scripted screenshot mode.
Measured on Linux x86-64 (2 vCPU) under Xvfb with Mesa lavapipe; eframe
0.37 / egui_dock 0.22 / wgpu 30, and iced 0.14.

| Criterion | egui + egui_dock | iced |
| --- | --- | --- |
| Docking | Tabbed panes you can drag, re-dock and float, out of the box | `pane_grid`: resizable split panes only; tabs and floating panes would have to be built |
| Ribbon from `CommandSpec` | Straightforward; immediate mode rebuilds the ribbon from data each frame | Straightforward; ribbon overflowed the window, so a scroll container must be added |
| Text quality | Good at 12–14 px. The default fonts lack ✔ and ⚠, so status marks are painted instead | Good; system fonts through cosmic-text, so ✔ and ⚠ render |
| IME | winit IME events, forwarded by egui | winit IME events, forwarded by iced |
| Accessibility | AccessKit integration (eframe `accesskit` feature, enabled) | No AccessKit dependency in iced 0.14 |
| wgpu integration | eframe exposes its `wgpu` device. `pxp-render` draws into its own texture and egui shows it (`register_native_texture`). Done in the real shell | `shader` widget or a custom primitive; feasible, not prototyped |
| Release binary, stripped | 18.7 MB | 16.7 MB |
| Clean release build (−j1, 2 vCPU, shared machine) | about 11 min | about 10 min |
| Prototype size | 267 lines | 227 lines |
| Crates in the lock file | 413 | 435 |
| Builds on first try against the documented API | No: three fixes for eframe 0.37's new `App::ui` and screenshot API | Yes |

## Decision

**egui + `egui_dock`, on eframe's wgpu backend.** Docking decides it. The
NX resource bar and floating navigators need tabbed, undockable panes, and
`egui_dock` provides them while iced would need them built. AccessKit is
also integrated in egui and absent from iced 0.14. Sharing the wgpu device
with `pxp-render` is already working in the real shell (`pxp-ui`, `pxp-app`).
iced's advantages are system-font glyph coverage and a slightly smaller
binary. Neither outweighs docking and accessibility, and the glyph gap is
closed by painting status marks or adding a font.

## Consequences

- `pxp-ui` and `pxp-app` depend on `eframe` and `egui_dock` (MIT OR
  Apache-2.0) and `wgpu` 30 (MIT OR Apache-2.0). wgpu's version follows
  eframe's, so the shell and viewport share one device.
- Under them, `winit` and ten smaller crates are Apache-2.0 only, so the
  desktop binary `pxp` is conveyed under LGPL-3.0. They are registered in
  ADR-0001 and `deny.toml`. iced sits on winit too, so this did not
  separate the candidates.
- egui changes its API between minor versions (0.37 changed `App::update`
  to `App::ui`, panels and screenshots). Upgrades are deliberate and come
  with screenshot checks (`pxp --screenshot`).
- Fonts: add a symbol font in Stage 1 so the navigators can use
  ✔ ⚠ glyphs like NX.
- The prototypes stay in `prototypes/` as evidence. They are their own
  workspaces, so CI does not build them.
