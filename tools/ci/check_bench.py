#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-2.1-or-later
"""Check criterion results against the Stage 0 performance budgets.

Usage: check_bench.py CRITERION_DIR BUDGETS_JSON [--max-regression 0.10]

CRITERION_DIR is criterion's output folder (`target/criterion`). Each
benchmark `group/name` has `CRITERION_DIR/group_name/new/estimates.json`
(criterion 0.7 turns `/` in an id into `_` for the folder name);
when the run compared against a saved baseline (`-- --baseline base`) it
also has `change/estimates.json`, holding the relative change of the mean.

BUDGETS_JSON (`docs/design/performance-budgets.json`) maps each benchmark
id to `{"budget_ms": <ceiling>, "what": <why the number matters>}`.

A benchmark fails when
  1. its mean is above its budget (an absolute ceiling, set with headroom
     for slower CI runners), or
  2. it compared against a baseline and the mean grew by more than
     --max-regression (the master prompt: "a regression beyond 10 % fails
     the build"). Criterion's own noise threshold is not applied here: the
     job runs baseline and candidate on the same runner, back to back.
A budgeted benchmark with no result fails too (a renamed benchmark must
rename its budget). Standard library only; prints a Markdown table that the
CI job appends to the step summary.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


def mean_ns(path: Path) -> float | None:
    """The point estimate of the mean from a criterion estimates file."""
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return float(data["mean"]["point_estimate"])
    except (OSError, KeyError, ValueError, TypeError):
        return None


def main(argv: list[str]) -> int:
    if len(argv) not in (2, 4) or (len(argv) == 4 and argv[2] != "--max-regression"):
        print(__doc__, file=sys.stderr)
        return 2
    root, budgets_path = Path(argv[0]), Path(argv[1])
    max_regression = float(argv[3]) if len(argv) == 4 else 0.10
    budgets = json.loads(budgets_path.read_text(encoding="utf-8"))["benchmarks"]
    failures = 0
    print("| Benchmark | Mean | Budget | Change vs base | Result |")
    print("| --- | ---: | ---: | ---: | --- |")
    for bench, spec in sorted(budgets.items()):
        budget_ms = float(spec["budget_ms"])
        folder = root / bench.replace("/", "_")
        mean = mean_ns(folder / "new" / "estimates.json")
        if mean is None:
            print(f"| `{bench}` | — | {budget_ms:g} ms | — | FAIL: no result |")
            failures += 1
            continue
        change = mean_ns(folder / "change" / "estimates.json")
        problems = []
        if mean / 1e6 > budget_ms:
            problems.append("over budget")
        # The change file holds a relative change (0.05 = 5 % slower), not
        # nanoseconds, despite sharing the estimates layout.
        if change is not None and change > max_regression:
            problems.append(f"regressed more than {max_regression:.0%}")
        failures += bool(problems)
        change_text = "—" if change is None else f"{change:+.1%}"
        result = "pass" if not problems else "FAIL: " + ", ".join(problems)
        print(f"| `{bench}` | {mean / 1e6:.3f} ms | {budget_ms:g} ms | {change_text} | {result} |")
    print()
    print(f"{failures} benchmark(s) failed." if failures else "All budgets hold.")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
