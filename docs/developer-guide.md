<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# Developer guide (Stage 0)

## Build and run

```sh
cargo build --release -p pxp-app -p pxp-cli   # pxp (desktop) and pxp-cli
target/release/pxp                            # the shell
target/release/pxp-cli status                 # what exists, what is planned
```

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.97, edition
2024). SolidCore, the SolidCraft kernel's base, is about 40 MB of Rust. A
debug build of it needs about 6 GB of RAM; an opt-level-3 release build
needs about 5 GB with `codegen-units=64`. On small machines, use
`CARGO_PROFILE_RELEASE_OPT_LEVEL=1`, as the nightly workflow does.

The OCCT backend runs in a worker process. Today the worker is Python on
the OCP bindings: `pip install cadquery-ocp==7.8.1.1`, then point
`PXP_OCCT_PYTHON` at that Python. `pxp --kernel occt` and
`pxp-cli bakeoff --kernel occt` use it. Without it, the OCCT tests skip
and say so.

## Checks (run all before a commit; CI runs the same)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings   # pedantic; unwrap/expect/indexing/panic denied
cargo test --workspace
cargo deny check
python3 tools/check_repo.py                            # layers, unsafe, docs, SPDX headers
```

GPU tests (`crates/pxp-render/tests/gpu.rs`) need any wgpu adapter. A
software rasteriser is enough: Mesa's lavapipe on Linux (`apt install
mesa-vulkan-drivers`), or WARP on Windows. Without one they skip and say
so.

## Layers

`tools/layers.toml` defines the dependency direction, and
`tools/check_repo.py layers` enforces it:

```text
foundation (units, math, platform, compute, diag)
  → document (pxp-doc)
    → storage and geometry (pxp-format, pxp-kernel, kernel backends, sketch, tess)
      → services (appkit, render, io, cam, …)
        → applications (pxp-app-*)
          → shell (pxp-ui, pxp-app, pxp-cli)
```

Only the shell may name a kernel backend. Applications never depend on
each other.

## Adding a command

1. Add a row to the application's command table, for example
   `crates/pxp-app-modeling/src/lib.rs`: id, label, ribbon tab and group,
   default shortcut, planned stage, and a description in our own words.
   The ribbon, Command Finder, tooltips and shortcut table pick it up with
   no UI code.
2. When it works, make its status `Available`, put the domain logic in the
   application crate (a transaction on the document), and bind its dialog
   in `pxp-ui::shell::State::run_command`. An available command with no
   binding reports itself on the status line, so it is never silently
   inert.
3. Add a user-guide page under `docs/user-guide/<application>/`.

## Dialogs

`pxp-ui::dialog::Dialog` is data: groups of formula fields, plus OK, Apply
and Cancel. The shell draws it, mirrors the active field in the on-screen
input box, and maps MB2 to OK and Esc to Cancel. Values are checked by
running the transaction, so the document's errors appear in the dialog.

## Kernel backends

Implement `pxp_kernel::GeometryKernel` and run every operation through
`KernelHost::op`, which rejects incomplete topology histories. Then:

- Pass the backend's conformance tests (copy
  `crates/pxp-kernel-native/tests/conformance.rs`). Every expected value
  must come from a hand calculation, never from another kernel.
- Run `pxp-cli bakeoff --kernel <name> --corpus all` and `stepstress` on
  the NIST files (`tools/fetch_nist_step.py`).
- Compare with `tools/compare_bakeoff.py`.

## Viewport

`pxp-render` draws offscreen into its own textures, so the same code
serves the shell, tests, screenshots and `pxp bench-orbit`. Camera maths
and the NX mouse grammar are pure and unit-tested; the GPU paths are
covered by the headless tests above.

## Screenshots and scripted runs

```sh
xvfb-run -s "-screen 0 1600x900x24" target/release/pxp \
  --new /tmp/demo.pxp --block "0 0 0 100 60 40" --dialog block \
  --select-center --screenshot shot.png
```

`pxp --help` lists the flags. The verification packet uses these runs.

## Benchmarks and budgets

`cargo bench -p pxp-doc --bench doc`, `-p pxp-format --bench file` and
`-p pxp-kernel-native --bench kernel`. The budgets are in
`docs/design/performance-budgets.json`. The CI `bench` job fails a
benchmark that exceeds its budget, or that regresses more than 10 %
against the base commit measured on the same runner.

## Fuzzing

`fuzz/` holds the cargo-fuzz targets (nightly toolchain):
`cargo +nightly fuzz run decode_object`. CI runs 60 s per target and the
nightly workflow 1 hour per target. On stable,
`PXP_SOAK_SECS=600 cargo test --release -p pxp-format --test codec_soak`
runs a seeded mutation soak with the same properties. It is not
coverage-guided.

## Journals and MCP

`pxp-cli journal record FILE -- part new …` records commands with their
geometry hashes. `journal replay FILE` re-runs them in a fresh folder and
fails on any hash change. Journals under `crates/pxp-cli/tests/journals/`
are regression tests. `pxp-cli mcp` exposes the same commands over
JSON-RPC 2.0 on stdio. With `--journal FILE`, an agent's session becomes
a journal.

## Crash reports

`pxp_diag::crash::install` is called first in each binary's `main`. A
panic writes a local text report (no document contents) and nothing is
sent anywhere. `pxp-cli crashes` lists the reports.
