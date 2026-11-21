# ADR-0001: Project licence — LGPL-2.1-or-later

- **Status:** Accepted
- **Date:** 2026-11-20
- **Decided by:** the user (asked on 2026-11-20, as the master prompt requires)
- **Supersedes:** none

## Context

The master prompt requires the licence to be chosen before any code is
written, because licence mistakes are expensive to undo once code is mixed.
The two options it offered were:

1. AGPL-3.0-or-later, which lets PrusaSlicer (AGPL-3.0) code be ported.
2. LGPL-2.1-or-later or MPL-2.0, which allow wider reuse and embedding but
   keep all PrusaSlicer code out of the tree.

The user chose **LGPL-2.1-or-later**.

## Decision

- Every source file in this repository carries the SPDX header
  `SPDX-License-Identifier: LGPL-2.1-or-later` (Rust: `// SPDX-...` on line 1;
  TOML, YAML, shell and Python: `# SPDX-...`; Markdown docs are exempt).
- `LICENSE` holds the verbatim LGPL-2.1 text. The "or later" clause is
  load-bearing (see Apache-2.0 below) and must never be dropped.
- `deny.toml` enforces the allowed-licence list in CI (see "Enforcement").

## Consequences for each reference project

| Project | Its licence | Can its code enter the tree? | How |
| --- | --- | --- | --- |
| SolidCraft | MIT OR Apache-2.0 (brand separate) | Yes | Taken under **MIT**, which is compatible with LGPL-2.1. Keep its copyright notice in each vendored file. Never use its name, logo, icons or UI design (LICENSE-BRAND). |
| SolidCore | Apache-2.0 only | Yes, with a caveat | The FSF considers Apache-2.0 compatible with GPLv3 but **not** with GPLv2-family licences, because of its patent-termination and indemnity clauses. Source files stay LGPL-2.1-or-later. A **binary** that links SolidCore is conveyed under **LGPL-3.0** (allowed because our licence is "or later"). Alternative: ask the SolidCore author for an MIT dual licence. |
| wasmtime | Apache-2.0 WITH LLVM-exception | Yes | The LLVM exception explicitly lets a GPLv2-family combination waive the conflicting Apache sections, so no LGPL-3.0 step is needed. |
| FreeCAD 1.1 | LGPL-2.0-or-later | Yes | Compatible. Line-by-line translations live only in crates whose name ends in `-fc` and whose header names the FreeCAD source file and commit. |
| OCCT | LGPL-2.1 (+ OCCT exception) | Yes, as a separate process | Runs in the out-of-process kernel worker. When linked, the relinking obligation is met because full source is shipped. |
| PrusaSlicer | AGPL-3.0 | **No** | Behaviour reference only. The Stage 3 slicer is written from published algorithms and observed behaviour. Never copy, translate or paraphrase PrusaSlicer source; cite papers in `docs/CITATIONS.md`. PrusaSlicer may be run as an external program to compare outputs in tests. |
| beso | LGPL-3.0 | Only in an LGPL-3.0 crate | Default: method reference only. If code is ever ported, it goes in a crate marked `LGPL-3.0-or-later`, and binaries containing it are conveyed under LGPL-3.0. |
| Netgen | LGPL-2.1 | Yes, as a separate process | Behind the `Mesher` trait in the kernel worker. |
| CalculiX, Gmsh | GPL | **No linking** | External processes only, for cross-validation. |
| faer, e57, PoissonRecon, Open3D | MIT | Yes | Compatible. |
| egui, egui_dock, wgpu, rayon, rusqlite | MIT OR Apache-2.0 / MIT | Yes | Taken under MIT. SQLite itself is public domain. |
| MPL-2.0 crates (for example persistent collections) | MPL-2.0 | Yes | MPL-2.0 is compatible with (L)GPL through its secondary-licence clause. |

### Static linking

Rust links statically by default. LGPL-2.1 §6 requires that a user can relink
a combined work against a modified library. Our own distributed builds meet
this because every release ships complete source. A **third party** that
embeds PXP crates in closed-source software must let its users relink, for
example by supplying object files or its own source. This is the trade-off the
user accepted in exchange for wider reuse than AGPL allows.

## Enforcement

`deny.toml` allows only these licences globally: MIT, BSD-2-Clause,
BSD-3-Clause, ISC, Zlib, 0BSD, BSL-1.0, CC0-1.0, Unicode-3.0,
Unicode-DFS-2016, MPL-2.0, `Apache-2.0 WITH LLVM-exception`,
LGPL-2.0-or-later, LGPL-2.1-or-later.

**Plain `Apache-2.0` is deliberately not in the global allow list.** Crates
offered as "MIT OR Apache-2.0" still pass, because MIT is allowed. A crate that
is Apache-2.0 *only* fails CI until it is added to the per-crate `exceptions`
list. That forces a conscious decision, recorded in this ADR, each time a
dependency moves our binaries to LGPL-3.0 conveyance. GPL and AGPL crates are
always denied.

Registered Apache-2.0-only exceptions:

| Crates | Registered | Why | Consequence |
| --- | --- | --- | --- |
| `solidcore`, `solidcore-convert`, `solidcore-ops`, `solidcore-record`, `solidcore-step`, `solidcore-tess` (from SolidCraft's repository, pinned commit `2091b88`) | 2026-11-21, session 2 | The B-rep kernel under SolidCraft's `solidcraft-kernel`, which `pxp-kernel-native` wraps for the Stage 0 kernel bake-off (ADR-0004). | Every binary that links `pxp-kernel-native` (today `pxp-cli`; later the desktop application) is conveyed under **LGPL-3.0**. Our source files stay `LGPL-2.1-or-later`. If OCCT wins the bake-off and the native backend is dropped from shipped binaries, this exception and the LGPL-3.0 conveyance can be removed. |

## Sources

- FSF licence list: Apache-2.0 is compatible with GPLv3 but not with GPLv2.
- Apache Software Foundation, "Apache License v2.0 and GPL Compatibility".
- SPDX text of the LLVM exception (GPLv2 compatibility clause).
- wasmtime `Cargo.toml`: `license = "Apache-2.0 WITH LLVM-exception"`.
- SolidCraft README: "MIT OR Apache-2.0".
- SolidCore repository: a Rust port of NVIDIA SMLib, Apache-2.0.
- PrusaSlicer repository: GNU AGPL-3.0.

This ADR is not legal advice. Have counsel review it before the first public
release.
