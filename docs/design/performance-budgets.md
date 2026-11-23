<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# Performance budgets (Stage 0)

The master prompt asks for measurable budgets for frame rate, open time and
recompute time, tracked with criterion in CI, where a regression beyond
10 % fails the build. The budgets live in `performance-budgets.json`; the
CI `bench` job checks them with `tools/ci/check_bench.py`.

## How the gate works

1. **Absolute budget.** Each benchmark's mean must stay under its
   `budget_ms`. Budgets are about 3× (frame-time budgets more) the
   measurement on the 2-vCPU development machine, so a slower shared
   runner still passes. They are also set below what a user would notice.
2. **Regression.** On a pull request, the job benchmarks the base commit
   and then the pull request on the same runner, back to back. Any mean
   more than 10 % slower fails. Comparing two different CI machines would
   be noise.

## Budgets and measurements (2026-11-21, Linux x86-64, 2 vCPU, opt-level 3)

| Benchmark | What it stands for | Measured | Budget |
| --- | --- | ---: | ---: |
| `doc/edit-one-of-100k` | one committed edit in a 100 000-object document (interactive latency) | 0.25–0.30 ms | 2 ms |
| `doc/undo-redo-100-steps` | 100 undo then 100 redo steps | 0.75 ms | 10 ms |
| `doc/recompute-chain-1000` | edit the head of a 1 000-expression chain, recompute (**recompute time**) | 18.2 ms | 100 ms |
| `file/open-10k-objects` | open a 10 000-object part (**open time**) | 24.2 ms | 250 ms |
| `file/save-10k-objects` | save a 10 000-object part | 27.3 ms | 250 ms |
| `kernel/native/block` | a block through `KernelHost::op` | 2.48 ms | 10 ms |
| `kernel/native/drill-block` | block minus a through-hole (commonest feature recompute) | 8.99 ms | 30 ms |
| `kernel/native/mass-properties` | volume, area and centroid of the drilled block | 10.3 ms | 35 ms |

`doc/edit-one-of-100k` was the reason for the persistent map (ADR-0006).
Before it, every transaction cloned the whole object map: about 1 ms per
100 000 objects, and growing with the document.

## Frame rate

**Proposed budget: orbiting the 5-million-triangle benchmark model at
1920×1080 holds at least 30 frames per second on the reference GPU.** The
reference GPU is a user decision; a mid-range discrete card such as an RTX
3060 or RX 6600 is proposed. Measure with:

```sh
pxp bench-orbit --triangles 5000000 --frames 120 --size 1920x1080
```

It renders headless (no window), waits for the GPU after every frame so
the times are honest, and reports mean fps, the worst frame and the
adapter. The development machine has no GPU. On Mesa's software
rasteriser (llvmpipe, 2 vCPU) it measured 2.10 fps at 5 million triangles
and 7.52 fps at 1 million. That only shows the software fallback works; it
says nothing about the budget. Frame rate is not a criterion benchmark,
because CI runners have no GPUs.

The kernel benchmarks measure SolidCraft, the backend linked today. When
OCCT becomes the primary kernel (ADR-0004), an OCCT benchmark joins them
with the same budgets.
