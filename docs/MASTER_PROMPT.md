# PXP Engineering Suite — Staged Master Build Prompt

Nov 19, 2026 · @Elvin

> Exported verbatim from the project document "PXP Engineering Suite — Staged
> Master Build Prompt" (claude.ai project "Open Source CAx Software Suite").
> Diagrams in the original are images; only their captions survive here.
> The agent re-reads this file at the start of every session.

## How to use this prompt

Give your coding agent the whole document once, then ask it to build one stage per working session; it must stop at every gate until you reply "Stage N approved." Everything from "Role, mission and scope" downward is written to the agent. This first section is for you.

- Create an empty Git repository and save this document in it as docs/MASTER_PROMPT.md (export it as Markdown). The agent re-reads it at the start of every session, so decisions never drift.

- Open a session with: *"**Read docs/MASTER_PROMPT.md and docs/STATUS.md. Work only on Stage N, sub-stage X. Follow the agent operating rules.**"*

- The agent plans, writes an architecture decision record (ADR) for any big choice, implements, tests, and produces a **verification packet**: build links for all three OSes, a checklist, sample .pxp files and known limitations.

- You test against the checklist and reply either *"**Stage N approved**"* or a list of fixes. The agent fixes the list and re-submits; it never starts the next stage on its own.

- Record the outcome in docs/STATUS.md (the agent maintains it; you confirm it).

The order is fixed by your requirements: Foundations → Modeling → Drafting → Manufacturing core → Post Processor Generation, Machine Kit Builder and Machine Simulation → CAE (FEA, topology optimization and scan to CAD). Each later application reads data the earlier one creates, so a later stage built on an unverified earlier stage would inherit its bugs.

*[diagram] staged roadmap · 6 stages, 5 user gates*

Every arrow passes through a gate: the agent hands over a verification packet and waits for your approval before the next stage starts.

**Why the gates are hard stops:** a coding agent that runs ahead will happily build drafting views on top of a modeling kernel with a broken topological-naming scheme, and every hour spent there is lost when the kernel is fixed. A gate costs you an afternoon of testing; skipping one can cost weeks.

The notes marked **Why:** throughout explain the reasoning behind each requirement. Leave them in when you paste; they help the agent make good calls in situations this prompt doesn't cover.

## Role, mission and scope

You are the lead architect and implementer of **PXP Engineering Suite** (working name, rename freely): an open-source, Rust-native CAD, Drafting, CAM and CAE system. Its interface and workflow follow Siemens NX as closely as the law allows, and it saves everything in one unified .pxp part file.

**Mission**

- One program, several *applications*: Modeling (including assemblies), Drafting, Manufacturing (subtractive and additive operations plus Machine Simulation), Post Processor Generation, Machine Kit Builder and CAE (FEA, topology optimization and scan to CAD). The user switches between them inside one open part, the way NX's Application tab works, without exporting or re-importing anything.

- One file type, .pxp, carries every application's data. A change in one application flows into the others through live associativity.

- Robustness comparable to commercial CAE: no data loss, deterministic recompute, crash recovery.

- Native desktop builds for Windows, macOS and Linux.

- Modular from day one: each application is a module written against a stable internal API, so new applications and features can be added later without touching the core.

**Scope check (read this twice)**

- NX is the product of decades of work by large teams. Parity with it is the direction of travel, not the bar for Stage 1. Every stage below defines a *minimum credible* bar plus a backlog toward parity. Build depth in each gated stage before breadth.

- Never fake a feature. A command that isn't implemented is hidden or greyed out with a tooltip saying so. It is never wired to a stub that silently does nothing or returns plausible-looking wrong geometry.

- Never claim parity in docs or UI. Report measured coverage instead, such as "212 of 240 reference parts rebuild identically."

- When a requirement conflicts with stability or data integrity, stability wins. Raise the conflict in the verification packet and propose options.

- Out of scope unless the user asks: cloud services, PLM/vault systems, multi-user editing, telemetry.

**Why:** the most likely failure is a wide, shallow program that looks like NX but breaks on real parts. Users forgive missing commands; they don't forgive a corrupted part file or a toolpath that gouges.

## Non-negotiable engineering constraints

These rules apply to every line of code in every stage; a pull request that breaks one is not done, whatever it adds.

**Language and toolchain**

- Rust stable, edition 2024, MSRV pinned in rust-toolchain.toml, one Cargo workspace.

- #![forbid(unsafe_code)] in every crate except a short, named list of FFI crates (for example pxp-kernel-occt-sys). Every unsafe block carries a // SAFETY: comment stating the invariant it relies on; run Miri on those crates where it can execute.

- No unwrap(), expect(), panic!, todo! or unimplemented! outside tests. Libraries return typed errors (thiserror); only the application shell converts them into user messages.

- Clippy runs with -D warnings, clippy::pedantic, and unwrap_used, expect_used, indexing_slicing denied in library crates. Any allow needs a comment saying why.

- cargo deny gates licences, duplicate crates and security advisories in CI.

**Reliability and data integrity**

- Every user action is a **transaction** on the document: it applies fully or rolls back fully, and it is undoable. No command mutates the model outside a transaction.

- Panics are caught at the command boundary (std::panic::catch_unwind), the transaction rolls back, the user sees an error with a "copy report" button, and the session continues.

- Native C/C++ code (for example OCCT, if used) can crash the process in ways catch_unwind cannot stop. All geometry calls therefore go through a KernelHost that can run a backend in-process (pure-Rust kernels) or in a separate worker process (any native backend). If the worker dies, it restarts and the transaction replays or rolls back.

- Saves are atomic (write, fsync, rename, or a SQLite transaction). A write-ahead recovery journal is updated on every committed transaction; on the next launch after a crash the user is offered recovery.

- Recompute is **deterministic**: the same inputs give the same topology, the same persistent names and the same geometry hash on every OS. CI tests this across the three platforms.

- Every file reader (.pxp, STEP, IGES, DXF, STL, G-code and the rest) is fuzzed with cargo-fuzz and must never panic or hang on malformed input.

**Cross-platform**

- Targets: Windows 10/11 (x64, ARM64), macOS 13+ (universal x86_64 + arm64, signed and notarized), Linux x86_64 and aarch64 (Wayland and X11). Packages: MSI/MSIX, DMG, AppImage plus Flatpak and .deb.

- All OS-specific code lives in one pxp-platform crate. CI builds and runs the full test suite on all three OSes for every pull request.

**Performance: multi-core and GPU**

- The UI thread never blocks. Long operations run on worker threads with a progress bar and a cancel button (cooperative cancellation tokens checked inside every long loop).

- Data-parallel work (tessellation, toolpath generation, slicing, meshing, element assembly) uses rayon. The recompute engine schedules independent branches of the feature graph in parallel.

- Rendering uses wgpu (Vulkan, Metal, DX12). Heavy numeric work that suits the GPU uses wgpu compute shaders: material-removal simulation, slice previews, large mesh operations, sparse matrix-vector products for iterative FEA solvers.

- Every GPU compute path has a CPU reference implementation, an automatic fallback when no suitable GPU exists, and a test asserting both agree within a stated tolerance.

- Stage 0 sets measurable performance budgets (frame rate while orbiting a large model, open time, recompute time, toolpath time). They are tracked with criterion benchmarks in CI, and a regression beyond 10 % fails the build.

**Units and tolerances**

- Internal length unit is the millimetre (f64), internal angle unit the radian, stored in newtypes (Length, Angle, Force, Stress and so on) so units cannot be mixed by accident. Each part has a display unit system (mm or inch), as NX parts do.

- Modeling distance and angle tolerances are stored per part and shown in part preferences.

**Code comments and documentation (required, not optional)**

- Every crate starts with //! docs explaining its responsibility, its data flow, the invariants it guarantees and what it must never do.

- Every public item has rustdoc covering what it does, why it exists, units of every quantity, errors it returns and an example where useful.

- Every non-trivial algorithm has a header comment in plain language: the method, the maths, complexity, known failure modes and a citation (paper, standard clause, or the FreeCAD/SolidCraft source file it was derived from, with licence).

- Each logic system (recompute graph, persistent naming, cross-application associativity, the post-processor event pipeline, the virtual controller, solver pipelines) gets a design note in docs/design/ with a Mermaid diagram kept in sync with the code.

**Why:** a CAE tool's users bet real parts, machines and money on its output. Typed units stop the classic mm-versus-inch crash; transactions and atomic saves stop data loss; the CPU reference path makes every GPU result checkable; and dense comments let the next contributor (human or agent) extend a module without re-deriving its logic.

## Reference projects and licensing

SolidCraft is the code base to build on, because it is already pure Rust under MIT/Apache-2.0; FreeCAD is the architecture and feature reference, because its C++/Python code would have to be rewritten and stays LGPL if translated. PrusaSlicer is AGPL-3.0, so the licence decision below must be made before any slicer code is ported.

| Project | Licence | What to take | How to use it |
| --- | --- | --- | --- |
| [SolidCraft](https://github.com/tangency-labs/solidcraft) | MIT OR Apache-2.0 code; brand and interface under a separate LICENSE-BRAND | Pure-Rust parametric CAD: own sketch constraint solver, feature timeline (extrude, revolve, sweep, loft, fillet, chamfer, shell, draft, holes, threads, patterns), assemblies, unit-aware expressions, STEP/IGES/3MF/STL/OBJ/glTF I/O, one command layer shared by desktop, CLI and MCP server, experimental WASM plugin runtime | Fork or vendor the Rust code with attribution. Do **not** reuse its name, logo, icons or interface design (LICENSE-BRAND). Its headless solidcraft-cli mcp server can act as an independent test oracle (volumes, areas, mass properties) on reference parts. |
| [SolidCore](https://github.com/bherbruck/solidcore) | Apache-2.0 | Safe-Rust B-rep kernel ported from NVIDIA SMLib: NURBS, topology, intersections, booleans, fillets, chamfers, offset/shell, sweeps, skins, healing, mass properties | Leading candidate for the primary geometry kernel backend (see kernel bake-off in Stage 0). |
| [FreeCAD](https://wiki.freecad.org/License) 1.1 | LGPL-2.0-or-later (C++/Python on OCCT, Coin3D, Qt) | Architecture: App/GUI split, document objects with a property system, recompute dependency graph, workbenches (= our applications), the topological-naming work that landed in 1.0, TechDraw (drafting), CAM workbench, FEM workbench | Study and reimplement in Rust. A line-by-line translation is a derivative work and stays LGPL: keep any translated code in clearly marked crates and record the source file in its header comment. |
| [OCCT](https://dev.opencascade.org/) via [cadrum](https://docs.rs/crate/cadrum/0.8.1-alpha.0) or [opencascade-rs](https://github.com/bschwind/opencascade-rs) | LGPL-2.1 (OCCT) | Mature STEP/IGES translators, shape healing, a second opinion on booleans and fillets | Optional backend run in the out-of-process kernel worker. Use it for data exchange and as a cross-check oracle in tests. If linked statically, meet LGPL relinking obligations. |
| [truck](https://github.com/ricosjp/truck), [arris](https://docs.rs/crate/arris/0.7.0), Fornjot | Apache-2.0 / MIT-or-Apache-2.0 | Pure-Rust NURBS B-rep, tessellation, wgpu rendering ideas, validation-first topology checks | References and fallback kernel candidates. |
| [PrusaSlicer](https://github.com/prusa3d/PrusaSlicer) | AGPL-3.0 | Slicing pipeline: perimeters, infill patterns, supports, seams, bridging, variable layer height, print/filament/printer profiles, G-code flavours | Port only if the project licence is AGPL-compatible (decision below); otherwise treat it as a behaviour reference and implement from published algorithms. |
| [Netgen](https://sourceforge.net/projects/netgen-mesher/) | LGPL | Robust tetrahedral and surface meshing, high-order elements | Candidate mesher for Stage 5, behind a Mesher trait. |
| CalculiX, Gmsh | GPL | FreeCAD's default FEA solver and mesher | Run only as optional external processes for cross-validation, the way [FreeCAD's FEM workbench](https://wiki.freecad.org/FEM_Install) calls a standalone CalculiX binary. Never link them in. |
| [faer](https://github.com/sarah-quinones/faer-rs) | MIT | Pure-Rust dense and sparse linear algebra: Cholesky, LDLT, LU, QR, eigen and SVD decompositions | Default linear-algebra backend for FEA. |
| [egui](https://github.com/emilk/egui) + [egui_dock](https://github.com/Adanos020/egui_dock) | MIT / Apache-2.0 | Immediate-mode GUI on wgpu, dockable and undockable tabbed panels | Leading candidate for the UI layer (confirm in a Stage 0 ADR against Slint and iced). |
| [beso](https://github.com/calculix/beso) and SIMP teaching codes (top88 family) | LGPL-3.0 (beso) | Topology optimization prior art: BESO driving CalculiX from FreeCAD; compact SIMP reference implementations | Study the methods and reimplement them in Rust on the Stage 5 solvers. Copy beso code only if ADR-0001 is LGPL-compatible. |
| [Open3D](https://www.open3d.org) | MIT | Point-cloud processing: normal estimation, ICP and RANSAC registration, Poisson reconstruction, mesh clean-up | Algorithm reference for the scanning pipeline; reimplement in Rust, or run it out of process if a step is not yet ported. |
| [PoissonRecon](https://github.com/mkazhdan/PoissonRecon) and the [poisson_reconstruction](https://docs.rs/crate/poisson_reconstruction/latest/) crate | MIT (PoissonRecon); confirm the crate's licence | Screened Poisson surface reconstruction from oriented points | Start from the Rust crate; validate its meshes against the reference code. |
| [e57](https://github.com/cry-inc/e57) crate | MIT | Pure-Rust E57 point-cloud reader and writer with no unsafe code | Use directly for E57 import and export. |

**Licence decision (ADR-0001, ask the user before writing code)**

- **AGPL-3.0-or-later for the whole suite (recommended if PrusaSlicer code will be ported).** SolidCraft (MIT/Apache-2.0), SolidCore (Apache-2.0) and FreeCAD-derived code (LGPL) can all be combined into an AGPL-3.0 work. Every distributed build must ship source.

- **LGPL-2.1-or-later or MPL-2.0 for the suite.** Allows wider reuse and embedding, but no PrusaSlicer code may enter the tree; slicing is written from papers and observed behaviour.

Record the choice, add SPDX headers to every file, and let cargo deny enforce the allowed-licence list.

**Imitating NX and Mastercam without copying them**

- Copy workflow, layout and the structure of dialogs. Do not copy Siemens or CNC Software icons, screenshots, help text, sample files or trademarks. The product name must not contain "NX"; use generic names ("Post Processor Generation", not "Post Configurator").

- Some constant-engagement and adaptive roughing methods have been patented. Implement from published academic work, keep a citations file, and tell the user to run a freedom-to-operate check before a public release.

**Why:** licence mistakes are expensive to undo once code is mixed. Choosing the licence first, keeping translated code in labelled crates, and running GPL tools only as separate processes keeps every option open.

## System architecture

Build a layered Cargo workspace: a document core that knows nothing about geometry or UI, a kernel layer with swappable backends, an application layer where each NX-style application is a module, and a thin UI shell on top. Lower layers never depend on higher ones; CI enforces this with a dependency-direction check.

*[diagram] architecture · 4 layers, 2 kernel backends*

Arrows point to what each layer uses. Nothing lower in the stack calls anything above it, and applications reach geometry only through the kernel trait. Inside the application layer, Machine Simulation in Manufacturing uses the post from Post Processor Generation and the kit from Machine Kit Builder.

**Crate layout (rename freely; keep the layering)**

| Layer | Crates | Responsibility |
| --- | --- | --- |
| Foundation | pxp-units, pxp-math, pxp-platform, pxp-compute, pxp-diag | Unit newtypes, f64 geometry maths and tolerances, OS abstraction, thread pool plus GPU device management with CPU fallback dispatch, tracing and crash reports |
| Document core | pxp-doc | Object store with stable UUIDs, typed unit-aware properties, an expression engine (NX-style named expressions such as p12 = width * 2), the dependency graph and recompute scheduler, transactions with undo/redo, change-notification bus, persistent-naming service, attributes, 256 layers, reference sets |
| File format | pxp-format | Reads and writes .pxp, schema migrations, lazy loading, recovery journal |
| Geometry | pxp-kernel (trait), pxp-kernel-native, pxp-kernel-occt, pxp-sketch, pxp-tess | Kernel API and backends, 2D/3D constraint solver, tessellation with level of detail |
| Applications | pxp-app-modeling (assemblies included), pxp-app-drafting, pxp-app-manufacturing (subtractive and additive operations plus Machine Simulation), pxp-app-postgen (Post Processor Generation), pxp-app-machine-kit (Machine Kit Builder), pxp-app-cae (FEA, topology optimization, scan to CAD) | Six applications, one crate each, each implementing the Application trait below. Applications never depend on each other; they share the domain libraries in the next row |
| Domain libraries | pxp-cam, pxp-additive, pxp-machine (kit model and kinematics), pxp-post (post engine and layer resolver), pxp-vnc (virtual controller), pxp-fea, pxp-topopt, pxp-scan, pxp-pointcloud | Engines the applications share. Machine Simulation in Manufacturing runs pxp-post and pxp-vnc on a pxp-machine kit; Post Processor Generation and Machine Kit Builder are the editors for those same libraries |
| Data exchange | pxp-io-step, -iges, -dxf, -pdf, -stl, -3mf, ... | One crate per format behind Importer / Exporter traits |
| Rendering | pxp-render | wgpu scene, GPU ID-buffer picking, edges and silhouettes, order-independent transparency, section clipping, instancing for assemblies, an annotation overlay |
| Shell and automation | pxp-ui, pxp-app (binary), pxp-cli | Ribbon, navigators, dialog framework, selection and snapping; headless CLI and MCP server running the same commands; journaling (record and replay user sessions) |

**Core contracts (write these first, with comments at this level of detail)**

```rust
/// An NX-style "application" (Modeling, Drafting, Manufacturing, CAE...).
///
/// Applications are editors over ONE shared document; none owns model data
/// exclusively. Switching application in an open part therefore never copies
/// or converts data: it only swaps the active ribbon tabs, navigators and
/// commands. This is what lets a Drafting dimension or a CAM operation stay
/// live against the Modeling geometry it references.
pub trait Application: Send + Sync {
    /// Stable id persisted in .pxp files, e.g. "pxp.modeling".
    /// Never change it after release: saved files refer to it.
    fn id(&self) -> AppId;

    /// Object kinds whose schema this application owns
    /// (Drafting owns DrawingSheet, DraftingView, Dimension, ...).
    fn owned_kinds(&self) -> &[ObjectKind];

    /// Commands contributed to the global registry. The UI builds ribbons
    /// from these; the CLI and MCP server expose the same commands headless.
    fn commands(&self) -> Vec<CommandSpec>;

    /// Called when upstream objects this application depends on change.
    /// Must only MARK dependants out of date; the recompute scheduler decides
    /// when and on which threads to regenerate them.
    fn on_upstream_change(&self, ctx: &mut ChangeCtx, change: &ChangeSet);

    /// Enter / leave this application inside an open part.
    fn activate(&self, ui: &mut UiContext);
    fn deactivate(&self, ui: &mut UiContext);
}

/// Returned by EVERY topological operation in EVERY kernel backend.
pub struct OpResult {
    /// Resulting body: solid, sheet (surface) or wire.
    pub shape: ShapeHandle,
    /// Which output faces/edges/vertices were generated from, modified from,
    /// or replaced which input entities. Persistent naming is built on this
    /// map: without it, a blend on "the top edge" is lost the moment the
    /// extrude that created that edge is edited.
    pub history: TopoHistory,
    /// Non-fatal issues (a tolerance relaxed, a tiny edge merged). Shown as a
    /// warning badge in the Part Navigator; never dropped silently.
    pub warnings: Vec<KernelWarning>,
}
```

**Concurrency model**

- The document has a single writer. Transactions are applied on a dedicated document thread.

- Readers (UI, renderer, recompute workers, CAM and FEA solvers) work on immutable **snapshots** of the document (persistent data structures or generation-stamped arenas), so no reader ever takes a lock that can freeze the UI.

- Recompute: the scheduler topologically sorts out-of-date objects, runs independent branches in parallel on the rayon pool against a snapshot, then commits all results in one transaction. If anything fails, nothing is committed and the failing feature is flagged in the navigator.

**Kernel strategy**

- All geometry goes through the pxp-kernel trait. Application code never calls a backend directly.

- Stage 0 runs a **kernel bake-off**: SolidCraft's kernel / SolidCore, and OCCT via the worker process, measured on a shared corpus of at least 200 parts for boolean success rate, blend success rate, STEP round-trip fidelity, speed and cross-OS determinism. Record the winner in an ADR. Keep the other backend as a test oracle and for data exchange.

**Extensibility**

- Built-in applications are statically linked crates.

- Third-party extensions are **WebAssembly components** run in wasmtime, against a versioned WIT interface (SolidCraft's craft:plugin interface is prior art). A plugin can add commands, feature types, object kinds with their schema, importers and exporters, CAM operations and post-processor layers. Rust has no stable ABI, so native Rust dylib plugins are not used.

**Why:** NX keeps working through decades of features because applications are layered over one part model. The snapshot model gives multi-core recompute without data races, and the history map in OpResult is the single most important interface in the program: drafting associativity, CAM associativity and CAE load associativity all depend on it.

## The unified .pxp file format

A .pxp file is a single SQLite database holding every object of every application (sketches, curves, datums, coordinate systems, solids, sheets, drawings, CAM setups, toolpaths, FEA models and results, topology-optimization studies, point clouds and scan meshes), plus references to, or embedded copies of, other .pxp files used as assembly components. This mirrors how NX uses one .prt type for parts, assemblies, drawings and manufacturing setups.

**Why SQLite:** it is transactional and crash-safe, so a power cut mid-save cannot corrupt the part. It is one portable file, it can load sections lazily, it saves incrementally instead of rewriting everything, and it can be inspected with standard tools when debugging. A ZIP-of-files container (FreeCAD's .FCStd approach) has to be rewritten whole on every save and has no transactions. Record this choice in an ADR; if you find a better container, argue it there before switching.

**Logical schema (illustrative; the agent refines it in Stage 0)**

```sql
-- Identify the file type to tools: PRAGMA application_id = <PXP magic>;
-- Schema version for migrations:     PRAGMA user_version  = <n>;

-- Part-level settings: units, tolerances, writer version, thumbnail.
CREATE TABLE meta (key TEXT PRIMARY KEY, value BLOB NOT NULL);

-- Every object of every application lives here. 'kind' is namespaced by
-- the owning application (e.g. 'modeling.extrude', 'drafting.view',
-- 'cam.operation.cavity_mill'), so one table serves all applications and
-- a new plugin application needs no new top-level file structure.
CREATE TABLE objects (
  id          BLOB PRIMARY KEY,   -- UUIDv7: stable forever, never reused
  kind        TEXT NOT NULL,      -- namespaced object kind
  kind_ver    INTEGER NOT NULL,   -- payload schema version for this kind
  owner_app   TEXT NOT NULL,      -- e.g. 'pxp.manufacturing'
  parent      BLOB,               -- navigator hierarchy (CAM groups, sheets)
  name        TEXT,
  layer       INTEGER,            -- 1..256, as in NX
  display     BLOB,               -- colour, translucency, line font/width
  state       INTEGER NOT NULL,   -- up to date / out of date / failed / suppressed
  payload     BLOB NOT NULL       -- versioned binary (e.g. postcard) per kind
);

-- Edges of the dependency graph across ALL applications. A drafting view
-- depends on a body; a CAM operation depends on faces; a FEA load on a face.
CREATE TABLE deps (from_id BLOB, to_id BLOB, role TEXT,
                   PRIMARY KEY (from_id, to_id, role));

-- Persistent names: how a reference to 'that face' survives edits.
-- Stores the naming chain built from kernel OpResult history maps.
CREATE TABLE topo_names (name_id BLOB PRIMARY KEY, owner BLOB,
                         chain BLOB NOT NULL, last_resolved BLOB);

-- Named expressions (NX-style parameters), with units.
CREATE TABLE expressions (name TEXT PRIMARY KEY, formula TEXT,
                          unit TEXT, cached REAL);

-- Exact B-rep per body, plus a disposable tessellation cache so the part
-- shows on screen before the kernel has loaded the exact geometry.
CREATE TABLE geometry (shape_id BLOB PRIMARY KEY, backend TEXT,
                       brep BLOB, hash BLOB, tess BLOB, tess_valid INTEGER);

-- Assembly components: referenced (default, like NX) or embedded.
CREATE TABLE components (id BLOB PRIMARY KEY, part_uuid BLOB,
  rel_path TEXT, abs_path TEXT, content_hash BLOB,
  transform BLOB, ref_set TEXT, embedded INTEGER);

-- Large data (toolpaths, IPW, meshes, FEA results, point clouds) split into chunks,
-- because one SQLite blob is limited in size.
CREATE TABLE blobs (blob_id BLOB, seq INTEGER, data BLOB,
                    PRIMARY KEY (blob_id, seq));
```

**Associativity rules (the heart of the multi-application workflow)**

- Every cross-application reference is an object UUID plus, where it points at topology, a persistent name. Never store a transient face index or a pointer.

- On each committed transaction the change bus emits a change set; every dependant is marked **out of date** through on_upstream_change.

- Each application chooses its update policy, which the user can change in preferences:

  - Modeling features recompute immediately.

  - Drafting views and dimensions update automatically (or on demand for large drawings).

  - CAM toolpaths are marked *needs regenerate* and are never regenerated silently: a programmer must review a toolpath after geometry changes. Post output is marked stale.

  - CAE meshes and results are marked out of date.

- A reference that cannot be resolved after an edit (the face was deleted) becomes a **broken reference** with a repair dialog for re-selecting it. It is never silently re-bound to a guess.

- Edits may flow downstream-to-upstream where it makes sense: changing a feature-driven dimension in Drafting edits the feature parameter in Modeling, as NX allows.

- Both NX workflows are supported: drawings, CAM and CAE inside the model part, or in a separate **master-model** part that references the design part as a read-only component.

**Compatibility and integrity**

- Every payload is versioned. Newer builds migrate older files in memory; older builds open newer files read-only and list what they cannot interpret.

- Objects of unknown kinds (for example from a plugin that is not installed) are preserved byte for byte and written back on save, never dropped.

- Geometry blobs carry hashes checked on load; a background PRAGMA integrity_check runs after open; the last N saves are kept as backups; an advisory lock stops two sessions from writing the same file.

**Export rules**

| What the user made or selected | Default export | Also offered |
| --- | --- | --- |
| Solid or sheet bodies | STEP AP242 (colours, names, layers; PMI when available) | STEP AP214/AP203, IGES, STL, 3MF, glTF |
| Sketches, curves, 2D geometry | DXF (layers mapped) | IGES curves |
| Drawing sheets | PDF (vector, fonts embedded, one page per sheet) | DXF per sheet |
| CAM programs | Post-processed NC code | CLSF-style tool-path listing |

The export dialog pre-selects the format from the active application and the selection, and the user can always override it. Scan meshes and topology-optimization results export as STL, PLY, OBJ or 3MF, point clouds as E57 or PLY, and fitted surfaces and solids follow the body rule.

**Why:** NX's single part type is what makes switching between Modeling, Drafting and Manufacturing seamless. Reproducing it needs three things: one container, one object table shared by every application, and references that survive edits. Persistent names and visible out-of-date states give users the NX experience of "change the model, see what needs attention downstream" without hidden surprises.

## NX-style UI and workflow

Reproduce NX's layout and interaction grammar so an experienced NX user can work without training: ribbon on top, resource bar with navigators on the left, graphics window in the centre, cue and status lines at the bottom, selection filters on the top border bar, and dialogs that step the user through selections.

**Window layout**

| Element | Behaviour to reproduce |
| --- | --- |
| Quick Access Toolbar | Save, undo, redo, repeat last command, window switching, in the title bar |
| File tab and Application switcher | File menu plus an Application ribbon tab listing the six applications: Modeling (assemblies included), Drafting, Manufacturing (subtractive and additive operations plus Machine Simulation), Post Processor Generation, Machine Kit Builder and CAE (scan to CAD included). Parts open in Modeling. Switching keeps the part open and changes only ribbons and navigators |
| Ribbon | Tabs per application (Modeling: Home, Curve, Surface, Assemblies, Analysis, View, Selection, Render, Tools, Application). Groups with galleries and overflow; fully customisable with right-click > Customize |
| Top border bar | Menu button (a classic menu path to every command), type selection filter, selection scope (entire assembly / within work part), snap points (end, mid, control, intersection, arc centre, quadrant, point on curve/face), curve rules (single, connected, tangent, face edges, region boundary) and face rules (single, tangent, adjacent, feature, body faces, region) |
| Resource bar (left, dockable) | Assembly Navigator, Constraint Navigator, Part Navigator, Reuse Library, History, Roles; plus Operation Navigator and Machine Tool view (Manufacturing), Layer Navigator (Post Processor Generation), Kinematics Navigator (Machine Kit Builder) and Simulation Navigator (CAE). Every navigator has Details, Dependencies and Preview panels |
| Graphics window | View triad, WCS and datum CSYS display, dynamic section clipping, rendering styles (shaded with edges, shaded, wireframe with dim or hidden lines, studio), translucency, named views, multiple view layouts |
| Cue line and status line | Cue line prompts the next action ("Select section geometry"); status line reports results and warnings |
| Information window | Text listing for Information > Object, measurements and command reports, copyable |

**Part Navigator (Modeling)**: features in timestamp order with icons, rollback (*Make Current*), edit with rollback, suppress and unsuppress, reorder, rename inline, show parents and children in the Dependencies panel, show the feature's expressions in the Details panel, filters, and status badges (up to date, out of date, failed, warning).

**Dialog conventions**

- Dialogs dock at the upper left of the graphics window by default and remember their last size and settings.

- Each dialog is a vertical stack of groups. Selection steps show "Select Object (0)" with a red marker while required, and focus moves to the next step automatically when a step is satisfied.

- On-screen handles and input boxes in the graphics window for distances, angles and directions, with reverse-direction arrows.

- Preview and Show Result toggles; buttons OK, Apply, Cancel and a Reset control.

- Every numeric field accepts an expression or a reference to a named expression, with units (25.4mm, 1in, =width/2).

- QuickPick: pausing over overlapping geometry offers a list to choose from.

**Mouse grammar**

- MB1 select; MB2 drag rotate; MB2+MB3 (or Shift+MB2) pan; MB1+MB2 (or Ctrl+MB2) and the wheel zoom, centred on the cursor.

- MB2 click acts as OK or advances to the next step inside a dialog.

- MB3 click opens the context menu for the object under the cursor; MB3 press-and-hold opens a radial menu; Ctrl+Shift+MB1/MB2/MB3 open three customisable radial toolbars.

**Default keyboard shortcuts (all remappable)**

| Keys | Command |
| --- | --- |
| Ctrl+M / Ctrl+Shift+D / Ctrl+Alt+M | Switch to Modeling / Drafting / Manufacturing |
| Ctrl+J | Edit Object Display: colour, translucency, layer, line font and width of any selected objects, faces included |
| Ctrl+B, Ctrl+Shift+K, Ctrl+Shift+U, Ctrl+Shift+B, Ctrl+W | Hide, show, show all, invert shown and hidden, show and hide by type |
| Ctrl+L | Layer settings |
| Ctrl+T | Move object |
| Ctrl+F, Ctrl+R, Ctrl+H | Fit, rotate, edit section |
| Ctrl+Alt+F / T / L / R | Front, top, left, right views |
| Home, End, F8 | Trimetric, isometric, snap to nearest orthographic view |
| F4 | Repeat last command |
| Ctrl+Z, Ctrl+Y, Ctrl+D | Undo, redo, delete |
| W, X, Ctrl+Q | Toggle WCS display, Extrude, Finish sketch |
| Ctrl+1, Ctrl+2, Ctrl+Shift+J | Customize, user-interface preferences, object preferences |

**Roles, defaults and preferences**

- Roles: an *Essentials* role (simplified ribbon) and an *Advanced* role (Menu button, top border bar, full ribbons), plus user-defined roles saved as files.

- Layered **customer defaults** (site, group, user files, later layers override earlier) control new-part settings; **part preferences** are stored in each .pxp; **session preferences** last until exit. This separation is how NX lets a company standardise drafting and CAM settings.

- Command Finder: a search box that runs any command and shows where it lives in the ribbon.

- Light and dark themes, high-DPI scaling, full keyboard navigation, translatable strings (Fluent) from day one.

**Do not copy:** Siemens icons, splash screens, colour themes that imitate NX branding, or help text. Draw an original, consistent icon set at the same sizes and positions.

**Why:** NX users' speed comes from muscle memory: where a command sits, what MB2 does inside a dialog, how selection intent works. Matching the grammar matters more than matching pixels. Customer defaults versus part preferences is easy to overlook and hard to retrofit, so it is designed in from Stage 0.

## Stage 0 — Foundations

Stage 0 delivers a real but nearly empty application: the NX-style shell, the document core with transactions and undo, a .pxp that opens and saves safely, a fast viewer, the kernel abstraction with a measured backend choice, and CI on all three OSes. The only modeling commands are primitives, there to exercise the pipeline.

**Decisions to record as ADRs (ask the user where marked)**

- ADR-0001 Project licence: **ask the user** (see Reference projects and licensing).

- ADR-0002 UI toolkit: egui + egui_dock versus Slint versus iced, judged on docking, ribbon feasibility, text quality, IME and accessibility, wgpu integration, and binary size. Build a one-day prototype ribbon and navigator in the top two before deciding.

- ADR-0003 File container: SQLite unless the prototype finds a blocker.

- ADR-0004 Geometry kernel: chosen from the bake-off results below.

- ADR-0005 Plugin interface: WASM components with a versioned WIT world.

**Deliverables**

- Workspace with every crate from the architecture section. Applications not yet built register with the registry and show their ribbons with commands greyed out and the tooltip "Planned for Stage N."

- pxp-doc: object store, typed unit-aware properties, expression parser and evaluator with dependency tracking, dependency graph with a parallel recompute scheduler, transactions, unlimited undo/redo within a memory cap, change bus, snapshots.

- pxp-format: new, open, save, save as, schema migration framework, recovery journal, rolling backups, advisory lock, fuzz target.

- pxp-render: wgpu viewer with the NX mouse grammar, GPU picking, edge and silhouette display, a section plane, view triad, named and standard views. On machines without a usable GPU, fall back to the OS software rasteriser (WARP on Windows, llvmpipe on Linux) with a warning.

- pxp-ui: data-driven ribbon built from CommandSpec, resource bar with dockable navigators, top border bar with selection filter and snap toolbar, dialog framework (selection steps, OK/Apply/Cancel, MB2 = OK, on-screen input boxes), cue and status lines, Information window, Command Finder, roles, layered customer defaults, remappable shortcuts, light and dark themes.

- **Kernel bake-off** harness and corpus of at least 200 parts: SolidCraft's reference parts, public-domain STEP sets such as the NIST MBE PMI test models, and synthetic stress cases (coincident faces, tangent booleans, tiny edges, near-degenerate blends). Report success rates, speed and cross-OS determinism per backend.

- Primitives (block, cylinder, cone, sphere), a default datum CSYS and WCS, all through the kernel trait, to prove the pipeline end to end.

- pxp-cli with an MCP server running the same commands headless, and journaling: record a session, replay it, compare resulting geometry hashes. Journals become regression tests.

- CI matrix (Windows, macOS arm64 and x86_64, Linux) running fmt, clippy, tests, cargo deny, a short fuzz smoke test, benchmarks and packaging. A nightly job runs long fuzzing and the full corpus.

- Local crash reporter (no telemetry) and autosave.

**Verification packet checklist for the user**

- ☐ Installs and launches from the CI packages on Windows, macOS and Linux.

- ☐ New part from mm and inch templates; create a block through its dialog; save, close, reopen; geometry hash identical.

- ☐ Kill the process mid-session; on relaunch, recovery is offered and restores the last committed action.

- ☐ 100 undo and redo steps behave correctly.

- ☐ Ctrl+M, Ctrl+Shift+D and Ctrl+Alt+M switch applications with the part still open; unbuilt commands are greyed with a tooltip.

- ☐ Orbiting the 5-million-triangle benchmark model holds the agreed frame rate on the reference GPU.

- ☐ Fuzzers ran for the agreed number of CPU-hours with no crash or hang.

- ☐ Kernel bake-off report delivered, with the agent's recommendation and the evidence for it.

**Gate:** stop here. Do not begin Stage 1 until the user replies "Stage 0 approved."

## Stage 1 — Modeling (CAD, including assemblies)

Stage 1 builds the Modeling application, assemblies included, in five sub-stages, each ending with a short demo packet for the user; the full CAD gate comes after 1E, and Drafting may not start until the user approves it.

**Interpretation to confirm with the user at the start of Stage 1:** "CAD modeling should be calculated off of sheets, edges and vertexes" is read as: every body is a boundary representation (vertices, edges, faces, shells); sheet (surface) bodies and wire bodies are first-class citizens alongside solids; and every command takes faces, edges, vertices and sheets as inputs through NX-style selection intent. If the user meant something else, adjust before coding.

### 1A — Coordinate systems, datums, curves and sketches

- **Coordinate systems:** WCS (dynamic reorientation, save and recall), multiple datum CSYS (absolute, offset, three-point, on face, on curve, from existing), a CSYS dialog with an inferred mode.

- **Datums:** planes (inferred, at distance, at angle, bisector, three points, tangent, normal to curve, on curve), axes, points, point sets; an NX-style Point dialog with inferred snapping.

- **Curves:** line, arc and circle, studio spline (through points or poles, degree, G0 to G3 end continuity), conics (ellipse, parabola, hyperbola, general conic), helix, law curve driven by expressions, text as font outlines, offset curve, offset in face, project, intersection, isoparametric, bridge, combined projection, wrap and unwrap, extract (edges, silhouettes), join, trim, divide, curve fillet and chamfer.

- **Sketcher:** sketch on plane or on path. Inferred constraints while drawing. Geometric constraints: coincident, point on curve, horizontal, vertical, parallel, perpendicular, tangent, equal, concentric, symmetric, collinear, midpoint, fixed. Dimensions (linear, angular, radial, diameter, perimeter) that drive named expressions. Driving versus reference dimensions. Associative projection of model edges. Degree-of-freedom arrows and NX-style colour states for under-, fully and over-constrained geometry. Conflict diagnostics that name the conflicting constraints. Auto-dimension.

- **Solver:** start from SolidCraft's constraint solver. Decompose the constraint graph into independent clusters, solve each with Newton–Raphson plus Levenberg–Marquardt damping, and prefer solutions closest to the previous state so sketches never flip unexpectedly. Comment the algorithm fully, per the documentation rules.

### 1B — Parametric solid features

- **Profile features:** extrude (value, until next, until selected, through all; draft; offset; boolean option), revolve, sweep along guide, swept (sections plus guides), tube, through curves (loft), through curve mesh, rib, groove, slot, emboss.

- **Booleans:** unite, subtract, intersect, with keep-target and keep-tool options.

- **Detail features:** edge blend (constant and variable radius, setbacks, stop short), face blend, chamfer (symmetric, asymmetric, offset and angle), shell (with per-face thickness), draft (from plane, from edges, tangent to faces, to parting edge), offset face, thicken, trim body, split body, divide face.

- **Patterns and copies:** pattern feature, geometry and face (linear, circular, along curve, polygon, spiral, general, reference), mirror feature and body, extract geometry (associative copy).

- **Synchronous (direct) modeling:** move face, pull face, offset region, replace face, delete face, resize blend, make coplanar, make coaxial. Essential for editing imported bodies that have no history.

- **Hole feature:** simple, counterbore, countersink, tapered and threaded holes, and hole series through several bodies. Standards tables for ISO metric coarse and fine, UNC, UNF, UNEF, NPT, NPTF, BSPP and BSPT, plus drill-size tables, tip angle, depth limits, start and end chamfers, and placement by points or sketch. Each hole stores its standard callout as data that Drafting and CAM read later.

- **Thread feature:** *symbolic* threads (lightweight display plus full thread data for drawing callouts and CAM threading) and *modeled* threads (true helical geometry from the standard profile, such as the ISO metric basic profile and the Unified profile), left or right hand, multiple starts, tolerance class. A threaded hole creates a symbolic thread by default and converts to a modeled one on request.

- **Parametric editing:** every parameter is an expression. Edit with rollback, suppress, reorder, feature groups, and failure handling that shows exactly which feature failed and why, keeping the rest of the model usable.

### 1C — Sheets, surfaces and analysis

- **Sheet bodies:** bounded plane, ruled, through curves, through curve mesh, swept, studio surface, N-sided, transition, offset and variable offset, extension, fill, face blend between sheets, trim and extend, untrim, sew and unsew, patch, enlarge, pole editing (X-form).

- **Topology tools:** sew sheets into a solid, thicken sheets, extract faces, split and merge faces, heal small edges and gaps, with a report of every change.

- **Analysis:** measure (distance, angle, length, area, volume, mass) with optional associative measurement features; deviation, curvature combs, zebra and highlight lines, draft analysis, section analysis, minimum radius, check geometry (validity), mass properties with material density, interference.

### 1D — Assemblies, section views, display and layers

- **Assemblies:** components referencing .pxp files (embedding optional); add, move and replace component; assembly constraints (touch/align, concentric, distance, angle, parallel, perpendicular, centre, fit, fix, bond); arrangements; reference sets (Entire Part, Model, Empty, custom); work part versus displayed part; in-context editing; associative interpart links (WAVE-style geometry links); component patterns; exploded views; interference and clearance checks; partial loading of large assemblies.

- **Section views (Modeling):** dynamic clipping planes and box, capped section faces with colour, several planes at once, saved per model view.

- **Object display:** Edit Object Display (Ctrl+J) on bodies, faces, edges, curves, sketches, datums and features: colour (palette plus true colour), translucency, line font and width, partial shading. Display state saves with the part.

- **Layers:** 256 layers with named categories, work layer, visible/selectable/invisible states, Move to Layer, Copy to Layer.

### 1E — Data exchange

- **Import (best effort):** STEP AP203/214/242, IGES, STL, OBJ, 3MF, glTF, PLY, DXF, FreeCAD .FCStd, SolidCraft designs, Rhino .3dm through openNURBS if its licence fits ADR-0001. Each importer lives in its own crate and reports exactly what it could and could not translate.

- **Native proprietary formats** (NX .prt, SOLIDWORKS, CATIA, Creo, Inventor, Parasolid, JT, DWG) need licensed SDKs or licence-compatible libraries. Design the Importer plugin interface so they can be added later. Never reverse-engineer a format in breach of a licence agreement, and tell the user which formats are blocked and why.

- Imported bodies arrive without history and are edited through synchronous modeling. Optional feature recognition (holes, blends, chamfers) is backlog.

- **Export:** STEP, DXF and PDF with the default rules in the file-format section, plus IGES, STL, 3MF and glTF.

### Stage 1 verification gate

- ☐ Five reference parts the user approves in advance, built from scratch in the app: a machined bracket with counterbored and threaded holes, a turned shaft with grooves and modeled threads, a lofted and surfaced part, a shelled and drafted housing with blends, and a 10-component constrained assembly.

- ☐ Editing an early sketch dimension in each part rebuilds every downstream feature, or flags a broken reference; nothing re-attaches to the wrong face. A naming torture suite (add and remove sketch edges, reorder features, change booleans) passes in CI.

- ☐ Save, close and reopen give identical geometry hashes on all three OSes.

- ☐ STEP round-trip: volume, surface area and centroid match the oracle (SolidCraft headless or OCCT) within 1e-6 relative.

- ☐ At least 95 % of a 100-file public STEP sample imports without error, with a report on the rest.

- ☐ Edit Object Display changes the colour of a single face, an edge and a whole body, and the change survives save and reload.

- ☐ An NX user completes the user's scripted modeling exercise without help, timing recorded.

- ☐ Performance budgets from Stage 0 still hold.

**Gate:** stop. Drafting begins only after the user replies "Stage 1 approved."

## Stage 2 — Drafting

Drafting produces multi-sheet drawings, inside the part or in a master-model part, whose views, dimensions and annotations stay associative to the model. Start only after the user has approved Stage 1.

**Sheets, templates and standards**

- Sheet sizes ISO A0 to A4, ANSI A to E and custom; first- or third-angle projection; any number of sheets per part, listed under Drawing in the Part Navigator.

- Sheet templates are themselves .pxp template files the user edits like any drawing: border, zone letters and numbers, title block, revision block, general notes. Title-block fields are attribute-driven (part number, material, mass, revision, drawn by, date, scale, "sheet n of m") and update automatically.

- Templates and drafting standards come from layered customer defaults (site, group, user).

- A **drafting standard** is a saved settings set (ISO, ASME, DIN, JIS, GB, or custom) controlling arrowheads, text height, line widths, precision, tolerance display, dimension placement and symbol style. Users edit and save their own.

- Customisable default font (TrueType/OpenType embedded on export, plus single-stroke CAD fonts for DXF), default sizes, line weights and border styles.

**Views**

- Base view from any model view at any scale; projected views; pictorial (isometric, trimetric); auxiliary views.

- Section views: full, half, offset (stepped), aligned (revolved), point-to-point, broken-out, removed and revolved sections. Hatch patterns follow the material; components such as fasteners and shafts can be excluded from sectioning.

- Detail views (circular or rectangular boundary), broken views, exploded assembly views from arrangements.

- Hidden-line, silhouette, smooth-edge and virtual-intersection settings; thread display per the selected standard from symbolic thread data; automatic centrelines, centre marks and bolt circles.

- View labels and scale labels; view alignment; out-of-date markers with automatic or on-demand update.

- Hidden-line removal is computed exactly from the B-rep (never rasterised), parallelised across cores, so PDF and DXF output is crisp vector geometry.

**Dimensions**

- Rapid (inferred) dimension plus linear (horizontal, vertical, aligned, perpendicular, point-to-point), radial, diameter, angular, arc length, thickness and chamfer dimensions.

- **Ordinate dimensioning:** ordinate origin and baselines, horizontal and vertical sets, margins, jogged leaders, automatic ordinate placement.

- Chain and baseline sets; hole and thread callouts generated from Hole and Thread feature data (for example M10x1.5-6H with depth); hole tables and coordinate tables referenced to an origin.

- Tolerances: limits, bilateral and unilateral plus/minus, ISO 286 fits (such as H7/g6) with values looked up automatically, basic, reference and inspection dimensions, dual units.

**GD&T and annotation**

- Feature control frames per ASME Y14.5-2018 and ISO 1101: every geometric characteristic each standard defines, material-condition and zone modifiers (MMC, LMC, projected tolerance zone, free state, tangent plane, statistical, unequally disposed profile, continuous feature, all around, all over, between), composite and multi-segment frames.

- Datum feature symbols, datum targets (point, line, area), datum reference frames with modifiers. A checker flags common errors for the selected standard.

- Surface texture symbols (ISO 21920 family, ASME Y14.36), weld symbols (AWS A2.4, ISO 2553), edge conditions (ISO 13715), notes with live attribute references, leaders with multiple segments, ID symbols and balloons, centrelines, tables and images.

- **PMI:** 3D annotations created in Modeling can be inherited into drawing views, and semantic PMI exports to STEP AP242. Build PMI in Modeling as part of this stage, because Drafting is its first consumer.

**Associativity, auto-dimensioning and overrides**

- Every dimension references model topology through persistent names. When the model changes, values update. If a referenced edge disappears, the dimension becomes *retained*: drawn in a distinct colour, listed in a report, with a repair action. It never silently re-attaches.

- **Feature-driven dimensions:** a command inherits sketch dimensions and feature parameters (extrude depth, hole diameter, pattern count) as drawing dimensions linked two-way to their expressions. Editing one on the drawing changes the model after confirmation, and undo restores both.

- **Auto-dimension:** proposes a dimension scheme per view from the feature data (ordinate or chain, per the user's setting), avoids duplicates across views, and places text without overlaps. The user can delete, move or override any of it.

- **Overrides:** the user may override displayed text or value ("TYP", a manual value). Overrides are stored separately from the true value, shown with a visible indicator, and listed in an overrides report, so a drawing can never silently disagree with the model.

**Assemblies and BOM**

- Assembly drawings with exploded views and section exclusions.

- Parts lists generated from the assembly structure: top level, all levels or leaves only; columns from attributes (part number, description, material, quantity, mass); rolled-up quantities; sorting; custom columns; several parts lists per drawing; export to CSV.

- Auto-balloon tied to components and parts-list rows; balloons renumber when the list re-sorts.

- Revision tables and general tabular notes.

**Output**

- PDF: vector, one page per sheet, fonts embedded, true line weights, layers as optional content, PDF/A option.

- DXF per sheet with layers, blocks for title blocks, text kept as text, correct scale.

- Print and plot with scaling, colour or monochrome, line-weight tables.

### Stage 2 verification gate

- ☐ Drawings of all five Stage 1 reference parts on a template the user customised (own title block, font and border).

- ☐ One multi-sheet drawing containing base, projected, full, half and offset section, detail, broken, auxiliary and isometric views.

- ☐ Ordinate dimensions with baselines, a hole table, and thread callouts generated from features.

- ☐ GD&T frames and datum targets reproducing a sample drawing the user supplies.

- ☐ Change a hole diameter and add a feature in Modeling: views and dimensions update; lost references show as retained; overridden dimensions are flagged.

- ☐ Assembly drawing with BOM and auto-balloons; quantities update when a component is added.

- ☐ Editing a feature-driven dimension on the drawing changes the model, and undo reverts both.

- ☐ PDF opens in common viewers as vector with embedded fonts; DXF opens in an independent DXF viewer with correct layers and scale.

**Gate:** stop. Manufacturing begins only after the user replies "Stage 2 approved."

## Stage 3 — Manufacturing core: subtractive and additive operations

Stage 3 builds the core of the Manufacturing application: NX's Operation Navigator with parent-group inheritance, subtractive milling, turning and mill-turn operations, solid-driven additive operations, and toolpath verification with tools, holders and fixtures. Machine Simulation, the third part of Manufacturing, needs a post processor and a machine kit, so it is completed in Stage 4 alongside Post Processor Generation and Machine Kit Builder. This stage ends with verified toolpaths, not with G-code for production. Start only after the user has approved Stage 2.

### Data model: four parent-group trees, as in NX

- **Program groups:** NC_PROGRAM, then programs, then operations; the order is the execution order.

- **Tool groups:** machine, then turret, pocket or carousel, then tools with holders.

- **Geometry groups:** MCS, then WORKPIECE, then sub-geometry (cut area, boundaries, hole groups; turning workpiece and avoidance for lathes).

- **Method groups:** rough, semi-finish, finish, drill and custom methods carrying part stock, inside and outside tolerances, feeds and speeds and default cut patterns.

- Every operation has exactly one parent in each tree and **inherits** parameters from its parents unless overridden. Each field shows where its value comes from ("inherited from MILL_FINISH"). Moving an operation to another parent re-inherits and marks it out of date.

- Setup and operation templates (planar milling, contour milling, multi-axis, drilling, turning, mill-turn, additive) define default groups and operations, as NX template parts do.

### Operation Navigator

- Four views: **Program Order**, **Machine Tool**, **Geometry** (the workpiece view) and **Machining Method**. Drag and drop between parents in any view.

- Configurable columns (status, tool, time, path length, stock, tolerance, feed; machine-code column in Stage 4) and status icons for *regenerate needed*, *complete* and *accepted*.

- MB3 menu: generate, replay, verify, list, copy, paste, paste inside, transform (translate, rotate, mirror, pattern operations), *generate all out-of-date*.

### Multiple parts and coordinate systems

- Any number of MCS objects, each with its own fixture offset (standard and extended work offsets), clearance plane, lower limit and output options.

- Any number of WORKPIECE groups under each MCS, each with its own part, blank and check geometry.

- Many parts machined independently on one setup (fixture plate, tombstone): each part gets its own operations, or operations are instanced across parts, sequenced by part or by tool.

- Setups live either in the part or in a master-model setup part that references the design part, fixtures and stock as components.

### Workpiece definition

- **Part geometry:** bodies, faces or facets.

- **Blank (initial stock):** bounding block with offsets, cylinder, offset from part, a body, an STL, or the previous operation's in-process workpiece (IPW).

- **Check geometry:** fixtures, clamps, vise jaws, with its own stock to leave.

- Each operation produces an **IPW** (dexel or voxel model, GPU-accelerated with a CPU reference) that later operations use for rest machining, stock recognition and collision checks.

### Tools and machines

- Tool library: flat, ball, bull-nose, chamfer, taper, barrel/lens/oval, T-slot, lollipop, face mill, thread mill, engraving; spot and twist drills, taps, reamers, boring bars, counterbores; turning inserts by ISO shape, grooving, threading, parting and button tools; live tools.

- Holders and extensions, gauge length, tool numbers, length and radius offset registers, a feeds-and-speeds library by material (surface speed, chip load). ISO 13399 import is backlog.

- **Machine kits:** import a machine kit in the folder format specified in Stage 4 (build the reader now) and ship read-only generic kits (3-axis, 5-axis, lathe, mill-turn) for testing. The Machine Tool view shows the kit's pockets; tools can be dragged into the carousel or turret and are shown there. Kinematic limits from the kit constrain multi-axis tool-axis solutions. Creating and editing kits is Machine Kit Builder's job in Stage 4.

### Operations (minimum credible set)

- **2.5D milling:** face, planar (profile, follow part, follow periphery, zig-zag, trochoidal), pocket with islands, contour with lead-in/out, chamfer, engraving, thread milling, slot milling.

- **Hole making:** spot, drill, peck, deep drill, tap, ream, bore, counterbore, countersink, with canned cycles. Holes and threads are recognised from Hole and Thread feature data (feature-based machining).

- **3-axis:** adaptive (constant-engagement) roughing, rest roughing, Z-level steep finishing, area finishing for shallow regions (raster, radial, spiral, constant stepover, streamline), pencil and corner clean-up, rest finishing.

- **Stock recognition roughing:** roughs only where material actually remains in the current IPW (from all prior operations, a defined stock or a scanned stock), recomputing engagement against that real stock, similar to stock-aware rest roughing in Mastercam.

- **Peel milling:** in the style of Mastercam's Peel Mill, peels material layer by layer between two selected chains with constant-climb trochoidal motion and fast back-feed moves while the tool is disengaged. Parameters: engagement or stepover, maximum stepdown, micro-lift, back-feed rate, helical entry.

- **3+2 positional** machining with a tool axis per operation.

- **5-axis simultaneous:** variable-axis contouring (surface, curve, boundary and streamline drive methods; tool axis by lead/tilt, toward point or line, normal to drive, interpolated vectors), swarf, multi-axis deburring; collision avoidance by tilting away from holder and check geometry.

- **Rotary milling (4th/5th axis):** rotary roughing and finishing around A or B, wrapping 2D geometry onto cylinders, helical and rotary-floor strategies, indexed positions.

- **C-axis substitution** (an operation option where the machine supports it): (a) axis substitution, where the path is computed on unwrapped geometry and Y is mapped to rotation about a wrap diameter; (b) polar-coordinate interpolation for face work on lathes; (c) cylindrical interpolation for OD work; (d) point-to-point X/C output with no control interpolation mode. Stage 3 computes the geometry and simulates it; Stage 4 posts emit each controller's codes.

- **Turning:** facing, OD and ID roughing (with rest), finishing, grooving (OD, ID, face), threading (single and multi-start, tapered, from Thread feature data), parting, centreline drilling and tapping; revolved IPW; main or sub spindle and turret selection.

- **Mill-turn:** turning and milling in one program, main and sub spindle, part transfer, B-axis head milling, synchronisation points (output in Stage 4).

- **Shared engine:** cut levels, separate floor and wall stock, tolerances from the method, a non-cutting-move engine (engage, retract, transfer; helix, ramp, plunge; collision-safe transfers), arc fitting, corner smoothing, toolpath editing (trim, transform, delete segments), statistics (time, length), display coloured by feed and with tool-axis vectors.

- Generation runs in parallel across operations and across levels inside an operation, can always be cancelled, and reports progress in the navigator.

### Additive manufacturing (PrusaSlicer-inspired, solid-driven)

- **Auto slice:** one-click slicing with printer, filament and print profiles, and G-code flavours for common firmwares. Layers are computed by intersecting the exact B-rep with layer surfaces rather than slicing a tessellated STL, which keeps dimensions exact and keeps the operation associative to the model. Tessellation is a fallback only.

- **Slicer features:** perimeters, infill patterns (rectilinear, grid, gyroid, honeycomb, lightning and more), top and bottom skins, seams, bridge and overhang detection, grid and tree supports, brim and skirt, adaptive variable layer height, per-region modifiers defined by bodies, multi-material, ironing, sequential printing.

- **Custom additive operations** in the Operation Navigator: planar operations on chosen regions with their own layer height and pattern; **non-planar** operations (conformal layers following a surface, curved top skins); multi-axis deposition (DED or wire-arc) with tool-axis control; hybrid sequences where deposition and machining share one IPW.

- Additive "tools" are nozzles or deposition heads with bead width, bead height and material.

- Deposition simulation, layer preview by line type, time and material estimates.

- If ADR-0001 is not AGPL-compatible, implement from published algorithms and use PrusaSlicer only to compare results.

### Toolpath verification in this stage

- Toolpath replay with tool and holder; dynamic material removal and addition on the GPU with a CPU reference; gouge checks against the part; collisions of tool, holder and shank with IPW, fixtures and check geometry; rapid-into-stock detection; final IPW versus part deviation map (overcut and undercut).

- Speed control, single step, breakpoints at events, a collision report with jump-to.

- **Not in this stage:** Machine Simulation. It runs real post output through a machine kit and a virtual controller, so it is completed in Stage 4 with Post Processor Generation and Machine Kit Builder.

- Output in this stage is a neutral toolpath listing. A fixed ISO writer marked *test only* may be added only if the user asks for one.

### Stage 3 verification gate

- ☐ Prismatic part: MCS and WORKPIECE, face, adaptive rough, stock recognition rough after a large tool, Z-level, area finish, drilling and tapping recognised from Hole features, chamfer. All four navigator views show the correct hierarchy, and dragging an operation to another method group changes its stock and tolerance through inheritance.

- ☐ Fixture plate with four parts under two MCS (two work offsets), sequenced by part and then by tool.

- ☐ Peel milling a slot between two chains; simulation shows constant engagement and back-feed moves.

- ☐ Rotary roughing plus wrapped engraving on a 4th axis.

- ☐ C-axis substitution computed and simulated for a face hex (polar) and an OD slot (cylindrical) on a lathe with live tooling.

- ☐ 5-axis swarf on a tapered wall; a deliberately short tool triggers a holder collision report.

- ☐ Turning the Stage 1 shaft: face, rough, finish, groove, thread; mill-turn flats plus part transfer.

- ☐ Additive auto-slice of a reference part compared with PrusaSlicer on the same model (layer count, path length, time estimate within agreed limits), plus a custom non-planar top-skin operation on a curved part.

- ☐ Changing a hole diameter in Modeling marks exactly the affected operations *regenerate needed*; regenerating gives correct paths.

**Gate:** stop. Post Processor Generation, Machine Kit Builder and Machine Simulation begin only after the user replies "Stage 3 approved."

## Stage 4 — Post Processor Generation, Machine Kit Builder and Machine Simulation

Stage 4 adds two applications and completes a third. Machine Kit Builder defines machines, Post Processor Generation builds layered Rust post processors for them, and Manufacturing gains Machine Simulation, which runs the post's NC code through the machine kit and a virtual controller. Start only after the user has approved Stage 3.

**How the three fit together**

- Machine Kit Builder writes a machine kit: kinematics, graphics, tool changer and setup files.

- Post Processor Generation reads the kit as layer 1 of a post, adds the controller as layer 2 and capability and customization layers above it, and compiles the post to WebAssembly.

- Manufacturing posts a program through that post, and Machine Simulation runs the resulting NC code through a virtual controller driving the kit's kinematics and graphics.

- Deviations between the simulated motion and the CAM toolpath are reported back in the Operation Navigator.

### Machine folder (one folder per machine)

One folder holds the kit, the controller settings and the post; copying it moves the whole machine to another computer or OS.

```text
machines/
  haas_umc_5ax/                   # one machine = one folder
    kit/                          # written by Machine Kit Builder
      machine.toml                # identity, kinematic chain (axes, limits,
                                  # pivots, offsets), spindles, channels, tool
                                  # changer, junctions, capabilities
      graphics/                   # STEP or glTF per kinematic component
      setup/                      # default fixtures, stock, carousel load-out,
                                  # work offsets, setup templates
    controller.toml               # virtual-controller dialect, options, rapid
                                  # rates, acceleration and jerk for cycle time
    post/                         # written by Post Processor Generation
      stack.toml                  # the layer stack: numbers, names, enabled
      layers/
        001-machine.toml          # generated from kit/machine.toml
        002-controller.toml       # flattened from the controller library
        003-mill-3axis.toml       # capability layers ...
        005-mill-5axis.toml
        005-mill-5axis.rs         # optional Rust hooks for any layer
        090-shop-standard.toml    # ... and customization layers
        099-user.toml
      Cargo.toml                  # the post is a real Rust crate,
      src/lib.rs                  # generated from the resolved stack + hooks
      build/post.wasm             # compiled, portable artifact (wasm32-wasip2)
    tests/                        # reference programs and golden NC output
```

Shared libraries sit beside the machines: library/controllers/ (ISO, Fanuc, Siemens, Okuma, Mazak, Haas) and library/layers/ (capability and shop layers). A post copies the library layers it uses into its own folder with a version stamp, so every machine folder stays self-contained.

**How posts are built and run**

- Saving in Post Processor Generation resolves the layer stack, generates the Rust source plus the user's hook functions, compiles it to a WebAssembly component, and stores post.wasm in the folder.

- Posts execute in a wasmtime sandbox: no file or network access beyond the output stream the host provides, so a buggy or malicious post cannot touch the user's files, and the same post.wasm runs unchanged on Windows, macOS and Linux.

- Users without a Rust toolchain can run every post and make data-only edits. Compiling custom hook code needs the toolchain; the app detects it and offers to install it, with the user's consent.

**Why WebAssembly:** your requirement is posts written in Rust and built on demand. Native Rust plugins have no stable ABI and would need rebuilding per OS and app version. A WASM component is built once, is portable, is sandboxed and can be hot-reloaded while Post Processor Generation previews output.

### Machine Kit Builder (application)

- **Kinematic chain editor:** a tree of components (base, linear axes, rotary axes, spindles, turrets, sub-spindle, tailstock) with axis direction, zero position, travel limits, maximum velocity and acceleration, continuous or limited rotation, and pivot offsets. Templates for table-table, head-table and head-head 5-axis mills, lathes and mill-turns.

- **Graphics:** import STEP, glTF or STL per component, from Modeling or from files; align by datum CSYS; simplify meshes for fast collision checks.

- **Tool changers:** carousels, chains and turrets with pocket count and positions, tool-change position, live-tool stations and B-axis heads. Manufacturing's Machine Tool view shows these pockets.

- **Junctions:** named mounting points used by CAM and simulation, such as the spindle gauge point, table or chuck zero, and turret stations.

- **Collision and limits:** collision pairs, clearances and soft limits; a jog panel to move every axis and test motions; a reach check for a sample toolpath.

- **Controller binding:** choose a controller from the library; this writes controller.toml and sets the template for layer 2 of the post.

- **Kit checker:** flags unconnected components, missing limits, inverted axes and unnamed junctions.

- **Generic kits:** 3-axis VMC, 5-axis table-table and head-table, 2- and 3-axis lathes, and a mill-turn, matching the default posts. Stage 3 ships these kits read-only so Manufacturing can be tested before this application exists.

- Any kit change marks that machine's posts for rebuild and clears its verified flags.

### Layered post architecture

A post processor is a numbered stack of layers: layer 1 is the Machine, layer 2 the Controller, and layers 3 and above add machine capabilities or any user customization. When two layers set the same customization to different values, the higher-numbered layer wins, so a value set in layer 10 overrides the same setting in layer 1.

*[diagram] post layer stack · higher number wins*

In this example the tool-change block is set in layer 2 and again in layer 99, so the post outputs layer 99's version; switch layer 99 off and layer 2's version returns.

| Layer | Holds | Contents |
| --- | --- | --- |
| 1 | Machine | Axis names and order, travel limits, rotary configuration, home and tool-change positions; generated from the machine kit |
| 2 | Controller | G- and M-codes, address formats, canned-cycle syntax, macro style, TCPM, polar and cylindrical mode codes; flattened from the controller library (for example Haas derives from Fanuc, which derives from ISO) |
| 3 to 19 | Milling capability | Suggested: 3 = 3-axis, 4 = 3+2 positioning, 5 = full 5-axis simultaneous |
| 20 to 29 | Turning | Suggested: 20 = 2-axis turning, 21 = C-axis live tooling |
| 30 to 39 | Mill-turn | Sub-spindle, part transfer, B-axis head, multi-channel sync |
| 40 to 49 | Additive | Extrusion or deposition, temperatures, layer change |
| 50 to 89 | Optional capabilities | Probing, high-speed modes, tool-life management |
| 90 to 98 | Shop customization | Company headers, safe-start blocks, program naming |
| 99 and above | User customization | Any override; the highest number has the last word |

The bands above layer 2 are suggestions, not hard rules: any number of 3 or more can hold a capability or a customization.

**Layer rules**

- **Precedence:** for every customization, the effective value comes from the highest-numbered enabled layer that sets it. A layer that does not mention a setting inherits it from below.

- **What counts as a customization:** anything addressable by a stable key: address formats, words, block templates per event, G- and M-code mappings, variables and defaults, buffers and flush points, user-defined events, output naming, safety settings and event handlers.

- **Event handlers chain:** a higher layer's handler replaces the lower one but may call next() to run the lower handler before or after its own. A user layer can add a line to the controller's tool change without copying it.

- **Explicit unset:** a layer can remove a lower layer's value, which is different from not mentioning it.

- **Validation:** layer numbers are unique within a post, with 1 and 2 reserved. The checker flags overrides that change a value's type or units, capability layers the kit cannot support (a 5-axis layer on a 3-axis kit), and settings an event needs that no layer provides.

- **Inter-compatibility:** capability layers talk only through the event stream and shared variables, never by calling each other, so mill plus turning, or lathe plus additive, is just a different stack.

- **Layer 1 is generated:** it is rebuilt from the machine kit whenever the kit changes. Machine facts are edited in Machine Kit Builder; post-specific overrides of them go in a higher layer.

```rust
/// One layer of a post processor.
///
/// Layers form a numbered stack: 1 = Machine, 2 = Controller, 3 and above =
/// machine capabilities (3-axis, 3+2 or 5-axis milling, turning, mill-turn,
/// additive) and user customizations. When two layers set the same key, the
/// HIGHER number wins, so a value in layer 10 overrides layer 1.
pub struct PostLayer {
    /// Precedence. Unique within one post; 1 and 2 are reserved.
    pub number: u32,
    /// Shown in the layer navigator, e.g. Full 5-axis milling.
    pub name: String,
    /// Machine | Controller | Capability(..) | Custom
    pub kind: LayerKind,
    /// Disabled layers are skipped during resolution; toggling one in the
    /// GUI re-previews the NC output immediately.
    pub enabled: bool,
    /// Settings this layer sets or explicitly unsets. Keys it does not
    /// mention are inherited from lower layers.
    pub settings: BTreeMap<SettingKey, Override>,
    /// Event handlers (Rust hooks) this layer provides.
    pub handlers: BTreeMap<EventKind, HandlerRef>,
}

pub enum Override {
    Set(SettingValue),
    /// Removes the value from all lower layers; not the same as silence.
    Unset,
}

/// Resolve one setting. The stack is sorted by ascending layer number, so we
/// walk it from the top down and stop at the first enabled layer that has an
/// opinion. Returning the layer number lets the inspector and "show where
/// output comes from" name the layer behind every value.
pub fn resolve<'a>(
    stack: &'a [PostLayer],
    key: &SettingKey,
) -> Option<(&'a SettingValue, u32)> {
    for layer in stack.iter().rev().filter(|l| l.enabled) {
        match layer.settings.get(key) {
            Some(Override::Set(v)) => return Some((v, layer.number)),
            Some(Override::Unset) => return None, // removed at this layer
            None => continue,                    // no opinion: look lower
        }
    }
    None
}

/// Event handlers resolve the same way, but a higher handler may delegate to
/// the next-lower one with ctx.next(), so a user layer can wrap the
/// controller's tool change instead of copying it.
pub trait EventHandler {
    fn handle(&self, ev: &Event, ctx: &mut HandlerCtx) -> Result<(), PostError>;
}
```

**Engine**

- The internal toolpath stream becomes events; each event runs its resolved handler chain; handlers build blocks from resolved formats and words; blocks go into named buffers; buffers flush to the NC file.

- **Events:** program start and end, operation start and end, tool change, spindle, coolant, work offset, rapid, linear, circular and NURBS motion, drilling and turning cycles, threading, polar and cylindrical mode on and off, rotary positioning, TCPM on and off, feed-mode change, channel sync, part transfer, additive events (extrude, temperature, fan, layer change), and user-defined events (operator message, optional stop, inserted code) that CAM users attach to operations.

- **Buffers:** user-definable named buffers with chosen flush points, such as a header filled at the end with the tool list and cycle time, look-ahead for next-tool pre-call, and block reordering.

- **Variables:** system variables (positions, feed, tool, work offset, every modal state), user variables (typed, with defaults, optionally shown to CAM users as post properties) and computed variables.

- **Formats, words and blocks:** leading and trailing zeros, decimals, modal or non-modal words, scaling, inch or mm, word order, conditional words, sequence numbers.

- **Motion:** arcs as IJK or R with quadrant splitting and helices, NURBS where supported, multi-axis inverse kinematics from the kit, singularity handling, rotary limits and unwinding, retract and re-position, TCPM versus computed axis output, inverse-time or per-minute feed.

### Post Processor Generation (application, like NX Post Configurator but simpler)

- **Layer navigator:** the stack with numbers, names, kinds and enable switches; add, renumber, duplicate, import from the library, update from a newer library version with a diff, and delete (except layers 1 and 2).

- **Setting inspector:** each setting shows its effective value, the layer it comes from, and the lower-layer values it overrides, struck through. Edits go to the active layer; *move to layer N* and *revert to lower layer* are one click each.

- **Event tree:** selecting an event shows its blocks; drag words into blocks, add conditions on variables, edit formats, and manage buffers and variables in side panels. Any NC output can be changed from the GUI, because every word comes from an event, block, buffer or variable the GUI edits.

- **Live preview** on a sample program with *show where output comes from*: clicking any NC line names the event, block, variable and layer that produced it.

- **Layer diff:** compare NC output with a layer switched on and off, or between two layers.

- **New-post wizard:** pick a machine kit (layer 1) and a controller (layer 2); capability layers are suggested from the kit's capabilities.

- Golden-output diff against tests/, a Rust hook editor with inline compile errors, and version history with rollback.

### Default posts (30, all assembled from library layers)

| Machine type | Controllers |
| --- | --- |
| 3-axis mill | ISO G-code, Fanuc, Siemens, Okuma, Mazak, Haas |
| 5-axis mill (table-table and head-table variants) | ISO G-code, Fanuc, Siemens, Okuma, Mazak, Haas |
| 2-axis lathe (X/Z) | ISO G-code, Fanuc, Siemens, Okuma, Mazak, Haas |
| 3-axis lathe (X/Z/C live tooling, polar and cylindrical interpolation) | ISO G-code, Fanuc, Siemens, Okuma, Mazak, Haas |
| Mill-turn (main and sub spindle, B-axis head, Y axis) | ISO G-code, Fanuc, Siemens, Okuma, Mazak, Haas |

- Each default post is a stack: layer 1 from the matching generic kit, layer 2 from the controller library, and library capability layers (for example 3, 4 and 5 for the 5-axis mill; 20 and 21 for the 3-axis lathe; 3, 4, 5, 20, 21 and 30 for the mill-turn).

- "ISO G-code" means neutral ISO 6983 / RS-274 output. Mazak posts target EIA/ISO mode; Mazatrol conversational is out of scope.

- Each post ships with a matching virtual-controller dialect, a generic machine kit, and reference programs with golden output.

- Every default post carries a header comment and a first-run dialog stating that it is generic and must be proven on the user's own machine (single block, reduced rapid and feed override, air cut) before production.

### Machine Simulation (part of Manufacturing)

- Machine Simulation is a Manufacturing command, run from the Operation Navigator, but it needs two inputs from the other applications: a machine kit from Machine Kit Builder and a compiled post from Post Processor Generation. Without both, the command is greyed out and says which one is missing.

- **The loop:** CAM toolpath, then the layered post (WASM), then the NC file, then the virtual controller, then the kit's kinematics, then material removal and collision checks with the Stage 3 engine and the kit's graphics.

- **Virtual controller:** one G-code interpreter per dialect: modal groups; canned drilling and turning cycles (Fanuc-style cycles, Siemens CYCLE calls, Okuma and Mazak equivalents); work offsets; tool length and radius compensation with look-ahead; tilted work planes; TCPM; polar and cylindrical interpolation; macro languages and control flow (Fanuc-style # variables, Siemens R parameters, Okuma common variables, Haas macros); subprograms; inch/mm; absolute/incremental; dwell; M-codes mapped to kit actions (tool change, spindle, coolant, sub-spindle transfer); multi-channel wait codes.

- A deviation report compares the simulated tool tip with the CAM toolpath, so post and kit bugs surface before the machine sees them.

- The NC listing scrolls in sync with the simulation (current block highlighted), with breakpoints on blocks and a Machine Code column in the Operation Navigator.

- Cycle-time estimates from the controller's rapid, acceleration and jerk parameters.

- The interpreter is fuzzed. Unsupported codes stop the simulation with a clear message; they are never skipped silently.

### Stage 4 verification gate

- ☐ The user builds a kit for one of their machines in Machine Kit Builder from STEP graphics (kinematic chain, limits, tool changer); jogging every axis moves correctly and stops at its limits; the kit checker passes.

- ☐ All 30 default posts pass golden-file tests, and their output re-simulated in Machine Simulation matches the CAM path within tolerance.

- ☐ **Layer precedence:** a setting defined in layers 1 and 10 outputs layer 10's value; disabling layer 10 restores layer 1's; an explicit unset in a higher layer removes it; the inspector and *show where output comes from* name the right layer each time.

- ☐ The user builds a post for their machine entirely in the GUI: changes the tool change in layer 99 using next() rather than copying it, adds a user-defined event, and adds a header buffer with a tool list. The post rebuilds and the output reflects every change.

- ☐ Composition: adding turning layers to a 5-axis mill post gives correct mill-turn output; adding the additive layer to a lathe post works; adding a 5-axis layer to a 3-axis kit is flagged by the checker.

- ☐ A machine folder copied from one OS to another runs unchanged.

- ☐ Machine Simulation catches a deliberately injected post bug (for example a reversed rotary sign) and a deliberately shortened travel limit in the kit.

- ☐ The user air-cuts at least one program on a real machine of each type they own (the user decides when this is safe).

**Gate:** stop. CAE begins only after the user replies "Stage 4 approved."

## Stage 5 — CAE: FEA, topology optimization and 3D scanning

Stage 5, the last stage of the rollout, completes the CAE suite with three capabilities. Linear static stress and deflection, modal, and harmonic response analysis. Topology optimization, which turns a design space plus loads into a lightweight shape. 3D scanning, which turns point clouds and scan meshes into surfaces and solids. All three stay associative to the model, run on all cores with optional GPU acceleration, and are validated against benchmarks. Start only after the user has approved Stage 4.

**Structure (NX-like, one file type):** the CAE application shows a Simulation Navigator with a FEM object (idealised geometry, mesh, materials, physical properties), a Simulation object (loads, constraints, solutions, results), an Optimization object (design space, objectives, constraints, iteration history) and a Scan object (point clouds, scan meshes, fitted regions). Scan to CAD is part of CAE: its fitted surfaces and solids are published into the part as Modeling bodies, and feature-based reconstruction creates ordinary Modeling features. NX splits FEM and simulation into separate file types; here everything lives in the .pxp, either in the design part or in a master-model .pxp that references it. Crates: pxp-fea, pxp-topopt and pxp-scan (with pxp-pointcloud) are domain libraries driven by pxp-app-cae.

### Pre-processing

- **Idealisation:** associative defeaturing (suppress blends and holes below a size threshold without touching the design model), splitting for mesh control, midsurface extraction for thin parts, symmetry cuts.

- **Meshing** behind a Mesher trait: quadratic tetrahedra (tet10, default for stress) and linear tets, shells (tri3, tri6, quad4, quad8); swept hex and beams are backlog. Size controls on edges, faces and bodies, curvature- and proximity-based refinement, quality checks (aspect ratio, Jacobian, warp). Candidates: a Rust-native mesher or Netgen (LGPL) in the kernel worker process. Bodies mesh in parallel.

- **Mesh-to-geometry association:** nodes and element faces map to B-rep faces through persistent names, so loads re-apply automatically after the geometry changes and the part is remeshed.

- **Materials:** library of isotropic linear-elastic materials (E, Poisson's ratio, density, yield and ultimate strength, damping), assigned per body; orthotropic is backlog.

- **Solver units:** the solver uses one consistent system (for example mm, N, MPa, tonne, s), converted at the boundary from the part's units through the unit newtypes. Inconsistent units are the most common FEA error, so this conversion has dedicated tests.

- **Loads:** force (total or per area) on faces, edges and vertices; pressure; moment; bearing load; gravity and acceleration; centrifugal load; remote force and moment through rigid or interpolating multi-point constraints; harmonic loads with frequency-dependent amplitude and phase.

- **Constraints:** fixed, pinned (cylindrical), enforced displacement, symmetry and frictionless support, elastic support, remote displacement; bonded contact between assembly bodies. Frictional contact is nonlinear and is backlog.

### Solvers

- **Linear static:** parallel element assembly; boundary conditions by elimination (document the method); sparse direct solution with faer's supernodal Cholesky or LDLT for small and medium models; preconditioned conjugate gradient with algebraic multigrid for large models, with sparse matrix-vector products on the GPU (CPU reference kept). Results: displacement, reactions, stress tensor, von Mises, principal and Tresca stress, strain, strain energy, factor of safety against yield, and an error estimate to guide refinement.

- **Deflection analysis:** displacement magnitude and components, maximum-deflection probe, deflection along a path, comparison against a user-set allowable. Large-deflection (geometric nonlinear) analysis is backlog.

- **Modal:** generalised eigenproblem solved by shift-invert Lanczos (or LOBPCG) for natural frequencies, mode shapes and effective mass participation; consistent or lumped mass; free-free rigid-body modes handled.

- **Harmonic response:** modal superposition (fast, reusing modal results) and direct frequency response (one complex solve per frequency, frequencies in parallel). Damping as modal, Rayleigh or structural. Results: amplitude and phase frequency-response plots at probes, response and stress at a chosen frequency, phase-sweep animation.

- The element, assembly and solver framework stays general, so buckling, thermal, nonlinear and fatigue analyses can be added later as plugins.

- Parallel reductions are deterministic, so results agree across OSes within a stated tolerance.

- **Cross-validation:** export the model as a CalculiX input deck and optionally run an installed CalculiX as an external process to compare results. It is never linked in.

### Post-processing

- Contour plots (nodal or elemental, averaged or unaveraged), deformed shape with scale, animation, iso-surfaces, section cuts through results, probes and path graphs, min and max markers, results on selected faces, legend control, comparison between solutions.

- Warnings on results that are mesh-dependent, such as stress singularities at sharp re-entrant corners.

- Report generator (images, tables, inputs, assumptions) and export to VTU for ParaView and CSV.

- Results are stored as chunked blobs in the .pxp, with an option to keep very large results in a referenced side file.

### Topology optimization

- **Workflow (generative design, NX style):** choose the design-space body, mark keep-in regions (bolt bosses, mating faces) and keep-out regions (clearance volumes, tool access), reuse the loads and constraints of a static or modal study, set an objective and constraints, run, review the iteration history, then convert the result to geometry.

- **Methods:** density-based SIMP with density filtering and Heaviside projection as the default; BESO as a second method (prior art: the LGPL beso project, which drives CalculiX from FreeCAD). Level-set methods are backlog. Optimisers: optimality criteria for single-constraint problems and the method of moving asymptotes (MMA) for several constraints.

- **Objectives:** minimum compliance (maximum stiffness) over one or more weighted load cases; minimum mass under displacement or stress limits; maximum first natural frequency, or frequencies kept out of a band.

- **Constraints:** volume or mass fraction, displacement limits at chosen points, stress (an aggregated p-norm, documented in the code), frequency bounds.

- **Manufacturing constraints:** minimum and maximum member size, minimum gap, planar and cyclic symmetry, extrusion (constant section along a direction), pull direction for casting and moulding (one- or two-sided, no undercuts), milling access from chosen tool directions, and an additive overhang angle for a build direction that links to the Stage 3 additive settings.

- **Solver reuse:** every iteration reuses the Stage 5 FEA assembly and solvers. On a regular voxel or hex grid, use matrix-free multigrid-preconditioned CG on the GPU (CPU reference kept): every element has the same stiffness matrix, so global assembly can be skipped, and optimization repeats hundreds of solves on one mesh.

- **Result to geometry:** extract an iso-surface at a user-chosen density threshold and smooth it while keeping keep-in faces exact. Then offer three outputs: (1) a facet body usable directly in Modeling, additive and FEA re-checks, the approach NX calls convergent modeling; (2) an automatically fitted NURBS surface network sewn into a solid, using the same fitting engine as scanning; (3) a translucent reference for the user to remodel parametrically.

- **Automatic re-check:** re-run static (and modal) analysis on the converted geometry and show the stiffness and stress change against the raw optimization result, so the user sees what smoothing and fitting cost.

- **Associativity:** changing the design space, keep-in regions or loads marks the study out of date. Studies are never re-run silently, because a run can take hours.

### Scan to CAD (3D scanning to surfaces and solids)

- **Import:** point clouds in E57 (through the pure-Rust, MIT-licensed e57 crate), PLY, PTS/XYZ text and LAS/LAZ; scan meshes in STL, OBJ, PLY and 3MF; colour, intensity and normals kept when present. Scanner hardware SDKs are proprietary: start with files and add devices later as importer plugins.

- **Large data:** clouds can hold hundreds of millions of points. Store them as chunked, compressed blobs with an octree level of detail, stream them to the GPU for display, and offer to keep very large raw scans in referenced side files.

- **Clean-up:** statistical and radius outlier removal, noise smoothing, voxel and curvature-aware decimation, normal estimation with consistent orientation, cropping by box or selection.

- **Registration:** manual point pairs, global registration (feature descriptors plus RANSAC), fine alignment by point-to-plane ICP, multi-scan alignment, and best-fit or datum-based (3-2-1) alignment to existing CAD or a coordinate system.

- **Meshing and repair:** screened Poisson reconstruction (a Rust implementation exists; Kazhdan's MIT-licensed PoissonRecon is the reference) and ball pivoting; hole filling, self-intersection repair, uniform or curvature-adaptive remeshing. Open3D (MIT) is the general algorithm reference.

- **Segmentation and fitting:** curvature-based region growing splits the mesh into regions; planes, cylinders, cones, spheres and tori are fitted by RANSAC plus least squares, with RMS and maximum error reported per region; freeform regions get NURBS patches fitted to a tolerance.

- **Scan to solid, three routes:** (1) *automatic surfacing* builds a quad patch layout, fits G1-continuous NURBS patches and sews them into a watertight solid, for organic shapes; (2) *feature-based reconstruction* turns fitted primitives and mesh sections into editable sketches, extrudes, revolves and holes, keeping design intent for prismatic parts; (3) *convergent use* keeps the mesh as a facet body for booleans, offsets, FEA and CAM roughing where exact surfaces are not needed.

- **Modeling aids:** section curves through the mesh to drive sketches, fit-curve and fit-surface commands with live deviation display, sketch snapping to fitted primitives, and suggested constraints (parallel, perpendicular, coaxial, symmetric) inferred from the fits.

- **Deviation and inspection:** signed distance colour map between scan and CAD with tolerance bands, pass or fail per region, labelled check points, sections with deviation plots, and a report. This doubles as scan-based inspection of a made part against its design.

- **Associativity:** fitted surfaces keep a link to their source region and fit tolerance. Re-fitting after a tolerance or segmentation change updates downstream features, under the same broken-reference rules as everywhere else.

### Validation suite (automated in CI)

- **Closed-form:** cantilever tip deflection (FL³/3EI) with a mesh-convergence study, thick-walled cylinder (Lamé), plate with a hole (stress concentration), natural frequencies of a simply supported beam, single-degree-of-freedom frequency response.

- **NAFEMS:** linear-elastic benchmarks (LE series, for example LE1), free-vibration benchmarks FV32 and FV52, and forced-vibration benchmarks from R0016 (for example 5H and 13H).

- **Topology optimization:** the classic MBB-beam and cantilever minimum-compliance problems reproduce published layouts and compliance values; results stay mesh-independent with filtering; a symmetry constraint gives a symmetric result and an extrusion constraint a constant section.

- **Scanning:** synthetic scans sampled from known CAD parts, with added noise and outliers, are reconstructed and compared with their source: primitive sizes and positions within tolerance, fitted-solid deviation under the agreed limit, and ICP recovering a known transform.

- Agreement within the tolerances the user sets (suggest 2 % on displacements and frequencies), reported per test.

### Stage 5 verification gate

- ☐ The Stage 1 bracket under a static load: stress and deflection plots, a mesh-convergence table, and agreement with an external CalculiX run within the agreed tolerance.

- ☐ Modal analysis of a plate matches closed-form frequencies.

- ☐ Harmonic sweep shows resonance peaks at the modal frequencies, frequency-response plots, and the effect of damping.

- ☐ Changing a model dimension marks the FEM and simulation out of date; re-solving updates results with loads still on the right faces.

- ☐ The same model in a mm part and an inch part gives identical physical results.

- ☐ The NAFEMS and closed-form suite passes in CI, and GPU and CPU solves agree.

- ☐ A model of about one million degrees of freedom solves within the agreed time on the reference machine.

- ☐ Topology optimization of the Stage 1 bracket for minimum compliance at an agreed volume fraction, with a minimum member size and an additive overhang constraint; the result converts to a fitted solid, the re-check shows the stiffness change, and the solid slices in the Stage 3 additive application.

- ☐ The same bracket optimized with a two-sided pull-direction constraint gives a castable shape with no undercuts.

- ☐ A user-supplied scan (E57 or STL) of a machined part becomes an editable feature-based solid whose dimensions match the real part within the agreed tolerance, with a deviation map and report.

- ☐ A scan of an organic part becomes a watertight NURBS solid by automatic surfacing, within the agreed deviation, and exports to STEP.

- ☐ A raw scan mesh is used as a facet body in a boolean and in a CAM roughing operation without conversion.

- ☐ A large point cloud (size agreed with the user, for example 100 million points) loads, displays at an interactive frame rate and saves within budget.

**Gate:** stop and hand over the final verification packet.

## Agent operating rules, testing and the verification packet

Work in small, reviewable steps, keep docs/STATUS.md true at all times, and never cross a gate without the user's explicit approval.

**Session loop**

- Read docs/MASTER_PROMPT.md, docs/STATUS.md and the latest ADRs.

- Write the session plan (sub-stage, tasks, risks) into STATUS.md before coding.

- Implement in small pull requests (aim for under about 500 changed lines), each with tests and documentation, using conventional commit messages.

- Run fmt, clippy, tests and cargo deny locally; CI must be green on all three OSes before anything counts as done.

- Update STATUS.md: done, in progress, blocked, decisions needed.

**When to ask and when to decide**

- **Ask the user:** the project licence; any ambiguous requirement (several are flagged in this prompt); anything irreversible, such as freezing the .pxp format at 1.0; anything affecting the safety of machine output; dropping or deferring a stated requirement.

- **Decide and record an ADR:** library choices within these constraints, internal APIs, algorithms, data structures.

- Do not ask permission for routine work inside the current stage.

**Definition of done for any feature**

- ☐ Implemented, with code comments meeting the documentation rules.

- ☐ Reachable from the UI and from the CLI/MCP server; undo and redo work.

- ☐ Survives save and reload; associativity behaviour defined and tested.

- ☐ Unit, integration and regression tests pass on Windows, macOS and Linux.

- ☐ User-guide page written; performance within budget.

**Testing standards**

- Unit tests with edge cases for every public function.

- Property tests (proptest) for geometric invariants: vol(A ∪ B) = vol(A) + vol(B) − vol(A ∩ B); rigid transforms preserve volume and area; tessellations are watertight; save then load is the identity.

- Golden-file tests for STEP, DXF, PDF, NC output and drawings, compared structurally, plus rendered-image comparison with a tolerance.

- A regression corpus of parts and recorded journals, run nightly; every bug fix adds a test that reproduces the bug.

- Fuzzing for every parser; Miri on unsafe code; address sanitizer on FFI crates in nightly builds.

- The UI is a thin layer over commands, so most behaviour is tested headless through the CLI. Key dialogs also get screenshot tests.

- Benchmarks with thresholds; cross-OS determinism checks; line coverage of at least 80 % on core crates (document, format, kernel trait, CAM engines, post engine, solvers), measured with cargo-llvm-cov.

**The verification packet (delivered at every gate)**

- Summary of what was built, what was not and why, with measured coverage figures.

- Download links for signed Windows, macOS and Linux builds.

- The gate checklist with step-by-step instructions for each item, plus sample .pxp files.

- Known limitations and open bugs, with severity.

- Decisions the user needs to make.

- Test, benchmark and cross-OS determinism results as tables.

- Updated user guide (one page per command, organised by application as NX help is), developer guide, design notes and ADRs.

- A proposed plan for the next stage, not yet started.

**Machine-output safety**

- NC output is never labelled "verified" by the software. Any change to a post, machine kinematics or controller settings clears that machine's verified flags in the user's records.

- Simulation reduces risk; it does not guarantee a safe program. Say so in the UI where NC is posted.

**Why:** the gates only work if the evidence at each gate is honest and easy to check. A fixed packet format makes every review the same shape, and the done checklist stops half-finished features that work in the UI but break after save and reload.

## Reference material

These are the sources used to build this prompt, grouped by the decision they informed; give them to the agent as starting reading.

| Topic | Sources |
| --- | --- |
| SolidCraft (licence, crates, features, MCP, limits) | [SolidCraft README](https://raw.githubusercontent.com/tangency-labs/solidcraft/d20e582bd95df2ef9b6c9664fb3f55f8d7976aae/README.md) (read in full), [AlternativeTo summary](https://alternativeto.net/software/solidcraft/about/), [SourceForge mirror](https://sourceforge.net/projects/solidcraft.mirror/) |
| SolidCore kernel | [bherbruck/solidcore](https://github.com/bherbruck/solidcore/wiki) |
| FreeCAD (licence, version, architecture, FEM) | [License](https://wiki.freecad.org/License), [About FreeCAD](https://wiki.freecad.org/About_FreeCAD), [Release history](https://wiki.freecad.org/History/cs), [FEM Install](https://wiki.freecad.org/FEM_Install) |
| PrusaSlicer licence | [prusa3d/PrusaSlicer](https://github.com/prusa3d/PrusaSlicer) |
| Rust geometry kernels and OCCT bindings | [truck](https://github.com/ricosjp/truck), [arris](https://docs.rs/crate/arris/0.7.0), [Fornjot](https://docs.rs/crate/fj-math/0.49.0/source/README.md), [cadrum](https://docs.rs/crate/cadrum/0.8.1-alpha.0), [opencascade-sys](https://docs.rs/crate/opencascade-sys/0.2.0/source/Cargo.toml) |
| Solvers, meshing, UI | [faer](https://github.com/sarah-quinones/faer-rs), [faer JOSS paper](https://www.theoj.org/joss-papers/joss.06099/10.21105.joss.06099.pdf), [Netgen](https://sourceforge.net/projects/netgen-mesher/), [egui_dock](https://docs.rs/egui_dock/) |
| NX single part file and master model | [Learning Siemens NX: part files](https://www.lynda.com/NX-tutorials/How-access-create-part-file/716051/760565-4.html), [Eng-Tips: master model drawings](https://eng-tips.com/threads/nx6-w-tc-what-are-the-pros-and-cons-of-a-separate-drawing-prt-file.265609), [Introduction to NX](https://www.scribd.com/doc/95409058/Introduction-to-Nx) |
| NX interface and shortcuts | [NX shortcuts list](https://tutorialtactic.com/blog/siemens-nx-shortcuts/), [defkey NX shortcuts](https://defkey.com/siemens-nx-shortcuts), [Siemens KB: roles and top border bar](https://support.sw.siemens.com/en-US/okba/PL8697772/Menu-button-and-the-top-border-bar-are-missing-from-the-NX-UI/index.html), [Radial menus in NX](https://workbench.grabcad.com/tutorials/the-use-of-radial-menus-in-siemens-nx) |
| NX CAM Operation Navigator | [NX12 tutorial ch. 9](https://me5763.github.io/lab/assets/books/NX12_Tutorial_CH9.pdf), [Manufacturing Essentials](https://pdfcoffee.com/manufacturing-essentials-pdf-free.html), [Siemens KB: Machine Code column](https://support.sw.siemens.com/en-US/okba/PL8635053/Adding-a-Machine-Code-Column-to-the-Operation-Navigator/index.html) |
| NX Post Configurator | [Post Configurator advanced modifications (Siemens)](https://blogs.sw.siemens.com/nx-manufacturing/wp-content/uploads/sites/15/2019/09/03_Post_Configurator_Enablement_AdvancedModifications.pdf), [Siemens KB: buffer events](https://support.sw.siemens.com/en-US/okba/PL8017331/How-you-can-get-the-buffer-events-tab-in-Postconfigurator/index.html) |
| Mastercam peel milling and rest roughing | [MMS: 2D high speed toolpaths](https://mmsonline.com/products/2d-high-speed-tool-paths), [Mastercam Dynamic Milling tutorial](https://www.scribd.com/document/411214623/Intro-Dynamic-Milling), [Mastercam for SOLIDWORKS tutorial](https://meamlabs.seas.upenn.edu/wp-content/uploads/2019/06/MCfSW_Tutorial.pdf) |
| C-axis polar and cylindrical interpolation | [SprutCAM: polar interpolation](https://kb.sprutcam.com/SC15manual/en/10083.html), [Fanuc G12.1 vs G7.1](https://industrialmonitordirect.com/blogs/knowledgebase/fanuc-g121-vs-g71-c-axis-3d-interpolation-compatibility) |
| FEA validation | [NAFEMS R0016 forced vibration](https://www.nafems.org/publications/resource_center/r0016/), [NAFEMS benchmark guide](https://www.nafems.org/publications/pubguide/benchmarks/Page5/), [NAFEMS benchmarks overview (Abaqus manual)](https://ceae-server.colorado.edu/v2016/books/bmk/ch04s01ovv01.html) |
| Topology optimization (methods, manufacturing constraints, prior art) | [beso (CalculiX + FreeCAD, LGPL-3.0)](https://github.com/ScalableDesign/beso), [Review of open-source and commercial TO codes](https://www.proquest.com/docview/2621274006), [Ansys manufacturing constraints](https://ansyshelp.ansys.com/public/Views/Secured/corp/v242/en/mech_struct_opt/ds_topo_manufact_constraint.html), [Abaqus Tosca manufacturing restrictions](https://docs.software.vt.edu/abaqusv2025/English/TsoUserMap/tso-c-user-TopOpt-DesArea-ManufCondGR.htm), [TO for AM with overhang and length-scale constraints (arXiv)](https://arxiv.org/pdf/2204.07333), [Siemens CIMdata e-book on NX generative design](https://www.plm.automation.siemens.com/media/global/pl/CIMdata_eBook_Siemens_PLM_Software_Advances_Generative_Design_Technology_in_NX_tcm83-61556.pdf) |
| 3D scanning and reverse engineering | [Open3D paper (MIT licence)](https://arxiv.org/pdf/1801.09847), [e57 crate docs](https://docs.rs/e57/latest/e57/), [poisson_reconstruction crate](https://docs.rs/crate/poisson_reconstruction/latest/), [PoissonRecon](https://github.com/mkazhdan/PoissonRecon), [NX Reverse Engineering with Convergent Modeling (solution brief)](https://www.emixa.com/hubfs/Emixa/Marketing/Content%20Dashboard/Sub%20Solution%20and%20Sub%20Services%20Content%20downloads/Siemens-PLM-Reverse-Engineering-with-NX-Convergent-Modeling-Solution-Brief-1.pdf), [Siemens blog: Convergent Modeling](https://blogs.sw.siemens.com/designcenter/convergent-modeling-shaping-a-better-future/) |

Not looked up, from general engineering knowledge: the standards cited by name (ASME Y14.5-2018, ISO 1101, ISO 286 and others) and the GPL licensing of CalculiX and Gmsh. Have the agent confirm current editions and licences in Stage 0.
