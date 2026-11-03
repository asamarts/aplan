#!/usr/bin/env python3
"""Refuse private publication material in any object reachable from public refs."""

from __future__ import annotations

from collections import defaultdict
from pathlib import Path
import subprocess
import sys


FORBIDDEN_PATHS = {
    "ADOPTION-ANALYSIS.md",
    "FIELD-RESEARCH.md",
    "RESEARCH.md",
    "docs/design/evolution.md",
}
FORBIDDEN_PREFIXES = ("docs/evaluations/", "docs/internal/", "docs/pilots/")

# Split literals so the auditor's own source does not contain what it rejects.
FORBIDDEN_CONTENT = {
    "ecosystem-positioning": b"Position in the " + b"ecosystem",
    "competitive-claim": b"competitive " + b"claims",
    "head-on-positioning": b"Compete " + b"head-on",
    "market-audience-analysis": b"under-served " + b"audience",
    "competitor-absence-claim": b"no competitor in the plan-tool " + b"space",
}

CONTENT_ALLOWLIST: dict[str, set[str]] = {}
MAX_BLOB_BYTES = 64 * 1024 * 1024


class AuditError(RuntimeError):
    pass


def git(repo: Path, *args: str, input_bytes: bytes | None = None) -> bytes:
    result = subprocess.run(
        ["git", *args],
        cwd=repo,
        input=input_bytes,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        detail = result.stderr.decode("utf-8", "replace").strip()
        raise AuditError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout


def reachable_objects(repo: Path) -> dict[str, set[str]]:
    objects: dict[str, set[str]] = defaultdict(set)
    for raw_line in git(repo, "rev-list", "--objects", "--all").splitlines():
        object_id, separator, raw_path = raw_line.partition(b" ")
        try:
            oid = object_id.decode("ascii")
            path = raw_path.decode("utf-8") if separator else ""
        except UnicodeDecodeError as error:
            raise AuditError(f"non-UTF-8 object inventory entry: {error}") from error
        if path:
            objects[oid].add(path)
        else:
            objects.setdefault(oid, set())
    return objects


def historical_paths(repo: Path) -> set[str]:
    raw = git(repo, "log", "--all", "--format=", "--name-only", "-z")
    try:
        return {item.decode("utf-8") for item in raw.split(b"\0") if item}
    except UnicodeDecodeError as error:
        raise AuditError(f"non-UTF-8 historical path: {error}") from error


def blobs(repo: Path, object_ids: list[str]):
    process = subprocess.Popen(
        ["git", "cat-file", "--batch"],
        cwd=repo,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    assert process.stdin is not None
    assert process.stdout is not None
    for requested in object_ids:
        process.stdin.write(requested.encode("ascii") + b"\n")
        process.stdin.flush()
        header = process.stdout.readline().rstrip(b"\n").split()
        if len(header) != 3:
            raise AuditError(f"malformed cat-file header for {requested}: {header!r}")
        returned, object_type, raw_size = header
        size = int(raw_size)
        if object_type == b"blob" and size > MAX_BLOB_BYTES:
            raise AuditError(
                f"reachable blob {requested[:12]} exceeds the 64 MiB history audit limit"
            )
        data = process.stdout.read(size)
        if process.stdout.read(1) != b"\n" or len(data) != size:
            raise AuditError(f"truncated cat-file payload for {requested}")
        if returned.decode("ascii") != requested:
            raise AuditError(f"cat-file returned the wrong object for {requested}")
        if object_type == b"blob":
            yield requested, data
    process.stdin.close()
    stderr = process.stderr.read() if process.stderr is not None else b""
    if process.wait() != 0:
        raise AuditError(f"git cat-file failed: {stderr.decode('utf-8', 'replace')}")


def audit(repo: Path) -> None:
    objects = reachable_objects(repo)
    findings: list[str] = []
    for path in sorted(historical_paths(repo)):
        if path in FORBIDDEN_PATHS or path.startswith(FORBIDDEN_PREFIXES):
            findings.append(f"forbidden historical path {path}")
    for object_id, paths in objects.items():
        for path in sorted(paths):
            if path in FORBIDDEN_PATHS or path.startswith(FORBIDDEN_PREFIXES):
                findings.append(f"forbidden historical path {path} in {object_id[:12]}")

    for object_id, data in blobs(repo, sorted(objects)):
        paths = objects[object_id]
        for rule, needle in FORBIDDEN_CONTENT.items():
            if needle not in data:
                continue
            allowed = CONTENT_ALLOWLIST.get(rule, set())
            if paths and paths.issubset(allowed):
                continue
            shown = ", ".join(sorted(paths)) if paths else "(path unavailable)"
            findings.append(f"forbidden historical content {rule} in {object_id[:12]}: {shown}")

    if findings:
        detail = "\n".join(f"- {finding}" for finding in sorted(set(findings)))
        raise AuditError(f"public history privacy audit failed:\n{detail}")


def main() -> int:
    repo = Path(sys.argv[1] if len(sys.argv) > 1 else ".").resolve()
    try:
        audit(repo)
    except AuditError as error:
        print(error, file=sys.stderr)
        return 1
    print("public history privacy audit passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
