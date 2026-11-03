#!/usr/bin/env python3
"""Reproducible end-to-end read-path benchmark for A1-588.

This is an observation tool, not a shared-runner latency gate. It measures the
real CLI process, including startup and output, and also consumes the CLI's
deterministic ``--timings`` counters. Run it for both development and release
binaries; A1-589 extends the state matrix with index-hit and one-edit cases.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import resource
import selectors
import signal
import shutil
import stat
import statistics
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable


TIMING_PREFIX = "aplan: timings "
TIMING_MS_FIELDS = (
    "total_ms",
    "config_ms",
    "load_ms",
    "validate_ms",
    "project_ms",
    "render_ms",
)
TIMING_WORK_FIELDS = (
    "config_loads",
    "plan_loads",
    "candidates",
    "readable",
    "bytes_read",
    "tasks_parsed",
    "retained_files",
    "retained_bytes",
    "graph_builds",
    "cache_hits",
    "cache_reused",
    "cache_parsed",
    "cache_bytes_read",
    "cache_bytes_written",
    "trace_reads",
    "trace_bytes_read",
    "markdown_parses",
    "structured_parses",
)


@dataclass(frozen=True)
class Sample:
    wall_ms: float
    cpu_ms: float
    first_byte_ms: float
    stdout_bytes: int
    stdout_sha256: str
    stdout_preview: bytes
    exit_code: int
    timings: dict[str, float | int]
    stderr_without_timings: bytes


@dataclass(frozen=True)
class SourceState:
    """The Git source state whose binary and plan are being measured."""

    revision: str
    sha256: str


def percentile(samples: list[float], fraction: float) -> float:
    """Nearest-rank percentile, deliberately simple and reproducible."""
    ordered = sorted(samples)
    return ordered[max(0, math.ceil(fraction * len(ordered)) - 1)]


def parse_timing_line(stderr: bytes) -> tuple[dict[str, float | int], bytes]:
    values: dict[str, float | int] = {}
    retained: list[bytes] = []
    diagnostics = 0
    for line in stderr.splitlines(keepends=True):
        text = line.decode("utf-8", errors="replace").rstrip("\r\n")
        if not text.startswith(TIMING_PREFIX):
            retained.append(line)
            continue
        diagnostics += 1
        for field in text.removeprefix(TIMING_PREFIX).split():
            key, raw = field.split("=", 1)
            if key in values:
                raise RuntimeError(f"timing diagnostic repeated field `{key}`")
            values[key] = float(raw) if key.endswith("_ms") else int(raw)
    if diagnostics != 1:
        raise RuntimeError(
            f"command emitted {diagnostics} aplan timing diagnostics rather than one"
        )
    expected = set(TIMING_MS_FIELDS + TIMING_WORK_FIELDS)
    if set(values) != expected:
        raise RuntimeError(
            "timing fields changed: "
            f"missing={sorted(expected - set(values))!r}, "
            f"unexpected={sorted(set(values) - expected)!r}"
        )
    if any(not math.isfinite(float(value)) or value < 0 for value in values.values()):
        raise RuntimeError(f"timing diagnostic contains an invalid value: {values!r}")
    return values, b"".join(retained)


def terminate_process_group(process: subprocess.Popen[bytes]) -> None:
    """Kill a timed-out sample and every descendant holding its pipes."""
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        # The process and its group have already left. ``wait`` still reaps the
        # direct child when it exited between the deadline and this call.
        pass
    process.wait()


def run_once(argv: list[str], cwd: Path, timeout_seconds: float) -> Sample:
    before = resource.getrusage(resource.RUSAGE_CHILDREN)
    started = time.perf_counter_ns()
    process = subprocess.Popen(
        argv,
        cwd=cwd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env={**os.environ, "NO_COLOR": "1"},
        start_new_session=True,
    )
    assert process.stdout is not None
    assert process.stderr is not None
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ, "stdout")
    selector.register(process.stderr, selectors.EVENT_READ, "stderr")
    output = bytearray()
    errors = bytearray()
    first_byte_ns: int | None = None
    deadline = time.monotonic() + timeout_seconds
    while selector.get_map():
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            terminate_process_group(process)
            raise RuntimeError(
                f"command exceeded {timeout_seconds:g}s: {' '.join(argv)}"
            )
        for key, _ in selector.select(timeout=remaining):
            chunk = os.read(key.fileobj.fileno(), 64 * 1024)
            if not chunk:
                selector.unregister(key.fileobj)
                continue
            if key.data == "stdout":
                if first_byte_ns is None:
                    first_byte_ns = time.perf_counter_ns()
                output.extend(chunk)
            else:
                errors.extend(chunk)
    try:
        exit_code = process.wait(timeout=max(0.001, deadline - time.monotonic()))
    except subprocess.TimeoutExpired as error:
        terminate_process_group(process)
        raise RuntimeError(
            f"command exceeded {timeout_seconds:g}s: {' '.join(argv)}"
        ) from error
    finished = time.perf_counter_ns()
    after = resource.getrusage(resource.RUSAGE_CHILDREN)
    timings, retained_stderr = parse_timing_line(bytes(errors))
    if first_byte_ns is None:
        first_byte_ns = finished
    cpu_seconds = (after.ru_utime - before.ru_utime) + (after.ru_stime - before.ru_stime)
    return Sample(
        wall_ms=(finished - started) / 1_000_000,
        cpu_ms=cpu_seconds * 1_000,
        first_byte_ms=(first_byte_ns - started) / 1_000_000,
        stdout_bytes=len(output),
        stdout_sha256=hashlib.sha256(output).hexdigest(),
        stdout_preview=bytes(output[:4096]),
        exit_code=exit_code,
        timings=timings,
        stderr_without_timings=retained_stderr,
    )


def stable_value(samples: Iterable[Sample], field: str) -> object:
    values = {getattr(sample, field) for sample in samples}
    if len(values) != 1:
        raise RuntimeError(f"{field} changed between benchmark samples: {values!r}")
    return values.pop()


def run_case(
    name: str,
    argv: list[str],
    cwd: Path,
    warmups: int,
    runs: int,
    timeout_seconds: float,
    one_edit: Path | None = None,
    clean_cache: bool = False,
) -> dict[str, object]:
    cache_root = Path(os.environ["APLAN_CACHE_DIR"])

    def reset_cache() -> None:
        shutil.rmtree(cache_root, ignore_errors=True)

    if clean_cache:
        for _ in range(warmups):
            reset_cache()
            run_once(argv, cwd, timeout_seconds)
        samples = []
        for _ in range(runs):
            reset_cache()
            samples.append(run_once(argv, cwd, timeout_seconds))
    elif one_edit is None:
        for _ in range(warmups):
            run_once(argv, cwd, timeout_seconds)
        samples = [run_once(argv, cwd, timeout_seconds) for _ in range(runs)]
    else:
        original = one_edit.read_bytes()
        changed = mutate_task_bytes(original)
        samples = []
        try:
            for _ in range(warmups):
                reset_cache()
                one_edit.write_bytes(original)
                run_once(argv, cwd, timeout_seconds)
            for _ in range(runs):
                reset_cache()
                one_edit.write_bytes(original)
                run_once(argv, cwd, timeout_seconds)
                one_edit.write_bytes(changed)
                samples.append(run_once(argv, cwd, timeout_seconds))
        finally:
            one_edit.write_bytes(original)
    varying_work = (
        {"cache_bytes_read", "cache_bytes_written"}
        if one_edit is not None or clean_cache
        else set()
    )
    timing_shapes = {
        tuple(
            sorted(
                (key, value)
                for key, value in sample.timings.items()
                if not key.endswith("_ms") and key not in varying_work
            )
        )
        for sample in samples
    }
    if len(timing_shapes) != 1:
        raise RuntimeError(f"deterministic work changed in `{name}`: {timing_shapes!r}")
    exit_code = stable_value(samples, "exit_code")
    if exit_code != 0:
        raise RuntimeError(
            f"`{name}` exited {exit_code}; refusing to benchmark an error path; "
            f"stdout={stable_value(samples, 'stdout_preview')!r}; "
            f"stderr={stable_value(samples, 'stderr_without_timings')!r}"
        )
    phase_p50_ms = {
        field.removesuffix("_ms"): statistics.median(
            float(sample.timings[field]) for sample in samples
        )
        for field in TIMING_MS_FIELDS
    }
    phase_p50_ms["unattributed"] = statistics.median(
        max(
            0.0,
            float(sample.timings["total_ms"])
            - sum(
                float(sample.timings[field])
                for field in TIMING_MS_FIELDS
                if field != "total_ms"
            ),
        )
        for sample in samples
    )
    work = dict(timing_shapes.pop())
    for field in varying_work:
        work[field] = int(statistics.median(int(sample.timings[field]) for sample in samples))
    return {
        "name": name,
        "argv": argv,
        "exit_code": exit_code,
        "stdout_bytes": stable_value(samples, "stdout_bytes"),
        "stdout_sha256": stable_value(samples, "stdout_sha256"),
        "stderr_sha256": hashlib.sha256(
            stable_value(samples, "stderr_without_timings")
        ).hexdigest(),
        "wall_min_ms": min(sample.wall_ms for sample in samples),
        "wall_p50_ms": statistics.median(sample.wall_ms for sample in samples),
        "wall_p95_ms": percentile([sample.wall_ms for sample in samples], 0.95),
        "wall_max_ms": max(sample.wall_ms for sample in samples),
        "cpu_p50_ms": statistics.median(sample.cpu_ms for sample in samples),
        "first_byte_p50_ms": statistics.median(sample.first_byte_ms for sample in samples),
        "first_byte_p95_ms": percentile([sample.first_byte_ms for sample in samples], 0.95),
        "phase_p50_ms": phase_p50_ms,
        "work": work,
    }


def mutate_task_bytes(source: bytes) -> bytes:
    marker = b"title: "
    start = source.find(marker)
    if start < 0:
        raise RuntimeError("one-edit task has no title field")
    value = start + len(marker)
    end = source.find(b"\n", value)
    if end < 0 or end == value:
        raise RuntimeError("one-edit task has no usable title field")
    changed = bytearray(source)
    changed[end - 1] = ord("X") if changed[end - 1] != ord("X") else ord("Y")
    return bytes(changed)


def write_generated_plan(root: Path, count: int) -> tuple[str, Path]:
    tasks = root / "aplan" / "tasks"
    tasks.mkdir(parents=True)
    width = max(5, len(str(count)))
    for number in range(1, count + 1):
        task_id = f"G-{number:0{width}d}"
        (tasks / f"{task_id}.md").write_text(
            "---\n"
            f"id: {task_id}\n"
            f"title: Generated task {number}\n"
            "phase: 1\n"
            "status: todo\n"
            "effort_days: 1\n"
            "---\n\n"
            "Generated by ci/scripts/read-performance.py.\n",
            encoding="utf-8",
        )
    first = f"G-{1:0{width}d}"
    return first, tasks / f"{first}.md"


def command(
    binary: Path,
    root: Path,
    words: list[str],
    no_config: bool = False,
    config_file: Path | None = None,
    cache_mode: str = "auto",
) -> list[str]:
    argv = [
        str(binary),
        "--timings",
        "--color",
        "never",
        "--cache",
        cache_mode,
    ]
    if no_config:
        argv.append("--no-config")
    elif config_file is not None:
        argv.extend(["--config", str(config_file)])
    argv.extend(["--root", str(root), *words])
    return argv


def update_file_digest(digest: object, path: Path) -> int:
    """Hash one path without opening a FIFO/device or retaining a large file."""
    metadata = path.lstat()
    digest.update(f"mode={metadata.st_mode:o}\0".encode("ascii"))
    if stat.S_ISLNK(metadata.st_mode):
        digest.update(b"link\0" + os.fsencode(os.readlink(path)))
        return 0
    if not stat.S_ISREG(metadata.st_mode):
        digest.update(b"special\0")
        return 0
    digest.update(b"file\0")
    size = 0
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
            size += len(chunk)
    return size


def file_identity(path: Path) -> tuple[int, str, int]:
    metadata = path.stat()
    if not stat.S_ISREG(metadata.st_mode):
        raise RuntimeError(f"benchmark binary is not a regular file: {path}")
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
            size += len(chunk)
    return size, digest.hexdigest(), metadata.st_mode


def git_source_state(cwd: Path) -> SourceState:
    """Fingerprint HEAD plus every tracked and untracked working-tree byte."""
    try:
        head = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=cwd, stderr=subprocess.DEVNULL
        ).strip()
        status = subprocess.check_output(
            ["git", "status", "--porcelain=v1", "-z", "--untracked-files=all"],
            cwd=cwd,
            stderr=subprocess.DEVNULL,
        )
        tracked_delta = subprocess.check_output(
            [
                "git",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--binary",
                "HEAD",
                "--",
            ],
            cwd=cwd,
            stderr=subprocess.DEVNULL,
        )
        untracked = subprocess.check_output(
            ["git", "ls-files", "--others", "--exclude-standard", "-z"],
            cwd=cwd,
            stderr=subprocess.DEVNULL,
        )
    except (OSError, subprocess.CalledProcessError):
        return SourceState("not-a-git-checkout", "unavailable")

    digest = hashlib.sha256()
    digest.update(b"HEAD\0" + head + b"\0STATUS\0" + status)
    digest.update(b"\0TRACKED\0" + tracked_delta + b"\0UNTRACKED\0")
    for encoded_path in untracked.split(b"\0"):
        if not encoded_path:
            continue
        digest.update(encoded_path + b"\0")
        path = cwd / os.fsdecode(encoded_path)
        try:
            update_file_digest(digest, path)
        except OSError as error:
            digest.update(
                b"error\0"
                + str(error.errno).encode("ascii", errors="replace")
                + b"\0"
                + os.fsencode(str(error))
            )
        digest.update(b"\0")
    revision = head.decode("ascii", errors="replace")
    if status:
        revision += "+dirty"
    return SourceState(revision, digest.hexdigest())


def render_markdown(report: dict[str, object]) -> str:
    lines = [
        f"# Read-path baseline: {report['profile']}",
        "",
        f"- Recorded: {report['recorded_at']}",
        f"- Host: `{report['host']}`",
        f"- Load average: `{report['load_average']}`",
        f"- Revision: `{report['revision']}`",
        f"- Source SHA-256: `{report['source_sha256']}`",
        f"- Binary: `{report['binary']}` ({report['binary_bytes']} bytes, SHA-256 `{report['binary_sha256']}`)",
        f"- Samples: {report['runs']} after {report['warmups']} warm-up run(s)",
        f"- Cache state: `{report['cache_state']}`",
        "",
        "Wall time and time to first byte include process startup. CPU is child user plus system time.",
        "Timing thresholds are observations on this host; deterministic work is the portable regression surface.",
        "",
        "| corpus / command | wall min | wall p50 | wall p95 | max | CPU p50 | TTFB p50 | TTFB p95 | bytes | exit |",
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for case in report["cases"]:
        assert isinstance(case, dict)
        lines.append(
            "| {name} | {wall_min_ms:.1f} ms | {wall_p50_ms:.1f} ms | {wall_p95_ms:.1f} ms | {wall_max_ms:.1f} ms | "
            "{cpu_p50_ms:.1f} ms | {first_byte_p50_ms:.1f} ms | {first_byte_p95_ms:.1f} ms | "
            "{stdout_bytes} | {exit_code} |".format(**case)
        )
    lines.extend(
        [
            "",
            "## CLI phase medians",
            "",
            "The CLI total starts before argument parsing. Unattributed time is total minus the five "
            "instrumented phases and includes orchestration and any work not yet assigned to a phase.",
            "",
            "| corpus / command | CLI total | config | load | validate | project | render | unattributed |",
            "|---|---:|---:|---:|---:|---:|---:|---:|",
        ]
    )
    for case in report["cases"]:
        assert isinstance(case, dict)
        phase = case["phase_p50_ms"]
        assert isinstance(phase, dict)
        lines.append(
            "| {name} | {total:.1f} ms | {config:.1f} ms | {load:.1f} ms | "
            "{validate:.1f} ms | {project:.1f} ms | {render:.1f} ms | "
            "{unattributed:.1f} ms |".format(name=case["name"], **phase)
        )
    lines.extend(
        [
            "",
            "## Deterministic work",
            "",
            "| corpus / command | configs | loads | candidates | read | bytes read | tasks parsed | retained files | retained bytes | graphs | cache hits | reused | parsed | cache read | cache written | trace reads | trace bytes | Markdown parses | structured parses |",
            "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|",
        ]
    )
    for case in report["cases"]:
        assert isinstance(case, dict)
        work = case["work"]
        assert isinstance(work, dict)
        lines.append(
            "| {name} | {config_loads} | {plan_loads} | {candidates} | {readable} | {bytes_read} | "
            "{tasks_parsed} | {retained_files} | {retained_bytes} | {graph_builds} | "
            "{cache_hits} | {cache_reused} | {cache_parsed} | {cache_bytes_read} | {cache_bytes_written} | "
            "{trace_reads} | {trace_bytes_read} | {markdown_parses} | {structured_parses} |".format(
                name=case["name"], **work
            )
        )
    lines.append("")
    return "\n".join(lines)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--root", type=Path, default=Path.cwd() / "aplan")
    parser.add_argument("--task-id", default="A1-588")
    parser.add_argument("--runs", type=int, default=15)
    parser.add_argument("--warmups", type=int, default=2)
    parser.add_argument("--generated-tasks", type=int, default=1000)
    parser.add_argument(
        "--cache-state", choices=("hit", "clean", "one-edit"), default="hit"
    )
    parser.add_argument("--timeout-seconds", type=float, default=30.0)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--json-output", type=Path)
    args = parser.parse_args()
    if (
        args.runs < 1
        or args.warmups < 0
        or args.generated_tasks < 0
        or not math.isfinite(args.timeout_seconds)
        or args.timeout_seconds <= 0
    ):
        parser.error(
            "runs and timeout-seconds must be positive; warmups and generated-tasks "
            "must be non-negative"
        )
    cwd = Path.cwd().resolve()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"binary does not exist: {binary}")
    binary_bytes, binary_sha256, binary_mode = file_identity(binary)
    source_state = git_source_state(cwd)

    with tempfile.TemporaryDirectory(prefix="aplan-read-performance-") as temporary:
        temporary_root = Path(temporary)
        os.environ["APLAN_CACHE_DIR"] = str(temporary_root / "cache")
        cache_mode = "auto"
        current_root = args.root.resolve()
        current_cwd = cwd
        current_edit: Path | None = None
        config_file: Path | None = None
        if args.cache_state == "one-edit":
            try:
                root_relative = current_root.relative_to(cwd)
            except ValueError:
                copied_root = temporary_root / "current-plan"
                shutil.copytree(current_root, copied_root, symlinks=True)
                current_root = copied_root
            else:
                copied_repo = temporary_root / "current-repository"
                shutil.copytree(
                    cwd,
                    copied_repo,
                    symlinks=True,
                    ignore=shutil.ignore_patterns(".git", "target", "__pycache__"),
                )
                # Structural policy discovers repository containment from this
                # marker; the benchmark never invokes Git or evidence providers.
                (copied_repo / ".git").mkdir()
                current_root = copied_repo / root_relative
                current_cwd = copied_repo
                discovered = copied_repo / ".aplan.yml"
                if discovered.is_file():
                    config_file = discovered
            matches = list(current_root.rglob(f"{args.task_id}.md"))
            if len(matches) != 1:
                raise RuntimeError(
                    f"one-edit state found {len(matches)} files for {args.task_id!r}"
                )
            current_edit = matches[0]

        cases: list[tuple[str, list[str], Path | None, Path]] = [
            (
                "current / ready",
                command(
                    binary,
                    current_root,
                    ["ready", "--format", "json"],
                    config_file=config_file,
                    cache_mode=cache_mode,
                ),
                current_edit,
                current_cwd,
            ),
            (
                "current / why",
                command(
                    binary,
                    current_root,
                    ["why", args.task_id, "--format", "json"],
                    config_file=config_file,
                    cache_mode=cache_mode,
                ),
                current_edit,
                current_cwd,
            ),
            (
                "current / show --no-resolve",
                command(
                    binary,
                    current_root,
                    ["show", args.task_id, "--no-resolve", "--format", "json"],
                    config_file=config_file,
                    cache_mode=cache_mode,
                ),
                current_edit,
                current_cwd,
            ),
            (
                "current / check --no-traces",
                command(
                    binary,
                    current_root,
                    ["check", "--no-traces", "--format", "json"],
                    config_file=config_file,
                    cache_mode=cache_mode,
                ),
                current_edit,
                current_cwd,
            ),
            (
                "current / check",
                command(
                    binary,
                    current_root,
                    ["check", "--format", "json"],
                    config_file=config_file,
                    cache_mode=cache_mode,
                ),
                current_edit,
                current_cwd,
            ),
        ]
        if args.generated_tasks:
            generated_root = temporary_root / "generated"
            generated_id, generated_edit = write_generated_plan(
                generated_root, args.generated_tasks
            )
            plan_root = generated_root / "aplan"
            edit = generated_edit if args.cache_state == "one-edit" else None
            cases.extend(
                [
                    (
                        f"generated-{args.generated_tasks} / why",
                        command(
                            binary,
                            plan_root,
                            ["why", generated_id, "--format", "json"],
                            no_config=True,
                            cache_mode=cache_mode,
                        ),
                        edit,
                        cwd,
                    ),
                    (
                        f"generated-{args.generated_tasks} / show --no-resolve",
                        command(
                            binary,
                            plan_root,
                            ["show", generated_id, "--no-resolve", "--format", "json"],
                            no_config=True,
                            cache_mode=cache_mode,
                        ),
                        edit,
                        cwd,
                    ),
                    (
                        f"generated-{args.generated_tasks} / check",
                        command(
                            binary,
                            plan_root,
                            ["check", "--format", "json"],
                            no_config=True,
                            cache_mode=cache_mode,
                        ),
                        edit,
                        cwd,
                    ),
                ]
            )
        warmups = max(1, args.warmups) if args.cache_state == "hit" else args.warmups
        measured = [
            run_case(
                name,
                argv,
                case_cwd,
                warmups,
                args.runs,
                args.timeout_seconds,
                one_edit,
                clean_cache=args.cache_state == "clean",
            )
            for name, argv, one_edit, case_cwd in cases
        ]

    if file_identity(binary) != (binary_bytes, binary_sha256, binary_mode):
        raise RuntimeError("benchmark binary changed while samples were running")
    final_source_state = git_source_state(cwd)
    if final_source_state != source_state:
        raise RuntimeError("Git source state changed while samples were running")
    report: dict[str, object] = {
        "profile": args.profile,
        "recorded_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "host": f"{platform.node()} / {platform.platform()} / {platform.machine()}",
        "load_average": " ".join(f"{value:.2f}" for value in os.getloadavg()),
        "revision": source_state.revision,
        "source_sha256": source_state.sha256,
        "binary": str(binary),
        "binary_bytes": binary_bytes,
        "binary_sha256": binary_sha256,
        "runs": args.runs,
        "warmups": warmups,
        "cache_state": args.cache_state,
        "timeout_seconds": args.timeout_seconds,
        "cases": measured,
    }
    markdown = render_markdown(report)
    if args.output:
        args.output.write_text(markdown, encoding="utf-8")
    else:
        sys.stdout.write(markdown)
    if args.json_output:
        args.json_output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
