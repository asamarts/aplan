#!/usr/bin/env python3
"""Write one bounded, host-neutral validation profile receipt.

The shell driver owns execution and passes only already-normalized profile data.
This helper owns JSON escaping, phase disposition and atomic publication. It
never reads command output or the environment wholesale, so a receipt cannot
accidentally become a secret or absolute-path dump.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import secrets
import stat
import sys
from pathlib import Path


SHA256 = re.compile(r"[0-9a-f]{64}")
COMMIT = re.compile(r"[0-9a-f]{40}")
NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}")
TOOLCHAIN = re.compile(
    r"toolchain: channel=(?P<channel>[0-9]+\.[0-9]+\.[0-9]+) "
    r"host=(?P<host>[A-Za-z0-9][A-Za-z0-9_.-]{0,127}) "
    r"commit=(?P<commit>[0-9a-f]{40}) components=(?P<components>[^ ]+) "
    r"cache-key=(?P<cache_key>[A-Za-z0-9][A-Za-z0-9_.-]{0,255})"
)
COMPONENTS = re.compile(r"[a-z0-9][a-z0-9_-]*(?:,[a-z0-9][a-z0-9_-]*)*")
PHASE_GATES = {
    "compile": ("clippy", "doc", "msrv-build"),
    "test": ("test",),
    "provider": ("evidence",),
    "coverage": ("coverage",),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--profile", choices=("development", "full", "release"), required=True)
    parser.add_argument("--assurance", choices=("selected", "full", "release"), required=True)
    parser.add_argument("--outcome", choices=("passed", "failed"), required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--dirty", choices=("true", "false"), required=True)
    parser.add_argument("--inventory-sha256", required=True)
    parser.add_argument("--toolchain", required=True)
    parser.add_argument(
        "--input-kind",
        choices=("working-tree", "base-head", "pre-push", "direct"),
        default="direct",
    )
    parser.add_argument("--base")
    parser.add_argument("--head")
    parser.add_argument("--runner-environment", required=True)
    parser.add_argument("--runner-os", required=True)
    parser.add_argument("--runner-arch", required=True)
    parser.add_argument("--queue-seconds", type=bounded_seconds)
    parser.add_argument("--setup-seconds", type=bounded_seconds)
    parser.add_argument("--selected", action="append", default=[])
    parser.add_argument("--widened", action="append", default=[])
    parser.add_argument("--deferred", action="append", default=[])
    parser.add_argument("--package", action="append", default=[])
    parser.add_argument("--widening-reason", action="append", default=[])
    parser.add_argument("--gate", nargs=3, action="append", default=[], metavar=("NAME", "OUTCOME", "SECONDS"))
    parser.add_argument("--output")
    parser.add_argument("--summary")
    parser.add_argument("--release-version")
    parser.add_argument("--release-tag")
    return parser.parse_args()


def bounded_seconds(value: str) -> int:
    try:
        parsed = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("duration must be an integer") from error
    if not 0 <= parsed <= 86_400:
        raise argparse.ArgumentTypeError("duration must be between 0 and 86400 seconds")
    return parsed


def checked_name(value: str, field: str) -> str:
    if not NAME.fullmatch(value):
        raise ValueError(f"{field} has an unsafe or overlong value")
    return value


def checked_text(value: str, field: str, limit: int = 512) -> str:
    if not value or len(value.encode("utf-8")) > limit:
        raise ValueError(f"{field} is empty or exceeds {limit} bytes")
    if any(ord(character) < 0x20 or ord(character) == 0x7F for character in value):
        raise ValueError(f"{field} contains a control character")
    if os.path.isabs(value):
        raise ValueError(f"{field} contains an absolute host path")
    return value


def disposition(gates: tuple[str, ...], results: dict[str, dict[str, object]], deferred: set[str]) -> dict[str, object]:
    observed = [results[gate] for gate in gates if gate in results]
    if observed:
        return {
            "state": "observed",
            "duration_ms": sum(int(result["duration_ms"]) for result in observed),
            "source": "gate" if len(gates) == 1 else "gate_aggregate",
        }
    if any(gate in deferred for gate in gates):
        return {"state": "deferred", "duration_ms": None, "source": None}
    return {"state": "not_selected", "duration_ms": None, "source": None}


def external_phase(seconds: int | None) -> dict[str, object]:
    if seconds is None:
        return {"state": "unavailable", "duration_ms": None, "source": None}
    return {"state": "observed", "duration_ms": seconds * 1000, "source": "caller"}


def receipt(args: argparse.Namespace) -> dict[str, object]:
    if not COMMIT.fullmatch(args.source):
        raise ValueError("source must be one full lowercase commit ID")
    if not SHA256.fullmatch(args.inventory_sha256):
        raise ValueError("inventory digest must be one lowercase SHA-256")
    toolchain_match = TOOLCHAIN.fullmatch(args.toolchain)
    if toolchain_match is None:
        raise ValueError("toolchain identity does not match the verified contract shape")
    if COMPONENTS.fullmatch(toolchain_match.group("components")) is None:
        raise ValueError("toolchain components do not match the verified contract shape")
    for revision, field in ((args.base, "base"), (args.head, "head")):
        if revision is not None and not COMMIT.fullmatch(revision):
            raise ValueError(f"{field} must be one full lowercase commit ID")

    selected = [checked_name(value, "selected gate") for value in args.selected]
    widened = [checked_name(value, "widened gate") for value in args.widened]
    deferred = [checked_name(value, "deferred gate") for value in args.deferred]
    packages = [checked_name(value, "package") for value in args.package]
    reasons = [checked_text(value, "widening reason") for value in args.widening_reason]
    for values, field in (
        (selected, "selected gates"),
        (widened, "widened gates"),
        (deferred, "deferred gates"),
        (packages, "packages"),
    ):
        if len(values) != len(set(values)):
            raise ValueError(f"{field} contain a duplicate")

    results: dict[str, dict[str, object]] = {}
    ordered_results = []
    for gate, outcome, raw_seconds in args.gate:
        gate = checked_name(gate, "gate result")
        if gate in results:
            raise ValueError(f"gate result `{gate}` appears twice")
        if outcome not in ("passed", "failed"):
            raise ValueError(f"gate result `{gate}` has an unknown outcome")
        seconds = bounded_seconds(raw_seconds)
        result = {"name": gate, "outcome": outcome, "duration_ms": seconds * 1000}
        results[gate] = result
        ordered_results.append(result)

    selected_or_widened = set(selected) | set(widened)
    if set(selected) & set(widened):
        raise ValueError("a gate cannot be both selected and widened")
    if any(gate not in selected_or_widened for gate in results):
        raise ValueError("a gate result was not selected or widened")
    if args.outcome == "passed" and set(results) != selected_or_widened:
        raise ValueError("a passing receipt must contain every selected and widened gate")
    if set(deferred) & selected_or_widened:
        raise ValueError("a gate cannot be both executed and deferred")
    if args.input_kind == "base-head" and (args.base is None or args.head is None):
        raise ValueError("base-head input requires both source revisions")
    if args.input_kind == "working-tree" and (args.base is not None or args.head is None):
        raise ValueError("working-tree input requires only the head revision")
    if args.input_kind in ("direct", "pre-push") and (
        args.base is not None or args.head is not None
    ):
        raise ValueError(f"{args.input_kind} input does not accept source revisions")

    phases = {
        "queue": external_phase(args.queue_seconds),
        "setup": external_phase(args.setup_seconds),
    }
    for name, gates in PHASE_GATES.items():
        phases[name] = disposition(gates, results, set(deferred))

    runner = {
        "environment": checked_name(args.runner_environment, "runner environment"),
        "os": checked_name(args.runner_os, "runner OS"),
        "arch": checked_name(args.runner_arch, "runner architecture"),
    }
    document: dict[str, object] = {
        "schema": "aplan/validation-receipt/v1",
        "profile": args.profile,
        "assurance": args.assurance,
        "outcome": args.outcome,
        "source": {
            "commit": args.source,
            "dirty": args.dirty == "true",
            "input": args.input_kind,
            "base": args.base,
            "head": args.head,
        },
        "inventory": {"sha256": args.inventory_sha256},
        "toolchain": toolchain_match.groupdict(),
        "runner": runner,
        "selection": {
            "selected_gates": selected,
            "widened_gates": widened,
            "deferred_gates": deferred,
            "packages": packages,
            "widening_reasons": reasons,
        },
        "phases": phases,
        "gates": ordered_results,
    }
    if args.profile == "release":
        if args.release_version is None or args.release_tag is None:
            raise ValueError("release receipt requires version and tag")
        document["release"] = {
            "version": checked_text(args.release_version, "release version", 128),
            "tag": checked_text(args.release_tag, "release tag", 128),
        }
    elif args.release_version is not None or args.release_tag is not None:
        raise ValueError("only a release receipt may carry release identity")
    return document


def publish(path_text: str, payload: bytes) -> None:
    path = Path(path_text)
    if path.exists() or path.is_symlink():
        raise ValueError("receipt output already exists")
    parent = path.parent
    if (
        not parent.is_dir()
        or parent.is_symlink()
        or parent.resolve() != Path(os.path.abspath(parent))
    ):
        raise ValueError("receipt output parent must be a real directory")
    temporary = parent / f".{path.name}.tmp-{os.getpid()}-{secrets.token_hex(8)}"
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(payload)
            stream.flush()
            os.fsync(stream.fileno())
        os.link(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)
    mode = stat.S_IMODE(path.stat().st_mode)
    if mode != 0o600:
        raise ValueError(f"receipt output has unexpected mode {mode:o}")


def append_summary(path_text: str, document: dict[str, object], canonical: str) -> None:
    path = Path(path_text)
    parent = path.parent
    if (
        path.is_symlink()
        or not parent.is_dir()
        or parent.is_symlink()
        or parent.resolve() != Path(os.path.abspath(parent))
    ):
        raise ValueError("summary path and parent must not be symlinks")
    phases = document["phases"]
    with path.open("a", encoding="utf-8") as stream:
        stream.write("### aplan validation receipt\n\n")
        stream.write(f"- Profile: `{document['profile']}` ({document['assurance']})\n")
        stream.write(f"- Outcome: {document['outcome']}\n")
        stream.write(f"- Source: `{document['source']['commit']}`\n")
        stream.write(f"- Gate inventory: `sha256:{document['inventory']['sha256']}`\n")
        stream.write(
            "- Runner: `{} / {} / {}`\n".format(
                document["runner"]["environment"],
                document["runner"]["os"],
                document["runner"]["arch"],
            )
        )
        stream.write("\n| Phase | State | Duration (milliseconds) |\n")
        stream.write("|---|---:|---:|\n")
        for name in ("queue", "setup", "compile", "test", "provider", "coverage"):
            phase = phases[name]
            duration = phase["duration_ms"] if phase["duration_ms"] is not None else "-"
            stream.write(f"| `{name}` | {phase['state']} | {duration} |\n")
        stream.write("\n<details><summary>Machine receipt</summary>\n\n")
        stream.write("```json\n")
        stream.write(canonical)
        stream.write("\n```\n\n</details>\n\n")


def main() -> int:
    try:
        args = parse_args()
        document = receipt(args)
        canonical = json.dumps(document, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
        payload = (canonical + "\n").encode("utf-8")
        if len(payload) > 32_768:
            raise ValueError("receipt exceeds the 32768-byte bound")
        if args.output is not None:
            publish(args.output, payload)
        if args.summary is not None:
            append_summary(args.summary, document, canonical)
        print(f"validation: receipt={canonical}")
        return 0
    except (OSError, ValueError) as error:
        print(f"validation receipt: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
