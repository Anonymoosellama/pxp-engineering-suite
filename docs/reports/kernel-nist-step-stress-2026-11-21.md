# Kernel stress test on the NIST MBE PMI STEP models: SolidCraft kernel (native backend), 2026-11-21

Stage 0, session 3, sub-stage 0F. Backend: `pxp-kernel-native` (SolidCraft's
`solidcraft-kernel` on SolidCore, commit `2091b8828c328d5ab0a0f162dbc7a36a558aa99d`).
The OCCT backend is not built yet, so this measures one candidate; ADR-0004
stays open until both have run the same corpus.

## The test models

The 39 STEP files of NIST's MBE PMI Validation and Conformance Testing
project that are public: the Combined Test Cases (CTC 1-5), Fully-Toleranced
Test Cases (FTC 6-11) and Simplified Test Cases (STC 6-10), each as one of
several commercial CAD systems exported it (NIST removed which system wrote
which file). NIST states they "can be used without any restrictions" and
asks for acknowledgement; this report acknowledges NIST as their source.
NIST also warns they are not error-free reference files.

| Release | Files | What |
| --- | ---: | --- |
| Current package (updated 2026-02-18) | 28 | 11 AP203 geometry-only files and 17 AP242 files (editions 1-4), one of them tessellated (FTC-08 `-e4-tg`) |
| January 2026 package | 11 | what the February 2026 update removed or replaced: 5 AP203 files with graphic PMI, and the earlier CTC-01, CTC-04, CTC-05, FTC-08 tessellated (`-e1-tg`), FTC-11 and STC-09 files |

**Provenance.** The sandbox still cannot reach `nist.gov` (the egress proxy
answers 403 for it), so the files come from the same package in NIST's own
repository, <https://github.com/usnistgov/SFA> (`Release/NIST-PMI-STEP-Files.zip`),
pinned to commit `1236017` for the current release and `c218911` for the
January 2026 one, with SHA-256 checksums in `tools/fetch_nist_step.py`. They
are byte-identical to the copy the user downloaded from `nist.gov`: all 21
files the two hold in common under the same name, and 32 of the 32
comparable files of the January package.

## What each file goes through

`pxp-cli stepstress` (`pxp_kernel::bakeoff::run_step_file`) runs every file
through seven checks. No answer comes from the kernel under test: the
references are the file's own entities and the validation properties the
exporting CAD system wrote into it, both read by a small independent Part 21
reader (`bakeoff::p21`), and identities that hold for any correct kernel.

| Check | Passes when |
| --- | --- |
| `import` | every solid the file declares comes in closed and passes the kernel's own validity check (SMLib's full check, `GeometryKernel::check_validity`); open bodies are allowed only up to the surface shells the file declares as sheets (CAD systems export construction surfaces and datum target areas as `SHELL_BASED_SURFACE_MODEL`s); a tessellated file must give at least one mesh body |
| `reference` | the closed solids' volume and area within 0.1 % (relative) of the file's **geometric validation properties**, and the centroid within 0.1 % of the box diagonal; not applicable when the file states none |
| `native-roundtrip` | export and import of SolidCraft's native bytes gives the same geometry hash |
| `step-roundtrip` | writing the bodies as STEP and reading them back keeps the body count, the closed count, the merged face count, and volume, area and centroid within 1e-6 (the Stage 1 gate tolerance) |
| `box-identity` | `A ∩ E`, where `E` is a block 5 % of the diagonal larger than A's exact box, equals A in volume and area (1e-6) |
| `half-split` | with `H` a block above a plane 0.37 % of the height past A's centroid: `V(A−H)+V(A∩H)=V(A)`, `V(A∪H)+V(A∩H)=V(A)+V(H)`, `S(A∪H)+S(A∩H)=S(A)+S(H)` (areas), all within 1e-6, every result a solid, and the cut section not negative |
| `drill` | the same identities for a cylinder through A along Z, a quarter of the smaller X/Y extent across, placed near the centroid or, where that misses material (a hollow part), at the first of four fixed points that meets it |

Every boolean and primitive goes through `KernelHost::op`, so an incomplete
history is an error. A check **fails** when the kernel answers wrongly and
is an **error** when it refuses or fails where it should succeed. The whole
procedure runs twice per file: identical hashes of every body made (imports,
round trips, boolean results) make it **deterministic**.

## How to reproduce

```sh
python3 tools/fetch_nist_step.py nist          # 39 files, checksums verified
cargo build -p pxp-cli -j1 --profile kb \
  --config 'profile.kb.inherits="release"' --config 'profile.kb.opt-level=1' \
  --config 'profile.kb.debug=false' --config 'profile.kb.codegen-units=256'
target/kb/pxp-cli stepstress --repeat 2 --format markdown nist/current nist/2026-01
```

This report's run used one process per file (so a hang or abort could not
take the others down) with a 40-minute limit each; no file hit it.

## Summary

Run of 2026-11-21 (22:25–23:26 UTC), binary SHA-256 `d7da7cfc…17c2`. 37 of the
39 files hold an exact B-rep with one solid each; the other two are
tessellated (FTC-08 `-e1-tg`, `-e4-tg`).

| Check | Pass | Fail | Error | n/a |
| --- | ---: | ---: | ---: | ---: |
| `import` | 24 | 13 | 2 | 0 |
| `reference` | 16 | 0 | 0 | 21 |
| `native-roundtrip` | 37 | 0 | 0 | 0 |
| `step-roundtrip` | 36 | 1 | 0 | 0 |
| `box-identity` | 32 | 0 | 5 | 0 |
| `half-split` | 31 | 0 | 6 | 0 |
| `drill` | 32 | 0 | 5 | 0 |

**24 of 39 files pass every check, 7 fail one or more, 8 error.** All 39
gave the same hashes on both runs; no process crashed, hung or panicked
(39 / 39 exited normally, the slowest after 269 s of the 2400 s limit).
The two tessellated files are refused at import, so later checks did not
run on them ("—" in the table below).

| Criterion (ADR-0004) | Result for SolidCraft |
| --- | --- |
| STEP import of exact geometry | 37 / 37 solids closed, no face left out, every declared surface model read as a sheet body; **24 / 37 solids pass SMLib's own check, 13 fail it** (12 for gaps above tolerance; finding 3) |
| Accuracy against the exporters' own numbers | 16 / 16 files that state them within 0.1 %; median error volume 1.0e-7, area 3.1e-8, centroid 1.3e-8 × diagonal; worst 3.7e-6 |
| Boolean robustness on real parts | 95 boolean checks pass with exact inclusion-exclusion (worst residual 1.6e-8 on a valid solid, 1.5e-7 on an invalid one); 16 refused, all on 6 of the 13 invalid solids |
| History completeness | 0 incomplete histories (every primitive and boolean went through `KernelHost::op`) |
| STEP round-trip fidelity (1e-6) | 36 / 37 (median residual 5e-15); 1 failure on an invalid solid (finding 4) |
| Tessellated AP242 | 0 / 2 (not supported yet; refused with a clear message) |
| In-process determinism | 39 / 39 |
| Cross-OS determinism | not measured yet: the nightly workflow added for it has not run (finding 6) |
| Speed (opt-level 1, 2 vCPU) | first import median 2.5 s, max 26.7 s; whole procedure ×2 median 72 s per file, 56.5 min for the corpus |
| Memory | peak median 144 MiB, max 483 MiB |

## Findings

1. **No wrong answers on valid geometry.** On the 24 files whose solid
   passes SMLib's check, every applicable check passed: the import, the
   reference values where stated, both round trips, and all 72 boolean
   identity checks (box, half-split, drill), the worst residual 1.6e-8
   (an area, FTC-11 AP203 drill) and most at 1e-15. Where the kernel gave
   an answer it was right; every problem below is a refusal, not a wrong
   number.

2. **The translator reproduces the exporting CAD systems' own numbers.**
   The 16 files that state geometric validation properties for their solid
   agree with SolidCraft's reading to a median 1.0e-7 in volume, 3.1e-8 in
   area and 1.3e-8 of the diagonal in centroid; the worst are STC-10
   (volume 3.3e-6, area 3.7e-6) and CTC-02 AP203 (centroid 3.1e-7), all far
   inside the 0.1 % tolerance. All 37 declared solids came in closed, no
   face was left out, and every surface model the files declare
   (construction surfaces and datum target areas, in 13 files) came in as a
   sheet body.

3. **The main weakness: 13 imported solids fail SMLib's own check.** They
   are closed and their volumes are right where the file says (CTC-05
   AP203 `rd`, invalid, is within 4.6e-7), but SMLib finds gaps larger than the
   model tolerance between vertices, edges and faces in 12 of them, a face
   vertex that does not classify back to its face in CTC-04 AP203, and, in
   FTC-07 and STC-07, B-spline trim curves turning sharply near surface
   poles and curve cusps. SolidCraft says so at import ("needs repair
   before it can be modelled") and does not heal them. What that costs:

   | Outcome | Files |
   | --- | --- |
   | Still model exactly (every boolean passes) | CTC-02 AP203 `rc`, CTC-05 AP242 `-e1` (both releases), CTC-05 AP203 with PMI, FTC-11 AP242 `-e2` and `-e3`, STC-09 `-e4` |
   | Partly | CTC-05 AP203 `rd`: half-split refused ("SMLib's result is not a closed solid"); box and drill exact |
   | Refused outright: every boolean fails with "SMLib's check refuses the result (the first operand already fails it)" | CTC-04 AP203 `rd`, FTC-07 AP242 and AP203, STC-07 (derived from FTC-07), CTC-02 AP203 with PMI |

   The same part written by another exporter is often valid (CTC-04 from
   AP242 and from the AP203-with-PMI file passes everything), so most
   defects come from how a file was written. FTC-07 fails from both
   exporters, and STC-07 inherits it, which points at its B-spline
   geometry itself. A user opening the five refused files could not cut or
   join them. This is the criterion OCCT has to be measured against (its
   shape healing commonly enlarges tolerances to accept such gaps). Next
   steps: apply SolidCraft's `repair` after import and re-measure, and
   report CTC-04 `rd` and FTC-07 upstream with these files.

4. **One STEP round-trip defect.** CTC-02 AP203 with PMI (January
   release, already invalid on import) is written by SolidCraft and read
   back as an **open** body with one face fewer (487 → 486 merged faces),
   so its volume is lost. The other 36 round trips keep body and face
   counts, and volume, area and centroid within 1e-6 (median residual
   5e-15). Area is the least exact quantity through STEP: CTC-04 AP242
   comes back with area 8.3e-7 off (volume 4.3e-12), close to the limit.

5. **Tessellated AP242 is not supported yet.** Both FTC-08 tessellated
   files are refused cleanly: `COMPLEX_TRIANGULATED_FACE` (AP242 edition 4)
   and `TESSELLATED_SHELL` (edition 1) "not supported yet". No crash, but
   the mesh path of the import could not be exercised.

6. **Deterministic in-process; cross-OS still open.** Every file gave the
   same hashes (every import, round trip and boolean result) on both runs.
   Comparing hashes across operating systems needs the new nightly
   workflow (`.github/workflows/nightly.yml`), which runs this corpus on
   all six CI targets; it has not run because this session could not push.

7. **What the files themselves taught the harness** (worth knowing for
   anyone grading against them):
   - The same test case from different exporters is not the same solid:
     volumes differ by up to 1.1e-3 (FTC-11) and areas by up to 1.1e-3
     (CTC-01), yet each file's import matches that file's own stated
     values (CTC-02: AP242 states 4.710679e7 mm³, AP203 4.710153e7; read
     4.710681e7 and 4.710162e7). Cross-file agreement is therefore shown
     below for information and never used as a grade.
   - STC-09 in the January release labels its validation volume and area
     in millimetres while its model is in inches. Taken in inches they are
     within 1.5e-4 (volume) and 1.2e-4 (area) of the import. NIST replaced
     the file in February 2026; the reader flags the mismatch and does not
     grade against it.
   - STC-06 states zero volume and area on an empty shape aspect, and the
     January STC-09 swaps the name and description of its property
     definitions. Both are handled explicitly and noted.
   - Thirteen files carry deliberate surface bodies next to the solid;
     counting them as failed solids (as a first version of the harness did)
     would misgrade a third of the corpus.

## Against the files' own validation properties

Graded files only (STC-09 January release: ungraded, finding 7).

| File | Stated volume (mm³) | Read volume (mm³) | ΔV | Stated area (mm²) | Read area (mm²) | ΔA | ΔC (× diagonal) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `current/nist_ctc_01_asme1_ap242-e1` | 1.464482e+07 | 1.464482e+07 | 5.8e-10 | 8.070808e+05 | 8.070808e+05 | 2.4e-08 | 6.5e-12 |
| `current/nist_ctc_02_asme1_ap242-e2` | 4.710679e+07 | 4.710681e+07 | 4.0e-07 | 2.480624e+06 | 2.480624e+06 | 1.7e-07 | 4.4e-8 |
| `current/nist_ctc_02_asme1_rc` | 4.710153e+07 | 4.710162e+07 | 1.9e-06 | 2.481848e+06 | 2.481842e+06 | 2.3e-06 | 3.1e-7 |
| `current/nist_ctc_03_asme1_ap242-e2` | 3.321229e+05 | 3.321229e+05 | 9.0e-13 | 3.547877e+05 | 3.547877e+05 | 0.0e+00 | 2.0e-12 |
| `current/nist_ctc_04_asme1_ap242-e2` | 1.751978e+07 | 1.751978e+07 | 4.2e-10 | 1.022497e+06 | 1.022497e+06 | 3.9e-08 | 8.5e-11 |
| `current/nist_ctc_05_asme1_rd` | 1.269491e+07 | 1.269491e+07 | 4.6e-07 | 8.612429e+05 | 8.612429e+05 | 1.7e-08 | 2.9e-8 |
| `current/nist_ftc_06_asme1_ap242-e2` | 3.323557e+06 | 3.323557e+06 | 1.3e-09 | 2.834130e+05 | 2.834131e+05 | 1.6e-07 | 6.0e-10 |
| `current/nist_ftc_07_asme1_ap242-e2` | 1.726248e+06 | 1.726246e+06 | 8.3e-07 | 3.913714e+05 | 3.913714e+05 | 1.4e-09 | 7.5e-8 |
| `current/nist_ftc_09_asme1_rd` | 1.364451e+05 | 1.364451e+05 | 9.7e-11 | 9.661423e+04 | 9.661423e+04 | 7.0e-11 | 2.7e-9 |
| `current/nist_stc_08_asme1_ap242-e3` | 5.035922e+05 | 5.035922e+05 | 5.3e-10 | 1.916879e+05 | 1.916879e+05 | 4.6e-09 | 2.1e-10 |
| `current/nist_stc_10_asme1_ap242-e2` | 1.883122e+05 | 1.883129e+05 | 3.3e-06 | 4.848627e+04 | 4.848646e+04 | 3.7e-06 | 1.8e-7 |
| `2026-01/nist_ctc_01_asme1_ap203` | 1.464280e+07 | 1.464282e+07 | 1.4e-06 | 8.079686e+05 | 8.079686e+05 | 8.5e-08 | 2.4e-8 |
| `2026-01/nist_ctc_01_asme1_ap242-e1` | 1.464482e+07 | 1.464482e+07 | 5.8e-10 | 8.070808e+05 | 8.070808e+05 | 2.4e-08 | 6.5e-12 |
| `2026-01/nist_ctc_03_asme1_ap203` | 3.319383e+05 | 3.319383e+05 | 2.0e-07 | 3.549786e+05 | 3.549786e+05 | 8.5e-13 | 2.9e-8 |
| `2026-01/nist_ctc_04_asme1_ap242-e1` | 1.751978e+07 | 1.751978e+07 | 4.2e-10 | 1.022497e+06 | 1.022497e+06 | 3.9e-08 | 8.5e-11 |
| `2026-01/nist_ctc_05_asme1_ap203` | 1.269492e+07 | 1.269491e+07 | 8.5e-07 | 8.612430e+05 | 8.612427e+05 | 3.2e-07 | 6.8e-8 |

## Per-file results

`current/` is the current NIST package, `2026-01/` the January 2026 one.
Faces as stored, merged count in brackets; time is the whole procedure run
twice; peak is the process's resident memory.

| File | Schema | Faces (merged) | Import | Reference | Native RT | STEP RT | Box | Half split | Drill | Import (s) | Run ×2 (s) | Peak (MiB) | Same hash |
| --- | --- | ---: | --- | --- | --- | --- | --- | --- | --- | ---: | ---: | ---: | --- |
| `current/nist_ctc_01_asme1_ap242-e1` | AP242 e1 | 117 (105) | pass | pass | pass | pass | pass | pass | pass | 1.0 | 38 | 103 | yes |
| `current/nist_ctc_01_asme1_rd` | AP203 | 139 (139) | pass | n/a | pass | pass | pass | pass | pass | 0.6 | 31 | 114 | yes |
| `current/nist_ctc_02_asme1_ap242-e2` | AP242 e2 | 637 (461) | pass | pass | pass | pass | pass | pass | pass | 8.8 | 269 | 483 | yes |
| `current/nist_ctc_02_asme1_rc` | AP203 | 664 (494) | FAIL | pass | pass | pass | pass | pass | pass | 23.1 | 269 | 470 | yes |
| `current/nist_ctc_03_asme1_ap242-e2` | AP242 e2 | 120 (105) | pass | pass | pass | pass | pass | pass | pass | 0.7 | 28 | 95 | yes |
| `current/nist_ctc_03_asme1_rc` | AP203 | 139 (139) | pass | n/a | pass | pass | pass | pass | pass | 1.5 | 32 | 106 | yes |
| `current/nist_ctc_04_asme1_ap242-e2` | AP242 e2 | 484 (372) | pass | pass | pass | pass | pass | pass | pass | 8.5 | 208 | 419 | yes |
| `current/nist_ctc_04_asme1_rd` | AP203 | 520 (408) | FAIL | n/a | pass | pass | ERROR | ERROR | ERROR | 11.7 | 143 | 319 | yes |
| `current/nist_ctc_05_asme1_ap242-e1` | AP242 e1 | 156 (156) | FAIL | n/a | pass | pass | pass | pass | pass | 7.6 | 73 | 144 | yes |
| `current/nist_ctc_05_asme1_rd` | AP203 | 209 (162) | FAIL | pass | pass | pass | pass | ERROR | pass | 6.0 | 76 | 148 | yes |
| `current/nist_ftc_06_asme1_ap242-e2` | AP242 e2 | 187 (144) | pass | pass | pass | pass | pass | pass | pass | 1.4 | 63 | 145 | yes |
| `current/nist_ftc_06_asme1_rd` | AP203 | 144 (144) | pass | n/a | pass | pass | pass | pass | pass | 1.1 | 53 | 124 | yes |
| `current/nist_ftc_07_asme1_ap242-e2` | AP242 e2 | 306 (271) | FAIL | pass | pass | pass | ERROR | ERROR | ERROR | 26.7 | 255 | 284 | yes |
| `current/nist_ftc_07_asme1_rd` | AP203 | 269 (269) | FAIL | n/a | pass | pass | ERROR | ERROR | ERROR | 9.2 | 72 | 173 | yes |
| `current/nist_ftc_08_asme1_ap242-e2` | AP242 e2 | 247 (247) | pass | n/a | pass | pass | pass | pass | pass | 1.2 | 55 | 172 | yes |
| `current/nist_ftc_08_asme1_ap242-e4-tg` | - | 0 (-) | ERROR | — | — | — | — | — | — | 0.0 | 0 | 32 | yes |
| `current/nist_ftc_08_asme1_rc` | AP203 | 270 (247) | pass | n/a | pass | pass | pass | pass | pass | 3.7 | 89 | 166 | yes |
| `current/nist_ftc_09_asme1_ap242-e1` | AP242 e1 | 163 (130) | pass | n/a | pass | pass | pass | pass | pass | 1.1 | 53 | 136 | yes |
| `current/nist_ftc_09_asme1_rd` | AP203 | 158 (125) | pass | pass | pass | pass | pass | pass | pass | 0.8 | 43 | 110 | yes |
| `current/nist_ftc_10_asme1_ap242-e2` | AP242 e2 | 282 (228) | pass | n/a | pass | pass | pass | pass | pass | 7.0 | 169 | 318 | yes |
| `current/nist_ftc_10_asme1_rb` | AP203 | 214 (214) | pass | n/a | pass | pass | pass | pass | pass | 2.5 | 83 | 160 | yes |
| `current/nist_ftc_11_asme1_ap242-e3` | AP242 e3 | 42 (42) | FAIL | n/a | pass | pass | pass | pass | pass | 1.4 | 18 | 77 | yes |
| `current/nist_ftc_11_asme1_rb` | AP203 | 6 (6) | pass | n/a | pass | pass | pass | pass | pass | 0.1 | 9 | 53 | yes |
| `current/nist_stc_06_asme1_ap242-e3` | AP242 e3 | 144 (144) | pass | n/a | pass | pass | pass | pass | pass | 1.6 | 47 | 117 | yes |
| `current/nist_stc_07_asme1_ap242-e3` | AP242 e3 | 306 (271) | FAIL | n/a | pass | pass | ERROR | ERROR | ERROR | 14.2 | 138 | 210 | yes |
| `current/nist_stc_08_asme1_ap242-e3` | AP242 e3 | 271 (248) | pass | pass | pass | pass | pass | pass | pass | 2.1 | 76 | 196 | yes |
| `current/nist_stc_09_asme1_ap242-e4` | AP242 e4 | 163 (130) | FAIL | n/a | pass | pass | pass | pass | pass | 6.5 | 76 | 142 | yes |
| `current/nist_stc_10_asme1_ap242-e2` | AP242 e2 | 256 (256) | pass | pass | pass | pass | pass | pass | pass | 9.4 | 115 | 193 | yes |
| `2026-01/nist_ctc_01_asme1_ap203` | AP203 | 139 (139) | pass | pass | pass | pass | pass | pass | pass | 0.6 | 35 | 115 | yes |
| `2026-01/nist_ctc_01_asme1_ap242-e1` | AP242 e1 | 117 (105) | pass | pass | pass | pass | pass | pass | pass | 1.1 | 39 | 106 | yes |
| `2026-01/nist_ctc_02_asme1_ap203` | AP203 | 487 (487) | FAIL | n/a | pass | FAIL | ERROR | ERROR | ERROR | 21.4 | 146 | 312 | yes |
| `2026-01/nist_ctc_03_asme1_ap203` | AP203 | 139 (139) | pass | pass | pass | pass | pass | pass | pass | 1.1 | 30 | 107 | yes |
| `2026-01/nist_ctc_04_asme1_ap203` | AP203 | 406 (406) | pass | n/a | pass | pass | pass | pass | pass | 2.6 | 105 | 283 | yes |
| `2026-01/nist_ctc_04_asme1_ap242-e1` | AP242 e1 | 484 (372) | pass | pass | pass | pass | pass | pass | pass | 8.9 | 216 | 394 | yes |
| `2026-01/nist_ctc_05_asme1_ap203` | AP203 | 156 (156) | FAIL | pass | pass | pass | pass | pass | pass | 6.3 | 70 | 141 | yes |
| `2026-01/nist_ctc_05_asme1_ap242-e1` | AP242 e1 | 156 (156) | FAIL | n/a | pass | pass | pass | pass | pass | 7.8 | 74 | 144 | yes |
| `2026-01/nist_ftc_08_asme1_ap242-e1-tg` | - | 0 (-) | ERROR | — | — | — | — | — | — | 0.0 | 0 | 33 | yes |
| `2026-01/nist_ftc_11_asme1_ap242-e2` | AP242 e2 | 42 (42) | FAIL | n/a | pass | pass | pass | pass | pass | 1.5 | 17 | 77 | yes |
| `2026-01/nist_stc_09_asme1_ap242-e3` | AP242 e3 | 125 (125) | pass | n/a | pass | pass | pass | pass | pass | 4.4 | 76 | 144 | yes |

## Same part, different exporters (information, not graded)

Volumes of the closed solids as read; each matches its own file's stated
values where it states any (finding 7).

| Test case | Files | Volume spread (max−min)/median | Area spread | Volumes (mm³) |
| --- | ---: | ---: | ---: | --- |
| CTC-01 | 4 | 1.4e-04 | 1.1e-03 | ctc_01_asme1_ap242-e1 1.464482e+07; ctc_01_asme1_rd 1.464282e+07; ctc_01_asme1_ap203 1.464282e+07; ctc_01_asme1_ap242-e1 1.464482e+07 |
| CTC-02 | 3 | 1.1e-04 | 4.9e-04 | ctc_02_asme1_ap242-e2 4.710681e+07; ctc_02_asme1_rc 4.710162e+07; ctc_02_asme1_ap203 4.710155e+07 |
| CTC-03 | 3 | 5.6e-04 | 5.4e-04 | ctc_03_asme1_ap242-e2 3.321229e+05; ctc_03_asme1_rc 3.319383e+05; ctc_03_asme1_ap203 3.319383e+05 |
| CTC-04 | 4 | 5.1e-05 | 5.8e-04 | ctc_04_asme1_ap242-e2 1.751978e+07; ctc_04_asme1_rd 1.751909e+07; ctc_04_asme1_ap203 1.751889e+07; ctc_04_asme1_ap242-e1 1.751978e+07 |
| CTC-05 | 4 | 1.3e-06 | 7.0e-07 | ctc_05_asme1_ap242-e1 1.269489e+07; ctc_05_asme1_rd 1.269491e+07; ctc_05_asme1_ap203 1.269491e+07; ctc_05_asme1_ap242-e1 1.269489e+07 |
| FTC-06 | 2 | 3.1e-06 | 8.3e-06 | ftc_06_asme1_ap242-e2 3.323557e+06; ftc_06_asme1_rd 3.323546e+06 |
| FTC-07 | 2 | 4.9e-11 | 5.9e-08 | ftc_07_asme1_ap242-e2 1.726246e+06; ftc_07_asme1_rd 1.726246e+06 |
| FTC-08 | 2 | 8.1e-05 | 9.4e-05 | ftc_08_asme1_ap242-e2 5.036330e+05; ftc_08_asme1_rc 5.035922e+05 |
| FTC-09 | 2 | 3.0e-09 | 2.6e-09 | ftc_09_asme1_ap242-e1 1.364451e+05; ftc_09_asme1_rd 1.364451e+05 |
| FTC-10 | 2 | 1.4e-11 | 3.2e-07 | ftc_10_asme1_ap242-e2 1.883180e+05; ftc_10_asme1_rb 1.883180e+05 |
| FTC-11 | 3 | 1.1e-03 | 1.6e-04 | ftc_11_asme1_ap242-e3 5.135041e+03; ftc_11_asme1_rb 5.129386e+03; ftc_11_asme1_ap242-e2 5.135041e+03 |
| STC-09 | 2 | 2.3e-09 | 2.0e-06 | stc_09_asme1_ap242-e4 1.364451e+05; stc_09_asme1_ap242-e3 1.364451e+05 |

## Environment

Linux x86_64 sandbox, 2 vCPU (Intel Xeon 2.30 GHz), 7 GB RAM, Rust 1.97.0
(the installed stable toolchain; rustup cannot download the pinned one by
name here). Optimised build as in "How to reproduce" (`opt-level = 1`;
`opt-level = 3` does not fit SolidCore's compile in 7 GB). Timings are wall
time on a shared machine and indicate scale, not a benchmark. This run's
per-file results, with the Linux x86_64 hashes the nightly cross-OS
comparison starts from, are in `kernel-nist-step-stress-2026-11-21.tsv`
next to this report (columns: `pxp-cli stepstress --format tsv --header`).

NIST MBE PMI test models courtesy of the National Institute of Standards
and Technology; use here does not imply NIST endorsement.
