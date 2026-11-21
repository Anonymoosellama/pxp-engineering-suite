# ADR-0004: Geometry kernel (bake-off plan)

- **Status:** Proposed. The winner is chosen from measured bake-off results.
  Session 2 measured the SolidCraft backend; OCCT is measured in session 3,
  then this ADR is decided.
- **Date:** 2026-11-20; measurements added 2026-11-21

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
| OCCT via cadrum or opencascade-rs | Worker process (`pxp-kernel-occt`) | LGPL-2.1 with exception: compatible. Static linking is satisfied by shipping full source. |
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
total. Implemented in session 2 (`pxp_kernel::bakeoff`, run by
`pxp-cli bakeoff`): 32 synthetic stress cases and the 35 SolidCraft
analytic kernel cases expressible with primitives and booleans (67). The
rest needs Stage 1 operations (sketch, extrude, blend) and STEP import, and
the NIST models need network access to `nist.gov`.

## Measurements so far

Full report: `docs/reports/kernel-bakeoff-2026-11-21-native.md`.

| Criterion | SolidCraft (native) | OCCT (worker) |
| --- | --- | --- |
| Boolean cases correct | 67 / 67 | session 3 |
| Incomplete histories | 0 (edges new to an operation and all vertices derived from adjacency) | session 3 |
| Time per case, median / p95 (opt-level 1, 2 vCPU) | 163 ms / 1.35 s | session 3 |
| Same hash on repeated runs | 67 / 67 | session 3 |
| Cross-OS hashes | compared by CI on six targets from session 2 | session 3 |
| Blends, STEP round trip | not measurable before Stage 1 | not measurable before Stage 1 |

Early observations, not yet a decision: SolidCraft gave no wrong answer on
this corpus, is pure safe Rust (in-process, no worker needed, panics
contained by `KernelHost`), and gives exact face and edge provenance. Its
costs are the LGPL-3.0 conveyance, a large compile, an unpublished
experimental API pinned to one commit, and face splitting at periodic seams
(handled by merged counts and by split-aware naming).

## Decision

Pending the OCCT measurements on the same corpus (session 3). The losing
backend stays as a test oracle and for data exchange.
