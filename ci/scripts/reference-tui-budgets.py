#!/usr/bin/env python3
"""Turn reference runs of the TUI's wall-clock gates into budgets (A2-164, ADR-0067).

    ci/scripts/reference-tui-budgets.py [--rust] RESULT...

Each RESULT is a file that ``ci/scripts/reference-tui-perf.sh --budgets measure``
wrote. For every wall-clock metric this prints the p95 of each run, how far the
runs spread, and the budget: the worst p95 times 1.25, rounded up to the
nanosecond, and at least the worst p95 plus 0.1 ms, so that a metric of a few
nanoseconds is not failed by an unrelated change in code layout. The runs must
measure one source (the crates, the workspace manifest and the lockfile,
whatever commit carried them), built with one compiler, pinned to one CPU, and
measured at one load limit.

A metric agrees when it is in every run and its p95 spreads across them by at
most 5% of the smallest, or by at most 0.1 ms for a metric whose worst p95 is
under 2 ms. One that does not agree gets no reference budget and keeps the
hosted rule on the reference host too, as the maintainer decided for the first
measurement (A2-164). So does every metric ``HOSTED_RULE_BY_DECISION`` matches,
whatever its spread. The exit status is 0 when every other metric agrees and 1
otherwise, so that a re-basing that leaves metrics on the hosted rule is a
decision someone makes. ``--rust`` prints, for each gate, the
``REFERENCE_BUDGETS_NS`` table and the ``HOSTED_RULE_ON_REFERENCE`` list, unless
a metric is missing from a run.
"""

from __future__ import annotations

import argparse
import fnmatch
import sys
from pathlib import Path

SMALL_NS = 2_000_000
SMALL_SPREAD_NS = 100_000
SPREAD = 0.05
FLOOR_NS = 100_000
# Metrics the reference host holds to the hosted rule by the maintainer's decision, whatever their
# spread. The real terminal's first feedback, about 7 ms and the slowest of three launches, failed
# a passing revision once in three reference runs (A2-164, 2026-11-19); A2-180 makes it measurable.
HOSTED_RULE_BY_DECISION = ("pty_*_feedback",)
# The real-terminal gate's metrics carry this prefix; the rest are in-process.
TERMINAL_PREFIX = "pty_"


def fields(line: str) -> dict[str, str]:
    return dict(word.split("=", 1) for word in line.split()[1:] if "=" in word)


def read_run(path: Path) -> tuple[dict[str, str], dict[str, int]]:
    """The run's setting (source, compiler, CPU, load limit) and each metric's p95 in ns."""
    setting: dict[str, str] = {}
    metrics: dict[str, int] = {}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("REFERENCE commit="):
            setting = fields(line)
            setting.pop("budgets", None)
            setting.pop("commit", None)
            if "source" not in setting:
                raise SystemExit(f"{path}: the run does not name its source")
        elif line.startswith("TUI_PERF "):
            values = fields(line)
            if "p95_ns" in values:
                if values["metric"] in metrics:
                    raise SystemExit(f"{path}: {values['metric']} appears twice")
                metrics[values["metric"]] = int(values["p95_ns"])
    if not setting:
        raise SystemExit(f"{path}: no REFERENCE line, so not a reference run")
    return setting, metrics


def budget(worst: int) -> int:
    return max(-(-worst * 5 // 4), worst + FLOOR_NS)


def agrees(values: list[int]) -> tuple[bool, str]:
    low, high = min(values), max(values)
    spread = high - low
    if high < SMALL_NS:
        return spread <= SMALL_SPREAD_NS, f"{spread / 1e6:.3f} ms"
    return spread <= SPREAD * low, f"{100 * spread / low:.1f}%"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--rust", action="store_true", help="print the budget tables")
    parser.add_argument("results", nargs="+", type=Path)
    arguments = parser.parse_args()

    runs = [read_run(path) for path in arguments.results]
    settings = {tuple(sorted(setting.items())) for setting, _ in runs}
    if len(settings) != 1:
        for path, (setting, _) in zip(arguments.results, runs):
            print(f"{path}: {setting}", file=sys.stderr)
        raise SystemExit("the runs differ in source, compiler, CPU or load limit")

    names = list(dict.fromkeys(name for _, metrics in runs for name in metrics))
    agreed = complete = True
    budgets: dict[str, int] = {}
    wide: list[str] = []
    print(f"{len(runs)} runs of {dict(runs[0][0])}")
    for name in names:
        values = [metrics.get(name) for _, metrics in runs]
        if None in values:
            print(f"{name}: missing from run {values.index(None) + 1}")
            agreed = complete = False
            continue
        present = [value for value in values if value is not None]
        ok, spread = agrees(present)
        shown = " ".join(f"{value / 1e6:.3f}" for value in present)
        decided = any(fnmatch.fnmatchcase(name, pattern) for pattern in HOSTED_RULE_BY_DECISION)
        if decided:
            wide.append(name)
            verdict = "the hosted rule, by decision"
        elif ok:
            budgets[name] = budget(max(present))
            verdict = f"budget {budgets[name] / 1e6:.3f} ms"
        else:
            agreed = False
            wide.append(name)
            verdict = "the hosted rule"
        print(f"{'ok  ' if ok else 'WIDE'} {name}: p95 ms {shown}; spread {spread}; {verdict}")
    if agreed:
        print("the runs agree")
    elif complete:
        print("the runs do not agree on some metrics, which keep the hosted rule")
    else:
        print("the runs do not agree: a metric is missing")
    if complete and arguments.rust:
        for title, terminal in (
            ("crates/aplan-tui/tests/support/performance.rs", False),
            ("crates/aplan/tests/tui_performance.rs", True),
        ):
            print(f"\n// {title}")
            print("const REFERENCE_BUDGETS_NS: &[(&str, u64)] = &[")
            for name, value in budgets.items():
                if name.startswith(TERMINAL_PREFIX) == terminal:
                    print(f'    ("{name}", {value:_}),')
            print("];")
            print("const HOSTED_RULE_ON_REFERENCE: &[&str] = &[")
            for name in wide:
                if name.startswith(TERMINAL_PREFIX) == terminal:
                    print(f'    "{name}",')
            print("];")
    return 0 if agreed else 1


if __name__ == "__main__":
    sys.exit(main())
