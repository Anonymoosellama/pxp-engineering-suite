# Kernel bake-off report: SolidCraft kernel (native backend), 2026-11-21

Stage 0, session 2. Backend: `pxp-kernel-native` (SolidCraft's
`solidcraft-kernel` on SolidCore, commit `2091b8828c328d5ab0a0f162dbc7a36a558aa99d`).
The OCCT backend is not built yet (session 3), so this report measures one
candidate; ADR-0004 stays open until both are measured on the same corpus.

## How to reproduce

```sh
cargo run -p pxp-cli -- bakeoff --format markdown --include-slow
```

Timings below come from an optimised build (`opt-level = 1`, see
"Environment"); a debug build gives identical results and hashes, about
20-50 times more slowly.

## Summary

| Corpus | Cases | Pass | Fail | Error | Same hash on 2 runs | Time (s) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Synthetic stress (PXP) | 32 | 32 | 0 | 0 | 32 | 3.9 |
| SolidCraft reference (analytic answers) | 35 | 35 | 0 | 0 | 35 | 197.4 |
| **All** | **67** | **67** | **0** | **0** | **67** | **201.3** |

| Criterion (ADR-0004) | Result for SolidCraft |
| --- | --- |
| Boolean success rate | 67 of 67 cases correct (volume, area, merged face count and shell count where the case fixes them), including coincident faces, tangent contacts, tiny (0.1 µm) and sub-tolerance (10 nm) offsets, voids, multi-lump results, non-manifold edge and vertex contact, and empty results reported as errors |
| History completeness | Every operation passed `KernelHost::op`'s `check_complete`: 0 incomplete histories. Edges new to an operation and all vertices use the adapter's derived records (see "Findings") |
| Speed (time per case, all steps) | median 163 ms, 95th percentile 1.35 s; slowest: the level-3 Menger sponge (560 primitives, 540 subtracts, then one join of 20 bodies) at 178 s |
| In-process determinism | 67 of 67 cases gave the same geometry hash on both runs; debug and optimised builds give identical hashes |
| Cross-OS determinism | Measured by CI from this session on: each of the six CI targets prints one hash per case and the `determinism` job compares them with Linux x86_64 |
| Blend success rate | Not measured: blends arrive in Stage 1 (the kernel trait has no blend operation yet) |
| STEP round-trip fidelity | Not measured: STEP import/export arrives with `pxp-io-step` |

## Findings

1. **No wrong answers.** Every case with an analytic volume matched within
   its tolerance (1e-6 relative for PXP's exact cases, SolidCraft's own
   1e-3 or tighter for theirs), every expected empty result came back as
   `EmptyResult`, and every invalid specification as `InvalidInput`.
2. **Face splitting.** In 11 cases the kernel stores more faces than there
   are surfaces (for example a sphere lens as 3 faces, a tangent-hole plate
   as 22 for 14 surfaces): periodic surfaces are split at seams and tangent
   contacts. Volumes and areas are unaffected. Counting adjacent pieces of
   one surface once (`GeometryKernel::merged_counts`, the count
   SolidCraft's own rig uses) gives the expected numbers. Persistent naming
   must expect several faces from one source face (`Change::Split`).
3. **Vertex history is derived.** SolidCraft records provenance for faces
   and edges, not vertices, and gives edges new to an operation no origin.
   The adapter derives those records from exact B-rep adjacency
   (`docs/design/kernel-contract.md`, "Derived records"); a block corner a
   boolean keeps is matched back to its input vertex.
4. **Slow case.** The level-3 Menger sponge join takes 178 s (SolidCraft's
   rig allows it 1200 s). It is skipped in routine runs and CI
   (`--include-slow` runs it).
5. **Build cost.** SolidCore is about 40 MB of Rust: a debug build of it
   needs roughly 6 GB of memory and an `opt-level = 3` release build did not
   fit in this sandbox's 7 GB. CI runners have more memory; a small
   developer machine should build with `-j1`.

## Corpus

- **Synthetic stress (32):** PXP's own cases in
  `crates/pxp-kernel/src/bakeoff/synthetic.rs`, each with its expected
  answer derived in its description.
- **SolidCraft reference (35):** every SolidCraft kernel regression case
  (`tests/kernel-cases`, answers from analysis, never from a kernel) that
  uses only primitives and booleans, translated by
  `tools/import_solidcraft_cases.py` (MIT; notice in the generated file).
- The master prompt asks for at least 200 parts. The remaining sources need
  operations or access this session lacks: SolidCraft's other ~1000 cases
  use sketches, extrudes, fillets and other Stage 1 features; the NIST MBE
  PMI STEP models need STEP import and access to `nist.gov`.

## Environment

Linux x86_64 sandbox, 2 vCPU (Intel Xeon 2.30 GHz), 7 GB RAM, Rust 1.97
(stable toolchain standing in for the pinned 1.97.0). Optimised build:

```sh
cargo build -p pxp-cli -j1 --profile kb \
  --config 'profile.kb.inherits="release"' --config 'profile.kb.opt-level=1' \
  --config 'profile.kb.debug=false' --config 'profile.kb.codegen-units=256'
```

## Per-case results

| Case | Result | Time (ms) | Same hash ×2 | Detail |
| --- | --- | ---: | --- | --- |
| `syn/prim/block-unit` | pass | 26 | yes | volume 1.000000, area 6.000000, 6 faces (6 as stored), 1 shells |
| `syn/prim/block-large-far-from-origin` | pass | 21 | yes | volume 125000000.000000, area 1750000.000000, 6 faces (6 as stored), 1 shells |
| `syn/prim/block-thin-plate` | pass | 25 | yes | volume 100.000000, area 20004.000000, 6 faces (6 as stored), 1 shells |
| `syn/prim/cylinder-tilted` | pass | 107 | yes | volume 1963.495408, area 942.477796, 3 faces (3 as stored), 1 shells |
| `syn/prim/cone-frustum` | pass | 55 | yes | volume 5497.787144, area 1825.916238, 3 faces (3 as stored), 1 shells |
| `syn/prim/cone-pointed-down` | pass | 42 | yes | volume 452.389342, area 365.990666, 2 faces (2 as stored), 1 shells |
| `syn/prim/sphere-tiny` | pass | 70 | yes | volume 0.000000, area 0.000013, 1 faces (1 as stored), 1 shells |
| `syn/prim/sphere-large` | pass | 125 | yes | volume 4188790204.786391, area 12566370.614359, 1 faces (1 as stored), 1 shells |
| `syn/refuse/zero-height-cylinder` | pass | 0 | yes | refused as expected |
| `syn/refuse/negative-sphere` | pass | 0 | yes | refused as expected |
| `syn/refuse/cone-equal-diameters` | pass | 0 | yes | refused as expected |
| `syn/bool/through-hole` | pass | 144 | yes | volume 5151.769984, area 2708.938010, 7 faces (7 as stored), 1 shells |
| `syn/bool/blind-hole` | pass | 124 | yes | volume 5717.256661, area 2388.495559, 8 faces (8 as stored), 1 shells |
| `syn/bool/coplanar-face-union` | pass | 22 | yes | volume 12000.000000, area 3200.000000, 6 faces (6 as stored), 1 shells |
| `syn/bool/coplanar-partial-face-union` | pass | 31 | yes | volume 12000.000000, area 3500.000000, 10 faces (10 as stored), 1 shells |
| `syn/bool/edge-touching-union` | pass | 31 | yes | volume 2000.000000, area 1200.000000, 12 faces (12 as stored), 2 shells |
| `syn/bool/vertex-touching-union` | pass | 30 | yes | volume 2000.000000, area 1200.000000, 12 faces (12 as stored), 2 shells |
| `syn/bool/disjoint-union` | pass | 29 | yes | volume 2000.000000, area 1200.000000, 12 faces (12 as stored), 2 shells |
| `syn/bool/internal-void` | pass | 109 | yes | volume 7476.401224, area 2714.159265, 7 faces (7 as stored), 2 shells |
| `syn/bool/tangent-cylinder-on-face` | pass | 185 | yes | volume 1785.398163, area 1071.238898, 9 faces (11 as stored), 2 shells |
| `syn/bool/inscribed-cylinder-cut` | pass | 209 | yes | volume 214.601837, area 757.079633, 13 faces (21 as stored), 4 shells |
| `syn/bool/tiny-offset-union` | pass | 25 | yes | volume 1000.010000, area 600.004000, 6 faces (6 as stored), 1 shells |
| `syn/bool/near-coincident-union` | pass | 22 | yes | volume 1000.000000, area 600.000000, 6 faces (6 as stored), 1 shells |
| `syn/bool/thin-cap-union` | pass | 23 | yes | volume 1000.010000, area 600.004000, 6 faces (6 as stored), 1 shells |
| `syn/bool/sphere-lens` | pass | 256 | yes | volume 1308.996939, area 628.318727, 2 faces (3 as stored), 1 shells |
| `syn/bool/cone-cut-by-block` | pass | 137 | yes | volume 1832.595715, area 919.560187, 3 faces (3 as stored), 1 shells |
| `syn/bool/coaxial-cylinder-union` | pass | 259 | yes | volume 1178.097245, area 628.318531, 3 faces (3 as stored), 1 shells |
| `syn/bool/tube` | pass | 433 | yes | volume 1130.973355, area 1357.168026, 4 faces (4 as stored), 1 shells |
| `syn/bool/nine-holes` | pass | 1289 | yes | volume 98232.854132, area 25060.287521, 15 faces (15 as stored), 1 shells |
| `syn/bool/diamond-cut-leaves-four-corners` | pass | 47 | yes | volume 171.572875, area 434.314575, 20 faces (20 as stored), 4 shells |
| `syn/bool/identical-subtract-is-empty` | pass | 12 | yes | refused as expected |
| `syn/bool/disjoint-intersect-is-empty` | pass | 21 | yes | refused as expected |
| `solidcraft/booleans/1107-join-hollow-hub-with-nine-blades` | pass | 1871 | yes | volume 4595.675435, area 6815.036914, 22 faces (22 as stored), 1 shells |
| `solidcraft/booleans/1279-cut-with-tool-cylinder-coaxial-with-hole` | pass | 258 | yes | volume 700.169466, area 576.059779, 11 faces (11 as stored), 1 shells |
| `solidcraft/booleans/1647-box-cylinder-general-intersect` | pass | 174 | yes | volume 7.239254, area 48.963151, 6 faces (6 as stored), 1 shells |
| `solidcraft/booleans/1649-crossing-cylinders-union-far-from-origin` | pass | 370 | yes | volume 2893.457918, area 1543.668365, 6 faces (7 as stored), 1 shells |
| `solidcraft/booleans/1717-two-overlapping-sphere-cuts` | pass | 191 | yes | volume 2043.654806, area 1395.853288, 9 faces (9 as stored), 1 shells |
| `solidcraft/booleans/1865-tube-and-sector-meeting-along-cylinder-walls` | pass | 890 | yes | volume 3110.176727, area 3426.636881, 7 faces (7 as stored), 1 shells |
| `solidcraft/booleans/1910-coaxial-cylinders-intersect` | pass | 349 | yes | volume 16470.993292, area 5404.544674, 3 faces (3 as stored), 1 shells |
| `solidcraft/booleans/1984-near-coaxial-pin-overlaps-bore-wall` | pass | 503 | yes | volume 0.032424, area 181.904935, 4 faces (6 as stored), 1 shells |
| `solidcraft/booleans/1984-rivet-with-flush-heads-a-hair-off-its-hole` | pass | 488 | yes | volume 0.024000, area 18.881556, 4 faces (6 as stored), 1 shells |
| `solidcraft/booleans/2008-coaxial-cylinders-join-one-wall` | pass | 369 | yes | volume 94708.211427, area 11516.827341, 3 faces (3 as stored), 1 shells |
| `solidcraft/booleans/2580-clip-round-small-hole-of-long-plate` | pass | 1248 | yes | volume 491.642480, area 642.261947, 7 faces (7 as stored), 1 shells |
| `solidcraft/booleans/2826-cut-up-to-face-leaves-no-skin` | pass | 285 | yes | volume 4264.204252, area 3673.172954, 9 faces (9 as stored), 1 shells |
| `solidcraft/booleans/2896-join-arm-end-into-ring-wall` | pass | 739 | yes | volume 8442.136556, area 5401.842491, 9 faces (10 as stored), 1 shells |
| `solidcraft/booleans/2896-join-arm-end-near-ring-wall-gap` | pass | 570 | yes | volume 8442.875568, area 5424.291675, 10 faces (10 as stored), 2 shells |
| `solidcraft/booleans/3027-pin-seated-line-to-line-in-bore` | pass | 241 | yes | volume 9000.000000, area 3000.000000, 6 faces (6 as stored), 1 shells |
| `solidcraft/booleans/307-tangent-hole-contacts-ring-against-grid` | pass | 1353 | yes | volume 46858.407346, area 22000.000000, 14 faces (22 as stored), 1 shells |
| `solidcraft/booleans/3327-cut-ending-flush-with-side-wall` | pass | 32 | yes | volume 8600.000000, area 3280.000000, 11 faces (11 as stored), 1 shells |
| `solidcraft/booleans/3460-cut-by-lid-with-coincident-walls-and-half-moon-tabs` | pass | 533 | yes | volume 14640.000000, area 10322.790377, 41 faces (43 as stored), 2 shells |
| `solidcraft/booleans/3497-collinear-cylinders-partial-height-intersection` | pass | 163 | yes | volume 141.371669, area 166.504411, 3 faces (3 as stored), 1 shells |
| `solidcraft/booleans/3497-pin-overlapping-bore-by-20-microns` | pass | 387 | yes | volume 2.269487, area 227.705149, 4 faces (4 as stored), 1 shells |
| `solidcraft/booleans/3526-intersect-keeps-every-lump` | pass | 171 | yes | volume 268.082573, area 301.592895, 4 faces (4 as stored), 2 shells |
| `solidcraft/booleans/4095-slab-cut-standing-off-a-bore-r10` | pass | 217 | yes | volume 40948.814693, area 9780.318531, 12 faces (12 as stored), 1 shells |
| `solidcraft/booleans/4313-join-3x3-grid-of-overlapping-rings` | pass | 5088 | yes | volume 41407.195654, area 23520.888366, 35 faces (44 as stored), 1 shells |
| `solidcraft/booleans/4468-box-minus-bore-measures-exact` | pass | 156 | yes | volume 1358.628331, area 987.699112, 7 faces (7 as stored), 1 shells |
| `solidcraft/booleans/500-coaxial-cylinder-cut-from-thin-disc` | pass | 534 | yes | volume 7539.822369, area 8293.804605, 4 faces (4 as stored), 1 shells |
| `solidcraft/booleans/5126-join-menger-sponge-level-3` | pass | 178150 | yes | volume 1000000.000000, area 451200.000000, 7326 faces (7326 as stored), 1 shells |
| `solidcraft/continuity/g0-box-edge-crease` | pass | 13 | yes | volume 8000.000000, area 2400.000000, 6 faces (6 as stored), 1 shells |
| `solidcraft/solids/cov-cone-frustum-along-x` | pass | 65 | yes | volume 3665.191429, area 1364.182957, 3 faces (3 as stored), 1 shells |
| `solidcraft/solids/cov-cone-full-apex-negative-height` | pass | 39 | yes | volume 452.389342, area 365.990666, 2 faces (2 as stored), 1 shells |
| `solidcraft/solids/cov-cone-zero-height-refused` | pass | 0 | yes | refused as expected |
| `solidcraft/tessellation/1263-concave-arc-knife-edge-plate` | pass | 226 | yes | volume 9591.394099, area 3491.479382, 6 faces (6 as stored), 1 shells |
| `solidcraft/tessellation/1648-sphere-touches-box-edge-union` | pass | 203 | yes | volume 8313.109083, area 2067.805191, 8 faces (8 as stored), 1 shells |
| `solidcraft/tessellation/1813-sphere-lens-thinner-than-tolerance` | pass | 153 | yes | volume 868.627848, area 626.472824, 7 faces (7 as stored), 1 shells |
| `solidcraft/tessellation/2528-stadium-joined-with-overlapping-cylinder` | pass | 672 | yes | volume 14784.541804, area 4361.148581, 8 faces (9 as stored), 1 shells |
| `solidcraft/tessellation/819-cup-pocket-splits-wall-seam` | pass | 690 | yes | volume 20633.980549, area 21262.299079, 5 faces (5 as stored), 1 shells |
