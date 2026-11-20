# ADR-0004: Geometry kernel (bake-off plan)

- **Status:** Proposed. The winner is chosen from measured bake-off results;
  nothing has been measured yet.
- **Date:** 2026-11-20

## Context

All geometry goes through the `pxp-kernel` trait (`GeometryKernel`,
`OpResult`, `TopoHistory`, implemented in session 1). The master prompt
requires a bake-off between SolidCraft's kernel / SolidCore (pure Rust,
in-process) and OCCT (native, in a worker process), measured on a shared
corpus of at least 200 parts.

## Candidates

| Backend | Hosting | Licence and ADR-0001 consequence |
| --- | --- | --- |
| SolidCore (safe-Rust port of NVIDIA SMLib) | In-process (`pxp-kernel-native`) | Apache-2.0 only. Linking it means distributed binaries are conveyed under LGPL-3.0 (allowed by "or later"); record a `deny.toml` exception. Alternatively ask for an MIT dual licence. |
| SolidCraft's kernel layer | In-process | MIT OR Apache-2.0 (taken as MIT). |
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

## Corpus

SolidCraft's reference parts, public-domain STEP sets such as the NIST MBE
PMI test models, and generated stress cases, at least 200 in total. The
corpus and harness live in `tests/kernel-corpus/` and run nightly.

## Decision

Pending measurements (sub-stage 0F, blocked on crates.io and git access).
The losing backend stays as a test oracle and for data exchange.
