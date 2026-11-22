#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
"""Compare two kernels' bake-off results and print Markdown tables.

Usage:
  compare_bakeoff.py primitive NATIVE.md NATIVE.details OCCT.md OCCT.details
  compare_bakeoff.py nist NATIVE.tsv OCCT.tsv

`primitive` reads `pxp-cli bakeoff --format markdown` (per-case grade and
time) and `--format details` (volume, area, `f{stored}/{merged}` faces) of
each kernel. `nist` reads `pxp-cli stepstress --format tsv --header`.
Produces the tables of docs/reports/kernel-bakeoff-2026-11-21-comparison.md:
pass counts and median / p95 times per corpus, the cases whose grades
differ, and how closely the two kernels agree where both pass. Standard
library only.
"""

from __future__ import annotations

import collections
import re
import statistics
import sys
from pathlib import Path

CHECKS = ["import", "reference", "native-roundtrip", "step-roundtrip", "box-identity", "half-split", "drill"]
CORPUS = {"syn": "Synthetic stress", "solidcraft": "SolidCraft reference", "gen": "Generated families"}


def cases(md: Path) -> dict[str, tuple[str, int]]:
    """Per-case table rows: id -> (grade, milliseconds)."""
    out = {}
    for line in md.read_text(encoding="utf-8").splitlines():
        m = re.match(r"\| `([^`]+)` \| (\w+) \| (\d+) \|", line)
        if m:
            out[m.group(1)] = (m.group(2), int(m.group(3)))
    return out


def details(path: Path) -> dict[str, tuple[float, float, int | None]]:
    """id -> (volume, area, merged faces)."""
    out = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        v = re.search(r"V=(\S+)", line)
        a = re.search(r"A=(\S+)", line)
        f = re.search(r"\bf(\d+)/(\d+)", line)
        if v and a:
            out[line.split()[0]] = (float(v.group(1)), float(a.group(1)), int(f.group(2)) if f else None)
    return out


def pct(xs: list[float], q: float) -> float:
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(q * (len(xs) - 1))))]


def primitive(nmd: Path, ndet: Path, omd: Path, odet: Path) -> None:
    nc, oc = cases(nmd), cases(omd)
    nd, od = details(ndet), details(odet)
    ids = sorted(set(nc) & set(oc))
    groups: dict[str, list[str]] = collections.defaultdict(list)
    for i in ids:
        groups[CORPUS.get(i.split("/")[0], "Other")].append(i)
    groups["All"] = ids
    print("| Corpus | Cases | Native pass | OCCT pass | Native median / p95 (ms) | OCCT median / p95 (ms) |")
    print("| --- | ---: | ---: | ---: | ---: | ---: |")
    for g in [*CORPUS.values(), "Other", "All"]:
        xs = groups.get(g, [])
        if not xs:
            continue
        nt = [nc[i][1] for i in xs]
        ot = [oc[i][1] for i in xs]
        print(
            f"| {g} | {len(xs)} | {sum(nc[i][0] == 'pass' for i in xs)} | {sum(oc[i][0] == 'pass' for i in xs)} "
            f"| {statistics.median(nt):.0f} / {pct(nt, .95):.0f} | {statistics.median(ot):.0f} / {pct(ot, .95):.0f} |"
        )
    print("\nGrades that differ:\n")
    for i in ids:
        if nc[i][0] != oc[i][0]:
            print(f"- `{i}`: native {nc[i][0]}, OCCT {oc[i][0]}")
    dv, da, df = [], [], []
    for i in ids:
        if i in nd and i in od and nc[i][0] == oc[i][0] == "pass":
            (v1, a1, f1), (v2, a2, f2) = nd[i], od[i]
            if v1 > 0:
                dv.append((abs(v1 - v2) / v1, i))
            if a1 > 0:
                da.append((abs(a1 - a2) / a1, i))
            if f1 is not None and f2 is not None and f1 != f2:
                df.append(f"`{i}` {f1} vs {f2}")
    if dv and da:
        dv.sort()
        da.sort()
        print(
            f"\nWhere both pass ({len(dv)} cases): volume median {statistics.median(x for x, _ in dv):.1e}, "
            f"worst {dv[-1][0]:.1e} (`{dv[-1][1]}`); area median {statistics.median(x for x, _ in da):.1e}, "
            f"worst {da[-1][0]:.1e} (`{da[-1][1]}`); merged face counts differ on {len(df)}: {', '.join(df[:8])}"
        )


def tsv(path: Path) -> dict[str, dict[str, str]]:
    rows = [line.split("\t") for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    head = rows[0]
    return {r[0]: dict(zip(head, r)) for r in rows[1:]}


def nist(ntsv: Path, otsv: Path) -> None:
    nn, on = tsv(ntsv), tsv(otsv)
    print("| Check | Native pass / fail / error / n/a | OCCT pass / fail / error / n/a |")
    print("| --- | --- | --- |")
    for c in CHECKS:
        def tally(t: dict[str, dict[str, str]]) -> str:
            k = collections.Counter(r[c].lower() for r in t.values())
            return f"{k['pass']} / {k['fail']} / {k['error']} / {k['n/a']}"
        print(f"| `{c}` | {tally(nn)} | {tally(on)} |")
    for name, t in [("Native", nn), ("OCCT", on)]:
        st = collections.Counter(r["status"].lower() for r in t.values())
        imp = [float(r["import_s"]) for r in t.values()]
        tot = [float(r["total_s"]) for r in t.values()]
        det = sum(r["deterministic"] == "yes" for r in t.values())
        print(
            f"\n{name}: {st['pass']} pass, {st['fail']} fail, {st['error']} error; import median {statistics.median(imp):.2f} s, "
            f"p95 {pct(imp, .95):.2f} s; whole file median {statistics.median(tot):.1f} s, p95 {pct(tot, .95):.1f} s; "
            f"same hash on both runs {det}/{len(t)}"
        )
    agree = []
    for i in sorted(set(nn) & set(on)):
        try:
            v1, v2 = float(nn[i]["volume_mm3"]), float(on[i]["volume_mm3"])
        except ValueError:
            continue
        if v1 > 0 and v2 > 0:
            agree.append((abs(v1 - v2) / v1, i))
    if agree:
        agree.sort()
        print(f"\nReaders' volumes agree on {len(agree)} files: median {statistics.median(x for x, _ in agree):.1e}, worst {agree[-1][0]:.1e} (`{agree[-1][1]}`)")


def main(argv: list[str]) -> int:
    if len(argv) == 5 and argv[0] == "primitive":
        primitive(*map(Path, argv[1:]))
        return 0
    if len(argv) == 3 and argv[0] == "nist":
        nist(*map(Path, argv[1:]))
        return 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
