# ADR-0004: Geometry kernel

- **Status:** Decided by the bake-off: OCCT primary, SolidCraft oracle.
  Becomes Accepted when the Stage 0 gate is approved; see the conditions.
- **Date:** 2026-11-20; measurements 2026-11-21; decision 2026-11-22

## Context

All geometry goes through the `pxp-kernel` trait (`GeometryKernel`,
`OpResult`, `TopoHistory`, implemented in session 1). The master prompt
requires a bake-off between SolidCraft's kernel / SolidCore (pure Rust,
in-process) and OCCT (native, in a worker process), measured on a shared
corpus of at least 200 parts.

## Candidates

| Backend | Hosting | Licence and ADR-0001 consequence |
| --- | --- | --- |
| SolidCraft's `solidcraft-kernel` on SolidCore (safe-Rust port of NVIDIA SMLib) | In-process (`pxp-kernel-native`, implemented session 2) | SolidCraft crates MIT OR Apache-2.0 (taken as MIT). SolidCore crates Apache-2.0 only: `deny.toml` exceptions recorded and binaries that link them conveyed under LGPL-3.0 (ADR-0001). Not on crates.io: a git dependency pinned to commit `2091b88`. |
| OCCT 7.8.1 | Worker process (`pxp-kernel-occt`, implemented session 3, protocol 1). Measured through the OCP Python bindings; a native worker (C++, or cadrum / opencascade-rs bindings) replaces it before release. | LGPL-2.1 with exception: compatible. Static linking is satisfied by shipping full source. |
| arris, truck, Fornjot | Fallback references | MIT OR Apache-2.0 / Apache-2.0. |

## Measurements (per backend)

1. Boolean success rate (unite, subtract, intersect), including synthetic
   stress cases: coincident faces, tangent booleans, tiny edges.
2. Blend success rate, including near-degenerate blends.
3. STEP round-trip fidelity: volume, area, centroid within 1e-6 relative.
4. Speed: median and 95th-percentile time per operation.
5. Cross-OS determinism: identical topology order, history and
   `geometry_hash` on Windows, macOS and Linux.
6. **History completeness**: `KernelHost::op` already rejects incomplete
   histories; the bake-off reports how often each backend trips it, because
   persistent naming depends on it.

Face counts are compared as **merged counts** (adjacent pieces of one
surface counted once, `GeometryKernel::merged_counts`): kernels differ in
where they split periodic faces, so raw counts are not comparable.

## Corpus

Target: SolidCraft's reference parts, public-domain STEP sets such as the
NIST MBE PMI test models, and generated stress cases, at least 200 in
total. Implemented: 32 synthetic stress cases, 34 SolidCraft analytic
kernel cases expressible with primitives and booleans (the 35th, a Menger
sponge, is in the slow set), 95 generated cases in 15 families
(`pxp_kernel::bakeoff::families`: disjoint arrays, hole grids, coplanar and
shifted stacks, Steinmetz solids, sphere caps, cone bores, tangent holes,
voids, transformed copies far from the origin, lenses, stepped shafts,
octant quarters, cone apexes), and the NIST MBE PMI STEP models (39 files
from two pinned NIST releases, `tools/fetch_nist_step.py`, run by `pxp-cli
stepstress`): 200 parts. Blends need Stage 1 operations.

## Measurements

Full reports: `docs/reports/kernel-bakeoff-2026-11-21-comparison.md` (both
kernels, all 200 parts), `docs/reports/kernel-bakeoff-2026-11-21-native.md`
and `docs/reports/kernel-nist-step-stress-2026-11-21.md` (SolidCraft's
earlier runs).

| Criterion | SolidCraft (native) | OCCT (worker) |
| --- | --- | --- |
| Primitive and boolean cases correct (161) | 159 (one refusal at 10⁶ mm, one thin-lens area off by 1e-6) | 160 (splits tangent-touching cylinder faces depending on seam position) |
| Agreement where both pass (152 cases) | volume to 1.1e-9, area to 6.5e-7 | (same) |
| Incomplete histories | 0 | 0 |
| Time per case, median / p95 (opt-level 3, 2 vCPU) | 160 ms / 1.24 s | 43 ms / 378 ms |
| Same hash on repeated runs | 161 / 161 | 161 / 161 |
| Cross-OS hashes | 65 / 66 identical on six targets (case 3460 differs on macOS) | not run yet (`occt` CI job added) |
| NIST: solids valid after import | 24 / 37 (no healing) | 38 / 39 files (shape healing) |
| NIST: agreement with the files' stated properties | 16 / 16 within 0.1 % | 16 / 16 within 0.1 % |
| NIST: boolean checks refused | 16 (on 6 invalid solids) | 0 |
| NIST: identity checks beyond 1e-6 | 0 on valid solids (worst 1.6e-8) | 7 on 5 freeform files (worst 4.6e-5) |
| NIST: STEP round trip within 1e-6 | 36 / 37 | 33 / 37 (ΔV up to 3.3e-5) |
| NIST: tessellated AP242 read | 0 / 2 | 2 / 2 |
| NIST: STEP import, median | 2.51 s (opt-level 1) | 0.16 s |
| Blends | not measurable before Stage 1 | not measurable before Stage 1 |

## Decision

**OCCT is the primary geometry kernel, hosted in its worker process.
SolidCraft stays in-process as the exact-answer oracle**: CI cross-checks
OCCT against it on analytic geometry, and it remains a second backend
behind the same trait.

Reasons, from the measurements:

1. Real geometry. OCCT healed and read 38 of 39 NIST files into valid
   solids and finished every boolean on them. SolidCraft has no healing:
   13 of 37 real solids fail its own check, and 16 booleans were refused.
   Passing OCCT's healed output to SolidCraft does not fix this (tested on
   three parts).
2. Speed. 3.7× at the median and 3.3× at p95 on the primitive corpus at
   full optimisation, worker overhead included. About 15× on STEP import.
3. Coverage for Stage 1. OCCT has blends, shells, offsets, drafts and
   sweeps; SolidCraft's are unmeasured.
4. Licence. LGPL-2.1 with exception adds no conveyance constraint, while
   SolidCore's Apache-2.0-only crates force LGPL-3.0 conveyance. This now
   matters only for `pxp-cli` and embedders of the libraries: the desktop
   binary is conveyed under LGPL-3.0 anyway, because winit (under every
   Rust GUI toolkit) is Apache-2.0 only (ADR-0001, ADR-0002).

What SolidCraft does better, and why it is kept: it is exact on clean
input (identities to 1.6e-8), pure safe Rust in-process, and has exact
face provenance. That is what a test oracle needs.

## Conditions (open until met)

1. Replace the Python worker (OCP) with a native worker before the first
   release. Same protocol, same conformance tests
   (`crates/pxp-kernel-occt/tests/conformance.rs`).
2. Mass properties use OCCT's adaptive integration (`MASS_EPS = 1e-9` in
   the worker; done). The fixed-order default was off by up to 5.8e-5.
3. Decide the STEP round-trip gate for imported freeform solids. The
   master prompt's 1e-6 holds for analytic parts on both kernels, but
   OCCT drifts up to 3.3e-5 on 4 of 37 freeform parts (SolidCraft: 1 of
   37). Proposal: 1e-6 for analytic geometry, a measured 1e-4 for imported
   freeform B-splines.
4. Confirm OCCT's cross-OS determinism with the `occt` CI job on three
   operating systems.
5. Persistent naming must not assume one face per cylinder: OCCT's
   tangent-contact splits depend on seam placement (case 307).

## Consequences

- Stage 1 starts by switching `pxp-app` and `pxp-cli` part commands to the
  OCCT backend. Today both default to SolidCraft, and `--kernel occt`
  selects OCCT.
- Parts store the backend that computed each feature (`kernel` property).
  A part made with SolidCraft displays only after a recompute with OCCT,
  and the shell says so.
- The bake-off corpus keeps running nightly on both kernels.
