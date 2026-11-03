#!/usr/bin/env python3
"""Compare Cargo and nextest's non-doctest inventories and print one identity."""

from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import subprocess
from typing import NoReturn


def fail(message: str) -> NoReturn:
    raise SystemExit(f"nextest inventory: {message}")


def read_nextest(path: pathlib.Path) -> tuple[set[str], dict[str, dict[str, object]]]:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
        suites = document["rust-suites"]
        declared_count = document["test-count"]
    except (OSError, UnicodeError, json.JSONDecodeError, KeyError, TypeError) as error:
        fail(f"cannot read {path}: {error}")
    if (
        not isinstance(suites, dict)
        or not isinstance(declared_count, int)
        or isinstance(declared_count, bool)
    ):
        fail("nextest JSON has invalid rust-suites or test-count fields")

    inventory: set[str] = set()
    by_target: dict[str, dict[str, object]] = {}
    for binary_id, suite in suites.items():
        if not isinstance(binary_id, str) or not binary_id or not isinstance(suite, dict):
            fail("nextest JSON has an invalid suite")
        package_id = suite.get("package-id")
        kind = suite.get("kind")
        binary_name = suite.get("binary-name")
        testcases = suite.get("testcases")
        cwd = suite.get("cwd")
        if not all(
            isinstance(value, str) and value
            for value in (package_id, kind, binary_name, cwd)
        ):
            fail(f"nextest suite {binary_id!r} has invalid identity or cwd")
        key = "\t".join((package_id, kind, binary_name))
        if key in by_target:
            fail(f"nextest target identity is duplicated: {key}")
        if not isinstance(testcases, dict):
            fail(f"nextest suite {binary_id!r} has invalid testcases")
        by_target[key] = {"binary-id": binary_id, "cwd": cwd}
        for name, testcase in testcases.items():
            if not isinstance(name, str) or not isinstance(testcase, dict):
                fail(f"nextest suite {binary_id!r} has an invalid testcase")
            kind = testcase.get("kind")
            ignored = testcase.get("ignored")
            if kind not in ("test", "benchmark") or not isinstance(ignored, bool):
                fail(f"nextest testcase {binary_id}::{name} has invalid metadata")
            record = f"{binary_id}\t{name}\t{kind}\t{str(ignored).lower()}"
            if record in inventory:
                fail(f"nextest testcase is duplicated: {record}")
            inventory.add(record)
    if len(inventory) != declared_count:
        fail(
            f"nextest declared {declared_count} tests but listed {len(inventory)}"
        )
    return inventory, by_target


def cargo_artifacts(path: pathlib.Path) -> list[dict[str, object]]:
    artifacts: list[dict[str, object]] = []
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as error:
        fail(f"cannot read {path}: {error}")
    for number, line in enumerate(lines, 1):
        try:
            message = json.loads(line)
        except json.JSONDecodeError as error:
            fail(f"Cargo JSON line {number} is malformed: {error}")
        if not isinstance(message, dict):
            fail(f"Cargo JSON line {number} is not an object")
        profile = message.get("profile")
        if (
            message.get("reason") == "compiler-artifact"
            and isinstance(profile, dict)
            and profile.get("test") is True
            and message.get("executable") is not None
        ):
            artifacts.append(message)
    if not artifacts:
        fail("Cargo reported no test executables")
    return artifacts


def listed(binary: str, cwd: str, ignored_only: bool) -> dict[str, str]:
    arguments = [binary]
    if ignored_only:
        arguments.append("--ignored")
    arguments.extend(("--list", "--format", "terse"))
    result = subprocess.run(
        arguments,
        cwd=cwd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        fail(
            f"{binary} could not list tests (status {result.returncode}):\n"
            + result.stderr.decode("utf-8", "replace")
        )
    entries: dict[str, str] = {}
    try:
        lines = result.stdout.decode("utf-8", "strict").splitlines()
    except UnicodeDecodeError as error:
        fail(f"{binary} emitted a non-UTF-8 libtest listing: {error}")
    for raw in lines:
        try:
            name, kind = raw.rsplit(": ", 1)
        except ValueError:
            fail(f"{binary} emitted an invalid libtest listing line: {raw!r}")
        if kind not in ("test", "benchmark") or not name:
            fail(f"{binary} emitted an invalid libtest entry: {raw!r}")
        if name in entries:
            fail(f"{binary} listed {name!r} more than once")
        entries[name] = kind
    return entries


def read_cargo(
    path: pathlib.Path, targets: dict[str, dict[str, object]]
) -> set[str]:
    inventory: set[str] = set()
    seen_targets: set[str] = set()
    for artifact in cargo_artifacts(path):
        target = artifact.get("target")
        if not isinstance(target, dict):
            fail("Cargo artifact has no target object")
        kinds = target.get("kind")
        if not isinstance(kinds, list) or len(kinds) != 1:
            fail("Cargo test target must have exactly one kind")
        kind = kinds[0]
        name = target.get("name")
        package_id = artifact.get("package_id")
        if kind not in ("lib", "bin", "test", "bench", "example"):
            fail(f"Cargo reported unsupported test target kind {kind!r}")
        if (
            not isinstance(name, str)
            or not name
            or not isinstance(package_id, str)
            or not package_id
        ):
            fail("Cargo test target has an invalid package or target name")
        key = "\t".join((package_id, kind, name))
        suite = targets.get(key)
        if suite is None:
            fail(f"Cargo target is absent from nextest: {key}")
        if key in seen_targets:
            fail(f"Cargo target identity is duplicated: {key}")
        seen_targets.add(key)
        binary = artifact.get("executable")
        if not isinstance(binary, str):
            fail(f"Cargo target has no executable: {key}")
        cwd = suite["cwd"]
        if not isinstance(cwd, str):
            fail(f"nextest target has no cwd: {key}")
        all_tests = listed(binary, cwd, False)
        ignored = listed(binary, cwd, True)
        unknown_ignored = ignored.keys() - all_tests.keys()
        if unknown_ignored:
            fail(f"Cargo ignored listing contains unknown tests: {sorted(unknown_ignored)!r}")
        binary_id = suite["binary-id"]
        for name, test_kind in all_tests.items():
            inventory.add(
                f"{binary_id}\t{name}\t{test_kind}\t{str(name in ignored).lower()}"
            )
    missing_targets = set(targets) - seen_targets
    if missing_targets:
        fail(f"nextest targets are absent from Cargo: {sorted(missing_targets)!r}")
    return inventory


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--nextest", required=True, type=pathlib.Path)
    parser.add_argument("--cargo", required=True, type=pathlib.Path)
    arguments = parser.parse_args()
    nextest, targets = read_nextest(arguments.nextest)
    cargo = read_cargo(arguments.cargo, targets)
    if cargo != nextest:
        missing = sorted(cargo - nextest)
        extra = sorted(nextest - cargo)
        fail(
            "Cargo and nextest inventories differ\n"
            f"  absent from nextest: {missing[:10]!r}\n"
            f"  absent from Cargo: {extra[:10]!r}"
        )
    canonical = "".join(f"{record}\n" for record in sorted(nextest)).encode()
    digest = hashlib.sha256(canonical).hexdigest()
    ignored = sum(record.endswith("\ttrue") for record in nextest)
    print(
        f"nextest-inventory: sha256={digest} tests={len(nextest)} "
        f"ignored={ignored} suites={len(targets)}"
    )


if __name__ == "__main__":
    main()
