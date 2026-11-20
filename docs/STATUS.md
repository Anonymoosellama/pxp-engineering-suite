# PXP Engineering Suite — Status

The agent keeps this file true at all times; the user confirms gate outcomes.

## Gates

| Stage | Scope | State |
| --- | --- | --- |
| 0 | Foundations | **In progress** (session 1) |
| 1 | Modeling (CAD, assemblies) | Not started; gated on "Stage 0 approved" |
| 2 | Drafting | Not started |
| 3 | Manufacturing core | Not started |
| 4 | Post Processor Generation, Machine Kit Builder, Machine Simulation | Not started |
| 5 | CAE: FEA, topology optimization, scan to CAD | Not started |

## Stage 0 sub-stages (defined by the agent; the master prompt lists deliverables only)

| Id | Sub-stage | State |
| --- | --- | --- |
| 0A | Repository, licence (ADR-0001), workspace skeleton with every crate, lint policy, CI matrix, layer check | Session 1 |
| 0B | Foundation crates: pxp-units, pxp-math (pxp-diag, pxp-platform, pxp-compute skeletons) | Session 1 |
| 0C | Document core pxp-doc: object store, typed unit-aware properties, expressions, dependency graph, recompute scheduler, transactions, undo/redo with memory cap, change bus, snapshots | Session 1 |
| 0D | Contracts: pxp-kernel trait, OpResult and TopoHistory, KernelHost; Application trait and command registry; every application registered with greyed "Planned for Stage N" commands | Session 1 |
| 0E | pxp-format: SQLite .pxp new/open/save/save as, migrations, recovery journal, backups, advisory lock, fuzz target | Blocked: needs crates.io (rusqlite) |
| 0F | Kernel bake-off harness and 200-part corpus (SolidCore in-process, OCCT in the worker process) | Blocked: needs network for crates and corpus |
| 0G | Primitives (block, cylinder, cone, sphere), datum CSYS and WCS through the kernel trait | After 0F |
| 0H | ADR-0002 UI prototypes (egui+egui_dock vs Slint), NX-style shell, wgpu viewer | Blocked: needs crates.io |
| 0I | pxp-cli MCP server, journaling, record and replay with geometry-hash comparison | Partly possible now (CLI); MCP needs serde |
| 0J | Crash reporter, autosave, packaging, performance budgets, fuzzing, verification packet | Later |

## Session 1 plan (2026-11-20) — Stage 0, sub-stages 0A to 0D

**Tasks**

1. Repository with `docs/MASTER_PROMPT.md`, this file and ADR-0001 (licence asked; user chose LGPL-2.1-or-later).
2. Workspace with every crate from the architecture section, each with `//!` docs (responsibility, data flow, invariants, must-never).
3. Lint policy (edition 2024, `forbid(unsafe_code)`, clippy pedantic plus `unwrap_used`, `expect_used`, `indexing_slicing`), `deny.toml`, CI matrix for Windows, macOS (arm64 and x86_64) and Linux, dependency-direction check.
4. Implement pxp-units, pxp-math, pxp-doc, pxp-kernel contracts, pxp-appkit (Application trait, command registry) and the six application crates' registrations.
5. Design notes with Mermaid diagrams for transactions/undo, the recompute graph and the expression engine; ADRs 0002 to 0006 as Proposed.
6. Run fmt, clippy and tests locally; update this file.

**Risks**

- No crates.io access in this session: only std-only crates can be built. thiserror, rusqlite, rayon, egui, wgpu, proptest and criterion are unavailable, so error types are written by hand and parallelism uses `std::thread::scope`. Each substitution is listed under "Temporary deviations" below.
- CI has never run: the workflow is written but cannot be exercised from this sandbox, so "CI green on all three OSes" is not yet met for anything.
- Snapshots copy the object map (Arc-shared objects, O(n) pointer copies per transaction). Acceptable for Stage 0 scale; replace with a persistent map once crates are available (ADR-0006).

## Done

*(updated at the end of the session)*

## In progress

*(updated at the end of the session)*

## Blocked

- 0E, 0F, 0H and the MCP half of 0I need crates.io. Fix: allow package installs in the claude.ai network settings, then start a new chat (an existing chat may not pick up the change).

## Decisions needed from the user

*(none open)*

## Decisions taken

- ADR-0001: LGPL-2.1-or-later (user, 2026-11-20).

## Temporary deviations from the master prompt

| Rule | Deviation | Why | Removed when |
| --- | --- | --- | --- |
| Typed errors via thiserror | Hand-written `Display` + `Error` impls | No crates.io access | Optional: hand-written impls are equivalent; adopt thiserror when the network is available if preferred |
| Data-parallel work uses rayon | Recompute levels use `std::thread::scope` chunks | No crates.io access | Network available; swap behind `pxp-compute` |
| Property tests with proptest | Deterministic table-driven tests only | No crates.io access | Network available |
| criterion benchmarks | None yet | No crates.io access | Network available (0J) |
