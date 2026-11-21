#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
"""Compare kernel bake-off hashes from every CI target with a reference target.

Usage: compare_hashes.py KNOWN_LIST REFERENCE_TARGET DIR...

Each DIR is an artifact folder named `bakeoff-hashes-<target>` holding
`bakeoff-hashes.txt` (lines: `<case id> <status> <hash>`). Differences
listed in KNOWN_LIST (`<case id> <target>`) are reported as warnings;
any other difference, a missing case or a missing target fails the job.
A listed difference that no longer occurs is reported so the entry can be
removed. Standard library only.
"""

from __future__ import annotations

import sys
from pathlib import Path


def load(path: Path) -> dict[str, tuple[str, str]]:
    out = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        parts = line.split()
        if len(parts) == 3:
            out[parts[0]] = (parts[1], parts[2])
    return out


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    known: set[tuple[str, str]] = set()
    for line in Path(argv[0]).read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            case, target = line.split()[:2]
            known.add((case, target))
    reference = argv[1]
    dirs = {Path(d).name.removeprefix("bakeoff-hashes-"): Path(d) for d in argv[2:]}
    if reference not in dirs:
        print(f"::error::no hashes from the reference target {reference}")
        return 1
    ref = load(dirs[reference] / "bakeoff-hashes.txt")
    failures = 0
    seen_known: set[tuple[str, str]] = set()
    for target, folder in sorted(dirs.items()):
        if target == reference:
            continue
        got = load(folder / "bakeoff-hashes.txt")
        for case in sorted(set(ref) | set(got)):
            a, b = ref.get(case), got.get(case)
            if a == b:
                continue
            if (case, target) in known:
                seen_known.add((case, target))
                print(f"::warning::known cross-OS difference: {case} on {target}: {b} vs {a} on {reference}")
                continue
            failures += 1
            print(f"::error::{case} on {target}: {b} vs {a} on {reference}")
        print(f"{target}: {len(got)} cases compared with {reference}")
    for case, target in sorted(known - seen_known):
        if target in dirs:
            print(f"::warning::{case} on {target} now matches {reference}: remove it from the known list")
    print(f"{failures} unexpected difference(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
