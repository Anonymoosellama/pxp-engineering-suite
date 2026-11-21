# PXP Engineering Suite — Status

The agent keeps this file true at all times; the user confirms gate outcomes.

Repository: <https://github.com/Anonymoosellama/pxp-engineering-suite>
(private). CI: `.github/workflows/ci.yml`, runs on every push.

## Gates

| Stage | Scope | State |
| --- | --- | --- |
| 0 | Foundations | **In progress** (sessions 1-2) |
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
| 0E | pxp-format: SQLite .pxp new/open/save/save as, migrations, recovery journal, backups, advisory lock, fuzz target | **Done, session 2** |
| 0F | Kernel bake-off harness and corpus; SolidCraft backend in-process; OCCT in the worker process | **Native half done, session 2** (harness, 67-case corpus, SolidCraft backend measured). OCCT worker, NIST STEP corpus, 200-part target: session 3 |
| 0G | Primitives (block, cylinder, cone, sphere), datum CSYS and WCS through the kernel trait | **Done headless, session 2** (CLI and Modeling recompute). WCS and the Block dialog arrive with the shell (0H) |
| 0H | ADR-0002 UI prototypes (egui+egui_dock vs Slint), NX-style shell, wgpu viewer | Blocked: needs crates.io |
| 0I | pxp-cli MCP server, journaling, record and replay with geometry-hash comparison | Partly possible now (CLI); MCP needs serde |
| 0J | Crash reporter, autosave, packaging, performance budgets, fuzzing, verification packet | Later |

## Session 2 plan (2026-11-21) — Stage 0, sub-stages 0E, 0F (native half), 0G

Environment check at session start: crates.io downloads work; public GitHub
repositories can be cloned; `nist.gov` is not reachable; the sandbox has 2
CPU cores and 7 GB RAM.

**Tasks**

1. **Housekeeping:** `pxp-compute::par_map` on rayon; proptest invariants for
   units, maths and the document core; a fuzz smoke job in CI.
2. **0E `pxp-format`:** `.pxp` as SQLite (rusqlite, bundled SQLite) per
   ADR-0003: schema v1, an explicit versioned binary codec for object
   properties (no serde coupling of the file format to Rust structs), new/open/
   save/save-as with atomic writes, a migration framework, the recovery journal
   (written after every committed transaction, replayed on next open), rolling
   backups, an OS advisory lock (`std::fs::File::try_lock`), byte-for-byte
   preservation of unknown kinds, and a cargo-fuzz target for the reader.
3. **0F native backend:** `pxp-kernel-native` on SolidCraft's
   `solidcraft-kernel` (built on SolidCore; git dependency pinned to one
   commit): primitives, booleans, topology, mass properties, geometry hash,
   and its exact history mapped to `TopoHistory`. A bake-off harness with
   synthetic stress cases and SolidCraft's reference parts. Licence: SolidCore
   is Apache-2.0 only, so record the `deny.toml` exception and LGPL-3.0
   conveyance (ADR-0001).
4. **0G primitives:** block, cylinder, cone, sphere and datum CSYS created
   through the kernel trait and recomputed by the Modeling application;
   headless version of the checklist item "create a block, save, close,
   reopen, geometry hash identical" (CLI and integration test).
5. Update design notes, ADR-0003/0004, this file; CI green on all targets.

**Deferred to session 3:** the OCCT worker-process backend (building OCCT from
source is impractical on 2 cores; options include prebuilt binaries or building
in CI), the NIST STEP corpus (`nist.gov` must be added to the allowed domains),
0H (UI toolkit prototypes, shell, viewer), 0I (MCP server, journaling), 0J.

**Risks**

- `solidcraft-kernel` is unpublished with an experimental API: pin the
  commit, wrap it entirely inside `pxp-kernel-native`, and expect breakage on
  upgrades.
- Compile time of SolidCore (about 43 MB of Rust source) on CI's six runners.
- The kernel's history may not cover every vertex and edge; `KernelHost`
  rejects incomplete histories, so any gap becomes a bake-off finding, not a
  silent workaround.

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

## Done (session 2, 2026-11-21)

Measured locally on Linux x86_64 (2 vCPU, 7 GB) with the stable toolchain
standing in for the pinned 1.97.0 (rustup cannot download 1.97.0 here). CI
results for the pushed commits are recorded under "CI" below.

| Item | Evidence |
| --- | --- |
| Housekeeping: `pxp-compute::par_map` on rayon; proptest invariants for units, maths, document undo/redo and the codec; fuzz smoke job in CI | `crates/*/tests/properties.rs`; `fuzz/`; CI job `fuzz` |
| 0E `pxp-platform`: advisory file locks, durable atomic file replacement | tests in `pxp-platform`, `pxp-format` |
| 0E `pxp-format`: `.pxp` on SQLite (schema v1, explicit binary codec, atomic save, 3 backups, recovery journal, lock, in-memory migrations, newer files read-only, byte-for-byte unknown kinds, `close` removes a clean journal) | 19 unit + 2 property tests; 2 cargo-fuzz targets; design note `docs/design/file-format.md`; ADR-0003 updated |
| 0F `pxp-kernel` additions: stored-shape envelope, `export_native`/`import_native`, `merged_counts`, `EmptyResult`, `DEFAULT_HASH_RESOLUTION` | 7 contract tests |
| 0F `pxp-kernel-native`: SolidCraft's kernel (git dependency pinned to `2091b88`): block (any CSYS), cylinder, cone (revolve), sphere, unite/subtract/intersect, topology, body kind, mass properties, geometry hash, merged counts, native B-rep I/O; complete `TopoHistory` with derived edge/vertex records | 12 conformance tests through `KernelHost::op`, all values against hand calculations |
| 0F bake-off harness and corpus (`pxp_kernel::bakeoff`, `pxp-cli bakeoff`): 32 synthetic stress cases + 35 SolidCraft analytic cases (converter `tools/import_solidcraft_cases.py`) | **67 / 67 pass, 0 incomplete histories, 67 / 67 deterministic**; median 163 ms per case: `docs/reports/kernel-bakeoff-2026-11-21-native.md`; ADR-0004 measurements |
| Licence: SolidCore (Apache-2.0 only) `deny.toml` exceptions; LGPL-3.0 conveyance of binaries linking it recorded | `deny.toml`; ADR-0001 table |
| 0G Modeling primitives: datum CSYS, block, cylinder, cone, sphere with expression parameters (`p0`, `p1`, …), immediate update policy, regenerated through `KernelHost`, stored with exact B-rep and geometry hash | `pxp-app-modeling::primitives`; tests below |
| 0G headless checklist "new part from mm and inch templates; create a block; save, close, reopen; geometry hash identical" | `pxp-cli part new/block/…/info/verify`; integration test `crates/pxp-cli/tests/part_roundtrip.rs` (3 tests: mm, inch, all primitives, refusal leaves the file untouched) |
| Cross-OS determinism check | CI: every target prints the corpus hashes; job `determinism` diffs them |
| Docs | `docs/design/kernel-contract.md` (derived records, backends, harness), `docs/design/file-format.md`, ADR-0001/0003/0004, README |

Workspace total: **181 tests passing, 0 failing**; clippy (pedantic,
`-D warnings`) and rustdoc (`-D warnings`) clean; `tools/check_repo.py`
0 problems; `cargo fmt --check` clean.

### CI

Pending for the session 2 push (see "In progress").

## Done (session 1, 2026-11-20)

Measured locally on Linux x86_64 with Rust 1.97.0, then confirmed by **CI run
#1 (2026-11-20, commit `460f201`): green on all six targets** (Linux
x86_64/aarch64, Windows x64/ARM64, macOS arm64/x86_64) plus the policy,
rustfmt and cargo-deny jobs, in 2 min 15 s:
<https://github.com/Anonymoosellama/pxp-engineering-suite/actions/runs/44846636374>.
The session 1 items below therefore meet the master prompt's "CI green on all
three OSes" condition.

| Item | Evidence |
| --- | --- |
| Repository, master prompt, ADR-0001 (licence asked; user chose LGPL-2.1-or-later), `LICENSE` | `docs/` |
| Workspace with all 39 crates, layered; 24 placeholders document their responsibility and planned stage and export no callable stubs | `cargo metadata`; `pxp-cli status` |
| Lint policy: edition 2024, `forbid(unsafe_code)` everywhere, clippy pedantic + `unwrap_used`/`expect_used`/`indexing_slicing`/`panic` denied | `cargo clippy --workspace --all-targets -- -D warnings`: 0 findings |
| Repo policy checks (layers, unsafe, crate docs, SPDX) | `python3 tools/check_repo.py`: 0 problems |
| `deny.toml` (ADR-0001 licence list; plain Apache-2.0 needs a recorded exception) | cargo-deny passed in CI run #1 (no external dependencies yet) |
| CI matrix: Linux x86_64/aarch64, Windows x64/ARM64, macOS arm64/x86_64 | CI run #1: all six clippy + test + doc jobs green |
| `pxp-units`: coherent mm-t-s units, newtypes, runtime quantities, unit parsing, mm/inch display (ADR-0007) | 27 unit tests + doc-tests |
| `pxp-math`: vectors, transforms, Csys, Aabb, tolerances, stable FNV-1a hash | 17 tests |
| `pxp-diag`, `pxp-compute`: error reports, panic messages, cancellation, ordered parallel map | 6 tests |
| `pxp-doc`: object store, expressions, dependency graph, transactions, undo/redo with memory cap, change bus, snapshots, parallel all-or-nothing recompute (ADR-0006) | 18 unit + 20 integration + 3 doc-tests, incl. 100-step undo/redo, panic rollback, failure isolation, determinism |
| `pxp-kernel`: `GeometryKernel` trait, `OpResult`/`TopoHistory`, primitive specs, `KernelHost` (in-process, panic containment, history-completeness check) | 6 contract tests |
| `pxp-appkit` + six applications registered (78 commands, all honestly "Planned for Stage N." except the six application switches) | 5 + 18 tests |
| `pxp-cli`: apps, commands, find, shortcut, eval, status, version | 5 tests; manual runs |
| Design notes with Mermaid diagrams: architecture, document core, kernel contract, expressions and units; ADR-0002 to ADR-0007; citations file | `docs/design/`, `docs/adr/`, `docs/CITATIONS.md` |

Workspace total: **129 tests passing, 0 failing**; rustdoc with `-D warnings`
clean.

## In progress

- CI for the session 2 push (first build of SolidCore on all six targets;
  first cross-OS hash comparison). Recorded under "Done (session 2) / CI"
  once it finishes.
- Stage 0 overall: the OCCT half of 0F, 0H, 0I and 0J remain (table above).

## Blocked

- **NIST MBE PMI STEP models** for the bake-off corpus: `nist.gov` is not
  reachable from the agent's sandbox. Fix: add `nist.gov` to the allowed
  domains (Settings → Capabilities), then start a new chat. They also need
  STEP import, which is not in the kernel trait yet.
- **OCCT backend**: building OCCT from source is impractical on the
  sandbox's 2 cores; session 3 tries prebuilt binaries or building in CI.

## Decisions needed from the user

- None blocking. For information: because `pxp-cli` now links the
  SolidCraft backend, its binaries are conveyed under LGPL-3.0 (ADR-0001,
  as planned for session 2). This is reversed if OCCT wins the bake-off and
  the native backend leaves shipped binaries.
- Optional, before Stage 4: whether user-written post processors that
  embed library layers should get an explicit licence exception (under LGPL
  they may stay private for in-shop use either way).

## Decisions taken

- ADR-0001: LGPL-2.1-or-later (user, 2026-11-20).
- ADR-0006: document core design and `Application` trait additions (agent).
- ADR-0007: coherent mm-t-s internal units; NX degree convention for bare
  numbers in trig (agent).
- ADR-0002 to ADR-0005: written as Proposed with evaluation plans; decisions
  wait for prototypes and measurements.
- Session 2 (agent): SolidCraft's kernel as the native bake-off candidate
  (git dependency pinned to one commit); face counts compared as merged
  counts; volume and area hashed to 9 significant digits; feature geometry
  stored as the object payload in a backend-tagged envelope; derived edge
  and vertex history records (documented in the kernel contract).

## Proposed plan for session 3 (Stage 0)

1. OCCT half of 0F: `pxp-kernel-occt` in a worker process (versioned
   protocol, restart, transaction replay), measured on the same corpus;
   decide ADR-0004.
2. Grow the corpus towards 200 parts: NIST STEP models once `nist.gov` is
   reachable and STEP import exists; more synthetic stress cases; slow cases
   in a nightly workflow.
3. 0H: ADR-0002 UI prototypes, the NX-style shell, the wgpu viewer, the
   Block dialog and the WCS.
4. 0I: MCP server and session journaling with geometry-hash comparison.
5. Persistent map for `DocState`; criterion benchmarks with the Stage 0
   budgets (0J).

## Temporary deviations from the master prompt

| Rule | Deviation | Why | Removed when |
| --- | --- | --- | --- |
| Typed errors via thiserror | Session 1 crates keep hand-written `Display` + `Error` impls; session 2 crates (`pxp-format`) use thiserror | Session 1 had no crates.io | Optional: equivalent; convert when those crates are next touched |
| criterion benchmarks | None yet | Budgets arrive with 0J | 0J |
| Kernel corpus of at least 200 parts | 67 cases | Stage 1 operations, STEP import and `nist.gov` access are missing | Session 3 onwards |
| Toolchain 1.97.0 pinned | Local checks run on the sandbox's stable toolchain; CI uses the pinned one | rustup cannot download 1.97.0 in the sandbox | Sandbox network allows static.rust-lang.org |
