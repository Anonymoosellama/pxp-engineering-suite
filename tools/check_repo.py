#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
"""Repository policy checks for PXP Engineering Suite.

Runs in CI (and locally) before clippy and the tests. Each check enforces a
rule from docs/MASTER_PROMPT.md that the Rust compiler cannot enforce on its
own:

  layers   Dependency direction between workspace crates (tools/layers.toml).
  unsafe   Every crate root carries #![forbid(unsafe_code)] unless allowlisted.
  docs     Every crate root opens with //! crate documentation.
  spdx     Every source/config file carries an SPDX licence header (ADR-0001).

Usage:  python3 tools/check_repo.py [layers|unsafe|docs|spdx ...]
        (no arguments runs every check)

Exit status is 0 when every selected check passes, 1 otherwise. The script
uses only the Python standard library (tomllib needs Python 3.11+), and gets
the dependency graph from `cargo metadata --no-deps`, which works offline.
"""

from __future__ import annotations

import json
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPDX = "SPDX-License-Identifier: LGPL-2.1-or-later"

# File types that must carry an SPDX header, and paths that are exempt
# (generated files, or formats that cannot hold comments).
SPDX_SUFFIXES = {".rs", ".toml", ".yml", ".yaml", ".py", ".sh", ".wit"}
SPDX_EXEMPT_NAMES = {"Cargo.lock"}
SKIP_DIRS = {".git", "target", "node_modules"}


def load_layers() -> tuple[dict[str, tuple[int, str]], set[str], set[str]]:
    """Return {crate: (layer_index, layer_name)}, kernel backends, unsafe allowlist."""
    data = tomllib.loads((ROOT / "tools" / "layers.toml").read_text(encoding="utf-8"))
    table: dict[str, tuple[int, str]] = {}
    for index, layer in enumerate(data["layer"]):
        for crate in layer["crates"]:
            if crate in table:
                raise SystemExit(f"layers.toml: {crate} is listed in two layers")
            table[crate] = (index, layer["name"])
    backends = set(data.get("kernel", {}).get("backends", []))
    unsafe_allow = set(data.get("unsafe", {}).get("allow", []))
    return table, backends, unsafe_allow


def cargo_metadata() -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(out.stdout)


def crate_roots(meta: dict) -> dict[str, Path]:
    """Map each workspace package to its library root, or its binary root."""
    roots: dict[str, Path] = {}
    for pkg in meta["packages"]:
        lib = [t for t in pkg["targets"] if "lib" in t["kind"]]
        binary = [t for t in pkg["targets"] if "bin" in t["kind"]]
        target = (lib or binary or [None])[0]
        if target is not None:
            roots[pkg["name"]] = Path(target["src_path"])
    return roots


def check_layers(meta: dict) -> list[str]:
    table, backends, _ = load_layers()
    errors: list[str] = []
    names = {pkg["name"] for pkg in meta["packages"]}
    for name in sorted(names - table.keys()):
        errors.append(f"[layers] {name} is not assigned to a layer in tools/layers.toml")
    for name in sorted(table.keys() - names):
        errors.append(f"[layers] tools/layers.toml lists {name}, which is not in the workspace")
    for pkg in meta["packages"]:
        src = pkg["name"]
        if src not in table:
            continue
        src_index, src_layer = table[src]
        for dep in pkg["dependencies"]:
            dst = dep["name"]
            if dst not in table:
                continue  # external crate: governed by cargo-deny, not layering
            dst_index, dst_layer = table[dst]
            if dst_index > src_index:
                errors.append(
                    f"[layers] {src} ({src_layer}) depends on {dst} ({dst_layer}): "
                    "lower layers must never depend on higher ones"
                )
            if src_layer == "applications" and dst_layer == "applications":
                errors.append(
                    f"[layers] application {src} depends on application {dst}: "
                    "applications share domain libraries, never each other"
                )
            if dst in backends and src_layer != "shell":
                errors.append(
                    f"[layers] {src} depends on kernel backend {dst}: only shell "
                    "crates wire backends; everything else uses the pxp-kernel trait"
                )
    return errors


def first_code_lines(path: Path, count: int = 5) -> list[str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    return [line.strip() for line in lines[:count]]


def check_unsafe(meta: dict) -> list[str]:
    _, _, unsafe_allow = load_layers()
    errors = []
    for name, root in sorted(crate_roots(meta).items()):
        if name in unsafe_allow:
            continue
        if "#![forbid(unsafe_code)]" not in root.read_text(encoding="utf-8"):
            errors.append(f"[unsafe] {name}: {root.relative_to(ROOT)} lacks #![forbid(unsafe_code)]")
    return errors


def check_docs(meta: dict) -> list[str]:
    errors = []
    for name, root in sorted(crate_roots(meta).items()):
        lines = [l for l in first_code_lines(root, 3) if l and not l.startswith("// SPDX")]
        if not lines or not lines[0].startswith("//!"):
            errors.append(f"[docs] {name}: crate root must open with //! documentation")
    return errors


def iter_files() -> list[Path]:
    files = []
    for path in ROOT.rglob("*"):
        if any(part in SKIP_DIRS for part in path.relative_to(ROOT).parts):
            continue
        if path.is_file() and path.suffix in SPDX_SUFFIXES and path.name not in SPDX_EXEMPT_NAMES:
            files.append(path)
    return sorted(files)


def check_spdx() -> list[str]:
    errors = []
    for path in iter_files():
        head = "\n".join(first_code_lines(path, 3))
        if SPDX not in head:
            errors.append(f"[spdx] {path.relative_to(ROOT)} lacks '{SPDX}' in its first 3 lines")
    return errors


def main(argv: list[str]) -> int:
    selected = set(argv) or {"layers", "unsafe", "docs", "spdx"}
    unknown = selected - {"layers", "unsafe", "docs", "spdx"}
    if unknown:
        print(f"unknown check(s): {', '.join(sorted(unknown))}", file=sys.stderr)
        return 2
    errors: list[str] = []
    meta = cargo_metadata() if selected & {"layers", "unsafe", "docs"} else {}
    if "layers" in selected:
        errors += check_layers(meta)
    if "unsafe" in selected:
        errors += check_unsafe(meta)
    if "docs" in selected:
        errors += check_docs(meta)
    if "spdx" in selected:
        errors += check_spdx()
    for line in errors:
        print(line)
    print(f"check_repo: {len(errors)} problem(s) in {', '.join(sorted(selected))}")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
