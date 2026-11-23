<!-- SPDX-License-Identifier: LGPL-2.1-or-later -->
# pxp-cli

`pxp-cli` runs PXP without a window. `pxp-cli --help` lists every command.

| Command | What it does |
| --- | --- |
| `pxp-cli part new FILE [--units mm\|in]` | New part from a template |
| `pxp-cli part block FILE --size X Y Z [--on-datum]` (also `cylinder`, `cone`, `sphere`) | Adds a primitive, recomputes and saves |
| `pxp-cli part info FILE` | Features, kernel, geometry hash, mass properties |
| `pxp-cli part verify FILE` | Recomputes everything and compares geometry hashes |
| `pxp-cli eval EXPR` | Evaluates a unit-aware expression (`2in + 3mm`) |
| `pxp-cli commands`, `find TEXT`, `apps` | The command registry |
| `pxp-cli journal record FILE -- CMD…` / `journal replay FILE` | Records a session; replays it in a fresh folder and compares every geometry hash |
| `pxp-cli mcp [--journal FILE]` | MCP server over stdio (JSON-RPC 2.0) offering the same commands to AI agents |
| `pxp-cli bakeoff [--kernel native\|occt]` | The kernel bake-off corpus |
| `pxp-cli stepstress FILE…` | STEP import stress test against the files' own validation properties |
| `pxp-cli crashes [--clear]` | Local crash reports |

Journals (`.pxj`) are JSON lines. The ones in `crates/pxp-cli/tests/journals`
are replayed by the test suite, so a change that alters any geometry hash
fails the build.
