# PXP Engineering Suite

An open-source, Rust-native CAD, Drafting, CAM and CAE system whose workflow
follows Siemens NX's interaction grammar, with every application's data in
one `.pxp` part file. Working name; licensed **LGPL-2.1-or-later**
(ADR-0001).

> **Status: pre-alpha, Stage 0 (Foundations), session 2.** There is no
> graphical application yet. What exists is listed below and by
> `pxp-cli status`. Nothing here claims parity with any commercial system.

## What works today

- **Document core** (`pxp-doc`): objects with stable UUIDv7 IDs, NX-style
  unit-aware expressions, a cross-application dependency graph, panic-safe
  all-or-nothing transactions, unlimited undo/redo under a memory cap, change
  notifications, immutable snapshots, and a parallel, deterministic,
  all-or-nothing recompute.
- **Units and maths** (`pxp-units`, `pxp-math`): typed quantities in a
  coherent mm-tonne-second system, mm/inch display, transforms, coordinate
  systems, tolerances, a stable cross-platform hash.
- **Contracts**: the geometry kernel trait with `OpResult`/`TopoHistory` and
  a panic-containing `KernelHost` (`pxp-kernel`); the `Application` trait and
  command registry (`pxp-appkit`); all six applications registered, with
  their commands shown as "Planned for Stage N."
- **Part files** (`pxp-format`): `.pxp` as a SQLite database with atomic
  saves, three rolling backups, a crash-recovery journal, an OS lock,
  in-memory schema migrations and byte-for-byte preservation of unknown
  object kinds.
- **Geometry** (`pxp-kernel-native`): SolidCraft's pure-Rust B-rep kernel
  (built on SolidCore) behind the kernel trait: block, cylinder, cone,
  sphere, booleans, a complete topology history for every operation,
  mass properties, a cross-platform geometry hash, exact B-rep storage.
- **Modeling primitives** (`pxp-app-modeling`): datum CSYS, block,
  cylinder, cone and sphere features whose parameters are expressions,
  regenerated through the kernel and saved with their exact geometry.
- **Kernel bake-off harness** (`pxp_kernel::bakeoff`): synthetic stress
  cases and SolidCraft's analytic kernel cases, graded against hand
  calculations.
- **CLI** (`pxp-cli`): `part` (new, block, cylinder, cone, sphere, info,
  verify), `bakeoff`, `apps`, `commands`, `find`, `shortcut`, `eval`,
  `status`, `version`.

## Build and try

```sh
cargo test --workspace
cargo run -p pxp-cli -- status
cargo run -p pxp-cli -- eval "7850[kg/m^3] * (100mm)^3" --decimals 3   # 7.850 kg
cargo run -p pxp-cli -- find blend
cargo run -p pxp-cli -- part new bracket.pxp --units mm
cargo run -p pxp-cli -- part block bracket.pxp --size 10 20 30
cargo run -p pxp-cli -- part info bracket.pxp     # hash, volume, area from the stored B-rep
cargo run -p pxp-cli -- part verify bracket.pxp   # reopen, re-import, regenerate: identical
cargo run -p pxp-cli -- bakeoff --corpus synthetic
python3 tools/check_repo.py      # layering, unsafe, docs and SPDX policy
```

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.97.0). The first
build compiles SolidCore (about 40 MB of Rust) and needs roughly 6 GB of
memory; on a small machine build with `-j1`.

Binaries that link `pxp-kernel-native` (today `pxp-cli`) are conveyed under
LGPL-3.0, because SolidCore is Apache-2.0 only (ADR-0001).

## Layout

- `crates/`: one crate per responsibility, layered foundation → document →
  geometry and format → services → applications → shell
  (`docs/design/architecture.md`).
- `docs/MASTER_PROMPT.md`: the staged build specification.
- `docs/STATUS.md`: what is done, in progress, blocked, and decided.
- `docs/adr/`: architecture decision records.
- `docs/design/`: design notes with diagrams for each logic system.
- `tools/`: repository policy checks.

## Reference projects

Builds on ideas from SolidCraft (MIT OR Apache-2.0, taken as MIT), SolidCore
(Apache-2.0), FreeCAD (LGPL, studied and reimplemented) and others; see
ADR-0001 for what may enter the tree and how. PrusaSlicer (AGPL-3.0) is a
behaviour reference only. Siemens, NX and Mastercam are trademarks of their
owners; this project copies workflow, not their icons, text or branding.
