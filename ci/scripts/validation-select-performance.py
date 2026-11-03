#!/usr/bin/env python3
"""Measure a production-shaped compiler-free validation selection (`A1-651`)."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import platform
import stat
import subprocess
import sys
import time
from pathlib import Path
from typing import Any


TIMING_PREFIX = "validation-select: timings="
SCHEMA = "aplan/validation-selector-performance/v1"


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[max(0, math.ceil(fraction * len(ordered)) - 1)]


def sha256_regular(path: Path) -> str:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise RuntimeError(f"measurement input `{path.name}` is unavailable: {error.strerror}")
    if not stat.S_ISREG(metadata.st_mode):
        raise RuntimeError(f"measurement input `{path.name}` is not a regular file")
    return hashlib.sha256(path.read_bytes()).hexdigest()


def git_bytes(root: Path, *arguments: str) -> bytes:
    output = subprocess.run(
        ["git", *arguments],
        cwd=root,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if output.returncode != 0:
        raise RuntimeError(
            f"git {' '.join(arguments)} failed: "
            + output.stderr.decode("utf-8", errors="replace")[:4096]
        )
    return output.stdout


def git_text(root: Path, *arguments: str) -> str:
    try:
        return git_bytes(root, *arguments).decode("ascii").strip()
    except UnicodeDecodeError as error:
        raise RuntimeError(f"git {' '.join(arguments)} returned a non-ASCII identity") from error


def source_snapshot(root: Path) -> tuple[dict[str, Any], str]:
    """One stable, clean source identity and its immutable first parent."""

    commit = git_text(root, "rev-parse", "--verify", "HEAD^{commit}")
    try:
        base = git_text(root, "rev-parse", "--verify", f"{commit}^1")
    except RuntimeError as error:
        raise RuntimeError("the measured commit has no first parent") from error
    if git_bytes(root, "status", "--porcelain=v1", "-z", "--untracked-files=all"):
        raise RuntimeError("the measured checkout is not clean")
    source = {
        "commit": commit,
        "working_tree_clean": True,
        "harness_sha256": sha256_regular(
            root / "ci/scripts/validation-select-performance.py"
        ),
        "selector_sha256": sha256_regular(root / "ci/scripts/validation-select.py"),
        "policy_sha256": sha256_regular(root / "ci/validation-selection.json"),
    }
    # Bound races inside the snapshot itself: the commit and cleanliness must
    # still be the values under which the two content digests were read.
    if git_text(root, "rev-parse", "--verify", "HEAD^{commit}") != commit:
        raise RuntimeError("the measured checkout changed while its source identity was read")
    if git_bytes(root, "status", "--porcelain=v1", "-z", "--untracked-files=all"):
        raise RuntimeError("the measured checkout changed while its source identity was read")
    return source, base


def exact_nonnegative_integer(value: Any) -> bool:
    return type(value) is int and value >= 0


def finite_nonnegative_number(value: Any) -> bool:
    return type(value) in {int, float} and math.isfinite(value) and value >= 0


def sample(
    root: Path, base: str, head: str, timeout_seconds: float
) -> dict[str, Any]:
    selector = root / "ci/scripts/validation-select.py"
    started = time.perf_counter_ns()
    output = subprocess.run(
        [
            sys.executable,
            str(selector),
            "--root",
            str(root),
            "--revisions",
            base,
            head,
            "--format",
            "json",
            "--timings",
        ],
        cwd=root,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout_seconds,
        check=False,
    )
    wall_ms = (time.perf_counter_ns() - started) / 1_000_000
    if output.returncode != 0:
        raise RuntimeError(
            "selector failed: " + output.stderr.decode("utf-8", errors="replace")[:4096]
        )
    lines = output.stderr.decode("utf-8", errors="replace").splitlines()
    timing_lines = [line for line in lines if line.startswith(TIMING_PREFIX)]
    if len(timing_lines) != 1:
        raise RuntimeError(f"selector emitted {len(timing_lines)} timing records")
    try:
        timing = json.loads(timing_lines[0].removeprefix(TIMING_PREFIX))
    except json.JSONDecodeError as error:
        raise RuntimeError("selector emitted malformed timing JSON") from error
    expected = {
        "cargo_processes",
        "census_ms",
        "classification_ms",
        "git_ms",
        "git_processes",
        "inventory_ms",
        "metadata_ms",
        "output_ms",
        "python_startup_ms",
        "rustc_processes",
        "total_ms",
    }
    if not isinstance(timing, dict):
        raise RuntimeError("selector timing record is not a JSON object")
    if set(timing) != expected:
        raise RuntimeError(
            f"timing fields changed: missing={sorted(expected - set(timing))!r}, "
            f"unexpected={sorted(set(timing) - expected)!r}"
        )
    for field in ["cargo_processes", "git_processes", "rustc_processes"]:
        if not exact_nonnegative_integer(timing[field]):
            raise RuntimeError(f"selector timing `{field}` is not a non-negative integer")
    for field in expected - {"cargo_processes", "git_processes", "rustc_processes"}:
        if not finite_nonnegative_number(timing[field]):
            raise RuntimeError(f"selector timing `{field}` is not a finite non-negative number")
    if timing["cargo_processes"] != 0 or timing["rustc_processes"] != 0:
        raise RuntimeError(f"selector crossed the compiler-free boundary: {timing!r}")
    try:
        report = json.loads(output.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError("selector emitted malformed report JSON") from error
    if (
        not isinstance(report, dict)
        or report.get("schema") != 2
        or report.get("mode") != "revisions"
        or not isinstance(report.get("inputs"), dict)
        or report["inputs"].get("resolved_base") != base
        or report["inputs"].get("resolved_head") != head
    ):
        raise RuntimeError("selector report does not identify the measured immutable revisions")
    return {
        "wall_ms": round(wall_ms, 3),
        "stdout_bytes": len(output.stdout),
        "stdout_sha256": hashlib.sha256(output.stdout).hexdigest(),
        "timings": timing,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument("--runs", type=int, default=7)
    parser.add_argument("--timeout-seconds", type=float, default=10.0)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--max-cache-free-p95-ms", type=float, default=5000.0)
    parser.add_argument("--max-warm-p95-ms", type=float, default=1000.0)
    args = parser.parse_args()
    if args.runs < 5:
        parser.error("--runs must be at least 5")
    for label, value in [
        ("--timeout-seconds", args.timeout_seconds),
        ("--max-cache-free-p95-ms", args.max_cache_free_p95_ms),
        ("--max-warm-p95-ms", args.max_warm_p95_ms),
    ]:
        if not math.isfinite(value) or value <= 0:
            parser.error(f"{label} must be finite and positive")
    root = args.root.resolve()
    if not root.is_dir():
        parser.error("--root must be a directory")
    source, base = source_snapshot(root)
    samples = [
        sample(root, base, source["commit"], args.timeout_seconds)
        for _ in range(args.runs)
    ]
    final_source, final_base = source_snapshot(root)
    if final_source != source or final_base != base:
        raise RuntimeError("the measured source changed while samples were running")
    walls = [entry["wall_ms"] for entry in samples]
    warm = walls[1:]
    output_hashes = {entry["stdout_sha256"] for entry in samples}
    output_sizes = {entry["stdout_bytes"] for entry in samples}
    git_process_counts = {entry["timings"]["git_processes"] for entry in samples}
    if len(output_hashes) != 1 or len(output_sizes) != 1:
        raise RuntimeError("selector output changed between identical samples")
    if len(git_process_counts) != 1:
        raise RuntimeError("selector Git process count changed between identical samples")
    document = {
        "schema": SCHEMA,
        "source": source,
        "environment": {
            "os": platform.system(),
            "arch": platform.machine(),
            "python": platform.python_version(),
        },
        "deterministic": {
            "cargo_processes": max(
                entry["timings"]["cargo_processes"] for entry in samples
            ),
            "git_processes": git_process_counts.pop(),
            "rustc_processes": max(
                entry["timings"]["rustc_processes"] for entry in samples
            ),
            "stdout_bytes": output_sizes.pop(),
            "stdout_sha256": output_hashes.pop(),
        },
        "samples": samples,
        "summary": {
            "cold_ms": round(walls[0], 3),
            "cache_free_p50_ms": round(percentile(walls, 0.50), 3),
            "cache_free_p95_ms": round(percentile(walls, 0.95), 3),
            "cache_free_max_ms": round(max(walls), 3),
            "warm_p50_ms": round(percentile(warm, 0.50), 3),
            "warm_p95_ms": round(percentile(warm, 0.95), 3),
            "warm_max_ms": round(max(warm), 3),
        },
    }
    json.dump(document, sys.stdout, indent=2, sort_keys=True)
    sys.stdout.write("\n")
    if args.check and (
        document["summary"]["cache_free_p95_ms"] > args.max_cache_free_p95_ms
        or document["summary"]["warm_p95_ms"] > args.max_warm_p95_ms
    ):
        print(
            "validation-select-performance: latency objective missed: "
            f"cache-free p95={document['summary']['cache_free_p95_ms']}ms "
            f"warm p95={document['summary']['warm_p95_ms']}ms",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
        print(f"validation-select-performance: {error}", file=sys.stderr)
        raise SystemExit(1) from None
