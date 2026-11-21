#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
"""Convert SolidCraft's kernel regression cases into PXP bake-off cases.

SolidCraft (MIT OR Apache-2.0, taken under MIT) keeps ~1000 kernel cases in
`tests/kernel-cases/<area>/<id>/`, each a `recipe.json` (SolidCraft commands)
and an `expected.json` whose answer comes from analysis, never from a kernel.
PXP's Stage 0 kernel trait has only primitives and booleans, so this script
keeps the cases that use nothing else and translates them into the
`pxp_kernel::bakeoff` vocabulary. The output is Rust source, committed as
`crates/pxp-kernel/src/bakeoff/solidcraft.rs`; rerun this script after
moving the pinned SolidCraft revision.

Usage:
    python3 tools/import_solidcraft_cases.py SOLIDCRAFT_CHECKOUT > \
        crates/pxp-kernel/src/bakeoff/solidcraft.rs
    cargo fmt -p pxp-kernel

Translation rules (SolidCraft command -> PXP):

- `solid.box` {length, width, height, corner | center}: a block; `corner`
  defaults to the origin, `center` is converted to the min corner.
- `solid.cylinder` {radius | diameter, height, base, axis}: a cylinder;
  `axis` is "X", "Y", "Z" or a vector (default Z). A negative height runs
  along the reversed axis from `base`.
- `solid.cone` {radius, top_radius, height, base, axis}: a cone, same rules.
- `solid.sphere` {radius | diameter, center}.
- A primitive with `operation` cut/join/intersect becomes a tool body
  combined into `targets`; with no `targets`, into the one existing body
  (cut: into every existing body). Anything else is skipped as needing
  SolidCraft's own multi-body target rules.
- `solid.combine` {target, tools, operation, keep_tools}. An intersect's
  tools are first united, as SolidCraft does.
- A case with `timeout_s` (slow by nature in SolidCraft's own rig) is
  marked `slow`: the harness skips it unless asked.
- Expected values: `volume_mm3`, `area_mm2` (relative tolerance
  `tolerance.rel`, default 1e-3, or `[min, max]`), `faces`, `shells`;
  `outcome: "refused"` becomes an expected refusal. Body counts are not
  graded (see the harness documentation).

Cases with expression strings, unknown commands or unknown parameters are
skipped and listed on stderr with the reason.
"""

from __future__ import annotations

import json
import math
import subprocess
import sys
from pathlib import Path

AREAS_ORDER = None  # every area, sorted
PRIMS = {"solid.box", "solid.cylinder", "solid.sphere", "solid.cone"}
KNOWN = PRIMS | {"solid.combine"}
OPS = {"cut": "Subtract", "join": "Unite", "intersect": "Intersect"}
ALLOWED_PARAMS = {
    "solid.box": {"length", "width", "height", "corner", "center", "operation", "targets", "body_name", "name"},
    "solid.cylinder": {"radius", "diameter", "height", "base", "axis", "operation", "targets", "body_name", "name"},
    "solid.cone": {"radius", "top_radius", "height", "base", "axis", "operation", "targets", "body_name", "name"},
    "solid.sphere": {"radius", "diameter", "center", "operation", "targets", "body_name", "name"},
    "solid.combine": {"target", "tools", "operation", "keep_tools"},
}


class Skip(Exception):
    """This case cannot be expressed faithfully."""


def num(v) -> float:
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        raise Skip(f"non-numeric value {v!r}")
    return float(v)


def vec(v, default=(0.0, 0.0, 0.0)) -> list[float]:
    if v is None:
        return list(default)
    if not isinstance(v, list) or len(v) != 3:
        raise Skip(f"not a 3-vector: {v!r}")
    return [num(c) for c in v]


def axis(v) -> list[float]:
    if v is None:
        return [0.0, 0.0, 1.0]
    if isinstance(v, str):
        named = {"X": [1.0, 0.0, 0.0], "Y": [0.0, 1.0, 0.0], "Z": [0.0, 0.0, 1.0]}
        if v.upper() not in named:
            raise Skip(f"unknown axis {v!r}")
        return named[v.upper()]
    a = vec(v)
    n = math.sqrt(sum(c * c for c in a))
    if n == 0:
        raise Skip("zero axis")
    return [c / n for c in a]


def rs(x: float) -> str:
    """A Rust f64 literal that round-trips exactly."""
    r = repr(float(x))
    if "e" in r or "E" in r:
        mant, exp = r.lower().split("e")
        if "." not in mant:
            mant += ".0"
        return f"{mant}e{int(exp)}"
    if "." not in r and "inf" not in r and "nan" not in r:
        r += ".0"
    return r


def arr(v: list[float]) -> str:
    return "[" + ", ".join(rs(c) for c in v) + "]"


def rstr(s: str) -> str:
    return json.dumps(s, ensure_ascii=False)


class Interp:
    def __init__(self) -> None:
        self.bodies: list[str] = []
        self.steps: list[str] = []
        self.tools = 0
        self.auto = 0

    def new_name(self, params) -> str:
        name = params.get("body_name") or params.get("name")
        if name is None:
            self.auto += 1
            name = f"Body{self.auto}"
        if not isinstance(name, str):
            raise Skip("non-text body name")
        if name in self.bodies:
            raise Skip(f"body name {name!r} reused")
        return name

    def prim(self, cmd: str, p: dict) -> str:
        if cmd == "solid.box":
            size = [num(p["length"]), num(p["width"]), num(p["height"])]
            if "center" in p:
                c = vec(p["center"])
                corner = [c[i] - size[i] / 2 for i in range(3)]
            else:
                corner = vec(p.get("corner"))
            return f"block({arr(corner)}, {arr(size)})"
        if cmd in ("solid.cylinder", "solid.cone"):
            if "diameter" in p:
                d = num(p["diameter"])
            else:
                d = 2 * num(p["radius"])
            h = num(p["height"])
            a = axis(p.get("axis"))
            if h < 0:
                a = [-c for c in a]
                h = -h
            base = vec(p.get("base"))
            if cmd == "solid.cylinder":
                return f"cylinder({arr(base)}, {arr(a)}, {rs(d)}, {rs(h)})"
            top = 2 * num(p.get("top_radius", 0))
            return f"cone({arr(base)}, {arr(a)}, {rs(d)}, {rs(top)}, {rs(h)})"
        if cmd == "solid.sphere":
            d = num(p["diameter"]) if "diameter" in p else 2 * num(p["radius"])
            return f"sphere({arr(vec(p.get('center')))}, {rs(d)})"
        raise Skip(f"unknown primitive {cmd}")

    def combine(self, target: str, tools: list[str], op: str, keep: bool) -> None:
        if target not in self.bodies:
            raise Skip(f"unknown target {target!r}")
        for t in tools:
            if t not in self.bodies:
                raise Skip(f"unknown tool {t!r}")
        names = ", ".join(rstr(t) for t in tools)
        self.steps.append(f"combine({rstr(target)}, &[{names}], {op}, {'true' if keep else 'false'})")
        if not keep:
            self.bodies = [b for b in self.bodies if b not in tools]

    def command(self, cmd: str, p: dict) -> None:
        if cmd not in KNOWN:
            raise Skip(f"command {cmd}")
        extra = set(p) - ALLOWED_PARAMS[cmd]
        if extra:
            raise Skip(f"{cmd} parameters {sorted(extra)}")
        if cmd == "solid.combine":
            op = OPS.get(str(p.get("operation", "")).lower())
            if op is None:
                raise Skip(f"combine operation {p.get('operation')!r}")
            tools = p.get("tools") or []
            keep = bool(p.get("keep_tools", False))
            if op == "Intersect" and len(tools) > 1:
                if keep:
                    raise Skip("intersect keeping several tools")
                self.combine(tools[0], tools[1:], "Unite", False)
                tools = tools[:1]
            self.combine(p["target"], tools, op, keep)
            return
        operation = str(p.get("operation", "new")).lower()
        if operation in ("new", "newbody", ""):
            name = self.new_name(p)
            self.steps.append(f"make({rstr(name)}, {self.prim(cmd, p)})")
            self.bodies.append(name)
            return
        op = OPS.get(operation)
        if op is None:
            raise Skip(f"operation {operation!r}")
        targets = p.get("targets")
        if targets is None:
            if len(self.bodies) == 1 or (op == "Subtract" and self.bodies):
                targets = list(self.bodies)
            else:
                raise Skip(f"{operation} with no targets among {len(self.bodies)} bodies")
        self.tools += 1
        tool = f"_tool{self.tools}"
        self.steps.append(f"make({rstr(tool)}, {self.prim(cmd, p)})")
        self.bodies.append(tool)
        for i, t in enumerate(targets):
            self.combine(t, [tool], op, i + 1 < len(targets))


def expected(e: dict) -> str:
    if e.get("outcome") == "refused":
        return "Expect::Refused"
    rel = float(e.get("tolerance", {}).get("rel", 1e-3))

    def rng(key: str) -> str:
        if key not in e:
            return "None"
        v = e[key]
        if isinstance(v, list):
            return f"Some(Range::between({rs(num(v[0]))}, {rs(num(v[1]))}))"
        return f"Some(Range::rel({rs(num(v))}, {rs(rel)}))"

    def count(key: str) -> str:
        if key not in e:
            return "None"
        v = e[key]
        if isinstance(v, list):
            raise Skip(f"{key} given as a range")
        return f"Some({int(v)})"

    if "volume_mm3" not in e and "area_mm2" not in e and "faces" not in e and "shells" not in e:
        raise Skip("no gradable expectation")
    return f"solid({rng('volume_mm3')}, {rng('area_mm2')}, {count('faces')}, {count('shells')})"


MIT_NOTICE = """\
// Copyright (c) 2026 SolidCraft contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE."""


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print(__doc__, file=sys.stderr)
        return 2
    root = Path(argv[0])
    cases_dir = root / "tests" / "kernel-cases"
    rev = subprocess.run(
        ["git", "-C", str(root), "rev-parse", "HEAD"], capture_output=True, text=True, check=True
    ).stdout.strip()
    out: list[str] = []
    skipped: list[tuple[str, str]] = []
    considered = 0
    for area in sorted(p for p in cases_dir.iterdir() if p.is_dir()):
        for case in sorted(p for p in area.iterdir() if p.is_dir()):
            recipe_path, expected_path = case / "recipe.json", case / "expected.json"
            if not recipe_path.exists() or not expected_path.exists():
                continue
            cid = f"{area.name}/{case.name}"
            try:
                recipe = json.loads(recipe_path.read_text(encoding="utf-8"))
                exp = json.loads(expected_path.read_text(encoding="utf-8"))
            except (OSError, ValueError) as err:
                skipped.append((cid, f"unreadable: {err}"))
                continue
            cmds = recipe.get("commands", [])
            if not cmds or any(c.get("command") not in KNOWN for c in cmds):
                continue  # uses commands PXP does not have yet
            considered += 1
            try:
                it = Interp()
                for c in cmds:
                    it.command(c["command"], c.get("params", {}))
                expect = expected(exp)
            except (Skip, KeyError) as why:
                skipped.append((cid, str(why)))
                continue
            sources = ", ".join(exp.get("sources", [])) or "SolidCraft kernel case"
            what = " ".join(str(exp.get("what", "")).split())
            steps = ",\n                ".join(it.steps)
            out.append(
                f"""        Case {{
            id: {rstr('solidcraft/' + cid)},
            source: {rstr(sources)},
            what: {rstr(what)},
            steps: vec![
                {steps},
            ],
            expect: {expect},
            slow: {'true' if 'timeout_s' in exp else 'false'},
        }},"""
            )
    print("// SPDX-License-Identifier: LGPL-2.1-or-later AND MIT")
    print("// @generated by tools/import_solidcraft_cases.py: do not edit by hand.")
    print("//")
    print(f"// Translated from SolidCraft's tests/kernel-cases at commit {rev}")
    print("// (https://github.com/tangency-labs/solidcraft). The case data (names,")
    print("// descriptions, expected answers) is SolidCraft's, used under the MIT licence:")
    print("//")
    print(MIT_NOTICE)
    print("//! SolidCraft's analytic kernel regression cases that need only primitives")
    print("//! and booleans, translated by `tools/import_solidcraft_cases.py`.")
    print("//!")
    print(f"//! {len(out)} of {considered} candidate cases translated; the script lists the")
    print("//! others and why on stderr.")
    print()
    print("use super::{Case, Expect, Range, block, combine, cone, cylinder, make, solid, sphere};")
    print("use crate::BooleanOp::{Intersect, Subtract, Unite};")
    print()
    print("/// The translated cases, sorted by area and id.")
    print("#[must_use]")
    print("#[allow(clippy::too_many_lines, clippy::unreadable_literal, clippy::excessive_precision, unused_imports)]")
    print("pub fn cases() -> Vec<Case> {")
    print("    vec![")
    for c in out:
        print(c)
    print("    ]")
    print("}")
    for cid, why in skipped:
        print(f"skipped {cid}: {why}", file=sys.stderr)
    print(f"translated {len(out)} of {considered} candidates", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
