<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# Kernel bake-off: SolidCraft versus OCCT (sub-stage 0F)

- **Date:** 2026-11-21 (runs finished 2026-11-22 UTC)
- **Corpus:** 200 parts: 161 primitive and boolean cases (32 synthetic
  stress, 34 SolidCraft reference, 95 generated) and 39 NIST MBE PMI STEP
  files from two pinned NIST releases.
- **Backends:** SolidCraft's `solidcraft-kernel` on SolidCore, git
  `2091b88`, in-process (`pxp-kernel-native`). OCCT 7.8.1 through the OCP
  Python bindings (`cadquery-ocp` 7.8.1.1), in a worker process speaking
  protocol 1 (`pxp-kernel-occt`).
- **Machine:** Linux x86-64, 2 vCPU, 7 GB RAM, no GPU. Nothing else ran
  during the timed bake-off runs.
- **Rule:** no expected value comes from a kernel. Grades use closed
  forms, the files' own CAx-IF validation properties, declared bodies, and
  identities such as V(A − T) + V(A ∩ T) = V(A).

## Recommendation

**OCCT as the primary geometry kernel, in its worker process. SolidCraft
stays in-process as the exact-answer test oracle.**

Real parts decide this. Users import STEP constantly. OCCT read 38 of 39
NIST files into valid solids and finished every boolean on every imported
solid. SolidCraft's reader matches the exporting systems' numbers, but 13
of 37 real solids fail its own validity check, and 16 booleans on 6 of them
were refused. OCCT is also faster: 3.7× at the median and 3.3× at the 95th
percentile on the primitive corpus at full optimisation, and about 15× on
STEP import. OCCT already has the blends, shells and offsets that Stage 1
needs. Its LGPL-2.1 licence adds no conveyance constraint, whereas
SolidCore's Apache-2.0-only crates force LGPL-3.0. That now matters only
for `pxp-cli` and for embedders, because the desktop binary is LGPL-3.0
anyway: winit is Apache-2.0 only (ADR-0001).

SolidCraft is the more exact kernel wherever it accepts the input. On
valid imported solids its boolean identities hold to 1.6e-8. It is pure
safe Rust, in-process, with exact face provenance. Those are the strengths
an oracle needs, so it keeps that role: every OCCT result on analytic
geometry is cross-checked against it in CI.

**Conditions** (they are in ADR-0004 and STATUS.md):

1. The Python worker is a measurement tool, not something to ship. A
   native OCCT worker, in C++ or Rust bindings, must replace it before the
   first release. It speaks the same protocol and must pass the same
   conformance tests.
2. Mass properties must use OCCT's adaptive integration. The worker now
   does (`MASS_EPS = 1e-9`); see finding 3.
3. On imported freeform solids, OCCT's identities and STEP round trips
   drift by up to 4.6e-5 relative, which is beyond the 1e-6 the master
   prompt sets for STEP round trips. That gate value needs a decision
   (finding 4).
4. OCCT's cross-OS determinism must be confirmed by the new `occt` CI job
   before the decision is final.

## Results

### Primitive and boolean corpus (161 cases, opt-level 3, `--repeat 2`)

| Corpus | Cases | SolidCraft pass | OCCT pass | SolidCraft median / p95 (ms) | OCCT median / p95 (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Synthetic stress | 32 | 32 | 32 | 32 / 229 | 24 / 53 |
| SolidCraft reference | 34 | 34 | 33 | 318 / 1237 | 50 / 364 |
| Generated families | 95 | 93 | 95 | 167 / 1210 | 44 / 451 |
| **All** | **161** | **159** | **160** | **160 / 1237** | **43 / 378** |

Both kernels gave the same hash on repeated runs for all 161 cases. Both
had no incomplete histories: `KernelHost::op` rejects incomplete ones as
errors, and OCCT reported none.

The grades differ on three cases:

| Case | SolidCraft | OCCT | Why |
| --- | --- | --- | --- |
| `gen/far/1e6` (holed block 10⁶ mm from the origin) | error | pass | SMLib's check refuses the translated operand, so the boolean is refused. |
| `gen/lens/intersect-19.9` (two spheres overlapping by 0.1 mm) | fail (area 6.283174 against 6.283179 to 6.283192) | pass | SolidCraft's area of a thin lens is off by 1e-6 relative. |
| `solidcraft/booleans/307-tangent-hole-contacts-ring-against-grid` | pass | fail (20 faces, 14 expected) | OCCT splits a cylindrical face where a tangent neighbour touches it, unless the touch falls on the face's seam: 6 of 8 holes split, and symmetric holes are treated differently. Reproduced directly in OCP; `ShapeUpgrade_UnifySameDomain` does not merge across the non-manifold contact line. |

On the 152 cases both kernels pass, they agree on volume to a median of 0
and a worst of 1.1e-9 (`gen/steinmetz/tricylinder`). They agree on area to
a median of 0 and a worst of 6.5e-7 (`gen/lens/intersect-19`). The merged
face counts that the cases do not grade differ on 4 cases, all by the
tangent-contact split above.

The same runs at opt-level 1 (`results2`, the build used for the CI
bake-off) put SolidCraft at 8× slower. That figure overstated the gap,
which is why these timings were re-measured.

### NIST MBE PMI STEP corpus (39 files, `--repeat 2`)

| Check | SolidCraft pass / fail / error / n/a | OCCT pass / fail / error / n/a |
| --- | --- | --- |
| `import` (declared solids closed and valid by the kernel's own check) | 24 / 13 / 2 / 0 | 38 / 1 / 0 / 0 |
| `reference` (file's stated volume, area, centroid, 0.1 %) | 16 / 0 / 0 / 21 | 16 / 0 / 0 / 23 |
| `native-roundtrip` | 37 / 0 / 0 / 0 | 39 / 0 / 0 / 0 |
| `step-roundtrip` (1e-6) | 36 / 1 / 0 / 0 | 33 / 4 / 0 / 2 |
| `box-identity` | 32 / 0 / 5 / 0 | 37 / 0 / 0 / 2 |
| `half-split` (identities, 1e-6) | 31 / 0 / 6 / 0 | 32 / 5 / 0 / 2 |
| `drill` (identities, 1e-6) | 32 / 0 / 5 / 0 | 35 / 2 / 0 / 2 |

| | SolidCraft | OCCT |
| --- | --- | --- |
| Files passing every check | 24 | 32 |
| Files with an error (refused operation) | 8 | 0 |
| STEP import, median / p95 | 2.51 s / 21.4 s (opt-level 1) | 0.16 s / 0.51 s |
| Whole file, all checks, median / p95 | 36.6 s / 128 s (opt-level 1) | 5.8 s / 28.7 s |
| Same hash on both runs | 39 / 39 | 39 / 39 |

The NIST timings for SolidCraft are from the opt-level-1 build. The
opt-level-3 rerun was cut short when the machine restarted, and only the
OCCT half was repeated. The primitive corpus suggests opt-level 1
overstates SolidCraft's time by about 2×.

On the 37 files where both readers produce solids, they agree on volume to
a median of 1.6e-7 and a worst of 3.1e-5 (`2026-01/nist_ctc_02_asme1_ap203`).

## Findings

1. **Healing is the difference on real parts.** Most of SolidCraft's 13
   import failures are gaps above its fixed tolerance. OCCT closes them by
   growing per-entity tolerances. Passing OCCT's healed output to
   SolidCraft does not help: three failing parts (`ctc_04_rd`, `ftc_07_rd`,
   `stc_07_ap242-e3`) were read by OCCT, run through `ShapeFix_Shape`, and
   written back as STEP. All three still fail SolidCraft's check, and only
   one then accepts booleans. A design with SolidCraft for modelling and
   OCCT only for import therefore does not solve the problem.
2. **OCCT never refused an operation** on this corpus: no errors on 161
   cases and 39 files. Its failures are tolerance failures, not refusals.
3. **OCCT's default mass integration is too coarse for 1e-6 checks.** The
   fixed-order `BRepGProp` overloads put case 1107 (a tube joined with nine
   bars) 2.8e-6 off the closed form when reproduced in OCP. In the bake-off
   the worker's figure was 5.8e-5 off. The adaptive overloads at
   `Eps = 1e-9` are off by 1.8e-10. The closed form is
   1335π + 9(15π − V_in), with V_in integrated numerically
   (4595.675434860895 mm³; SolidCraft gives 4595.675435). The worker now
   uses the adaptive overloads. That cut the worst cross-kernel volume
   difference on the primitive corpus from 5.8e-5 to 1.1e-9.
4. **The freeform drift is not the integrator.** On imported NIST parts,
   adaptive integration moved OCCT's half-split residual on `ctc_02_ap203`
   only from 4.1e-5 to 1.7e-5 in a direct OCP experiment. The failures
   remain, at most 4.6e-5 relative: 7 identity checks on 5 files and 4
   STEP round trips (ΔV up to 3.3e-5). They come from OCCT's approximated
   intersection curves and its STEP writer's B-spline output. The master
   prompt requires STEP round trips within 1e-6. OCCT meets that on
   analytic parts but not on every freeform one, and so does SolidCraft
   (1 of 37). The value needs deciding at the gate: keep 1e-6 for analytic
   geometry, and set a measured bound such as 1e-4 for imported freeform
   solids.
5. **Topology differences matter for naming, not for correctness.** OCCT's
   seam-dependent splitting at tangent contacts (case 307) means persistent
   naming must not assume one face per cylinder. Merged counts and
   split-aware names already allow for this (ADR-0006).
6. **Precision on clean input is a tie.** When both kernels accept an
   analytic input, they agree to 1e-9.

## Not measured yet

- Blends, shells and offsets: they need Stage 1 operations, as ADR-0004
  says.
- OCCT on Windows and macOS. The `occt` CI job (`.github/workflows/ci.yml`)
  runs the conformance tests and the full bake-off on three operating
  systems and compares hashes. It has not run, because the commits are not
  pushed.
- A native OCCT worker (condition 1).

## Reproduce

```sh
cargo build --release -p pxp-cli
export PXP_OCCT_PYTHON=/path/to/python-with-cadquery-ocp
pxp-cli bakeoff --kernel native --corpus all --repeat 2 --format markdown
pxp-cli bakeoff --kernel occt   --corpus all --repeat 2 --format markdown
python3 tools/fetch_nist_step.py DIR        # pinned NIST files, SHA-256 checked
pxp-cli stepstress --kernel occt --repeat 2 --format tsv --header DIR/*.stp
```

Raw outputs of these runs: the bake-off Markdown and details files, the
NIST TSVs and details, and the input hashes (`pxp-cli` binary
`f2c357e1…`, worker `e76596bb…`). They are in the session's results
folders (`results2`: opt-level 1; `results3`: opt-level 3 with adaptive
integration). The tables above were generated from them by
`tools/compare_bakeoff.py` (`primitive` and `nist` modes).
