#!/usr/bin/env python3
"""Classify changed repository surfaces for development validation.

This is the one production classifier. It deliberately uses only Python's
standard library, Git and content-bound generated data: classification must
happen before the repository decides which Rust targets need compiling.
``cargo xtask validation`` derives and audits the normalized policy consumed
here.
"""

from __future__ import annotations

import time

PROCESS_STARTED_NS = time.perf_counter_ns()

import argparse
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
from collections import defaultdict, deque
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, NoReturn


POLICY = Path("ci/validation-selection.json")
SOURCE = Path("ci/validation.yml")
POLICY_SCHEMA = "aplan/validation-selection-policy/v1"
REPORT_SCHEMA = 2
MAX_POLICY_BYTES = 1024 * 1024
MAX_GRAPH_INPUT_BYTES = 4 * 1024 * 1024
MAX_DIAGNOSTIC_BYTES = 4096
SHA256 = re.compile(r"[0-9a-f]{64}")
NAME = re.compile(r"[a-z][a-z0-9-]*")
BOOTSTRAP_GLOBAL_PATHS = (
    "ci/validation.yml",
    "ci/validation-selection.json",
    "ci/scripts/validation-select.py",
    "ci/scripts/preflight.sh",
)


class SelectionError(RuntimeError):
    """A fail-closed selector error."""


@dataclass
class Metrics:
    metadata_ns: int = 0
    git_ns: int = 0
    inventory_ns: int = 0
    census_ns: int = 0
    classification_ns: int = 0
    output_ns: int = 0
    cargo_processes: int = 0
    git_processes: int = 0

    def emit(self, total_finished_ns: int) -> None:
        document = {
            "cargo_processes": self.cargo_processes,
            "census_ms": milliseconds(self.census_ns),
            "classification_ms": milliseconds(self.classification_ns),
            "git_ms": milliseconds(self.git_ns),
            "git_processes": self.git_processes,
            "inventory_ms": milliseconds(self.inventory_ns),
            "metadata_ms": milliseconds(self.metadata_ns),
            "output_ms": milliseconds(self.output_ns),
            "python_startup_ms": milliseconds(self.inventory_started_ns - PROCESS_STARTED_NS),
            "rustc_processes": 0,
            "total_ms": milliseconds(total_finished_ns - PROCESS_STARTED_NS),
        }
        print(
            "validation-select: timings="
            + json.dumps(document, separators=(",", ":"), sort_keys=True),
            file=sys.stderr,
        )

    inventory_started_ns: int = field(default=0, repr=False)


@dataclass(frozen=True)
class Package:
    package_id: str
    name: str
    root: bytes


@dataclass
class Workspace:
    packages: list[Package]
    name_to_id: dict[str, str]
    id_to_name: dict[str, str]
    reverse: dict[str, set[str]]

    def package_names(self) -> list[str]:
        return [package.name for package in self.packages]


@dataclass
class Changed:
    paths: dict[bytes, set[str]] = field(default_factory=lambda: defaultdict(set))
    reasons: set[str] = field(default_factory=set)

    def observe(self, path: bytes, observation: str) -> None:
        self.paths[path].add(observation)

    def widen(self, reason: str) -> None:
        self.reasons.add(reason)


@dataclass(frozen=True)
class Options:
    root: Path
    mode: str
    output_format: str
    base: str | None = None
    head: str | None = None
    revisions: tuple[str, str] | None = None
    push_updates: tuple[tuple[str, str], ...] = ()
    timings: bool = False


def milliseconds(nanoseconds: int) -> float:
    return round(nanoseconds / 1_000_000, 3)


def fail(message: str) -> NoReturn:
    raise SelectionError(message)


def strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, member in pairs:
        if key in value:
            fail(f"generated selector policy repeats field `{key}`")
        value[key] = member
    return value


def require_mapping(value: Any, owner: str, fields: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        fail(f"{owner} must be an object")
    actual = set(value)
    if actual != fields:
        fail(
            f"{owner} fields differ: missing={sorted(fields - actual)!r}, "
            f"unexpected={sorted(actual - fields)!r}"
        )
    return value


def require_strings(value: Any, owner: str, *, nonempty: bool = False) -> list[str]:
    if not isinstance(value, list) or any(not isinstance(item, str) for item in value):
        fail(f"{owner} must be an array of strings")
    if nonempty and not value:
        fail(f"{owner} must not be empty")
    if len(value) != len(set(value)):
        fail(f"{owner} repeats a value")
    return value


def safe_relative_path(value: str, *, prefix: bool) -> bool:
    return bool(value) and not (
        value.startswith("/")
        or "//" in value
        or "\\" in value
        or any(ord(character) < 32 or ord(character) == 127 for character in value)
        or any(part in {".", ".."} for part in value.split("/"))
        or value.endswith("/") != prefix
    )


def is_exact_integer(value: Any, expected: int) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value == expected


def load_inventory(
    root: Path, metrics: Metrics
) -> tuple[dict[str, Any], str, Workspace]:
    metrics.inventory_started_ns = time.perf_counter_ns()
    started = metrics.inventory_started_ns
    policy_path = root / POLICY
    source_path = root / SOURCE
    try:
        policy_stat = policy_path.lstat()
        source_stat = source_path.lstat()
    except OSError as error:
        fail(f"validation selector policy input is unavailable: {error.strerror}")
    if not stat.S_ISREG(policy_stat.st_mode) or not stat.S_ISREG(source_stat.st_mode):
        fail("validation selector policy and source must be regular files")
    if policy_stat.st_size > MAX_POLICY_BYTES or source_stat.st_size > MAX_POLICY_BYTES:
        fail("validation selector policy input exceeds the 1 MiB bound")
    try:
        policy_bytes = policy_path.read_bytes()
        source_bytes = source_path.read_bytes()
        document = json.loads(policy_bytes, object_pairs_hook=strict_object)
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"could not read generated selector policy: {error}")

    document = require_mapping(
        document, "selector policy", {"schema", "source", "inventory", "workspace"}
    )
    if document["schema"] != POLICY_SCHEMA:
        fail(f"unsupported selector policy schema `{document['schema']}`")
    source = require_mapping(document["source"], "selector policy source", {"path", "sha256"})
    if source["path"] != SOURCE.as_posix() or not isinstance(source["sha256"], str):
        fail("selector policy names an unsupported source")
    actual_sha256 = hashlib.sha256(source_bytes).hexdigest()
    if not SHA256.fullmatch(source["sha256"]) or source["sha256"] != actual_sha256:
        fail(
            "generated selector policy is stale for ci/validation.yml; "
            "run `cargo xtask validation`"
        )
    inventory = validate_inventory(root, document["inventory"])
    workspace_started = time.perf_counter_ns()
    workspace = validate_workspace(root, document["workspace"])
    workspace_elapsed = time.perf_counter_ns() - workspace_started
    metrics.metadata_ns += workspace_elapsed
    metrics.inventory_ns += time.perf_counter_ns() - started - workspace_elapsed
    return inventory, actual_sha256, workspace


def validate_inventory(root: Path, value: Any) -> dict[str, Any]:
    inventory = require_mapping(value, "validation inventory", {"schema", "profiles", "selection", "gates"})
    if not is_exact_integer(inventory["schema"], 1):
        fail("validation inventory schema must be 1")
    profiles = require_mapping(
        inventory["profiles"], "validation profiles", {"development", "full", "release"}
    )
    expected_profiles = {
        "development": {"assurance": "selected", "requires": None, "same_source": False},
        "full": {"assurance": "full", "requires": None, "same_source": False},
        "release": {"assurance": "release", "requires": "full", "same_source": True},
    }
    for name, expected in expected_profiles.items():
        profile = require_mapping(
            profiles[name], f"profile `{name}`", {"assurance", "requires", "same_source"}
        )
        if (
            profile.get("assurance") != expected["assurance"]
            or profile.get("requires") != expected["requires"]
            or profile.get("same_source") is not expected["same_source"]
        ):
            fail(f"profile `{name}` does not match the validation contract")

    gates = inventory["gates"]
    if not isinstance(gates, list) or not gates:
        fail("validation gates must be a nonempty array")
    gate_names: set[str] = set()
    gate_commands: set[str] = set()
    development: dict[str, list[str]] = {"always": [], "selectable": [], "deferred": []}
    for index, gate_value in enumerate(gates):
        gate = require_mapping(
            gate_value,
            f"gate {index}",
            {"name", "command", "input_class", "isolation", "development", "full", "release"},
        )
        name = gate["name"]
        if not isinstance(name, str) or not NAME.fullmatch(name) or name in gate_names:
            fail("validation gate names must be unique kebab-case names")
        gate_names.add(name)
        command = gate["command"]
        expected_command = f"ci/scripts/{name}.sh"
        if command != expected_command or command in gate_commands:
            fail(f"validation gate `{name}` must own `{expected_command}` exactly once")
        gate_commands.add(command)
        command_path = root / command
        try:
            mode = command_path.stat().st_mode
        except OSError:
            fail(f"validation gate `{name}` command is missing")
        if not stat.S_ISREG(mode) or mode & 0o111 == 0:
            fail(f"validation gate `{name}` command is not an executable file")
        membership = gate["development"]
        if not isinstance(membership, str) or membership not in development:
            fail(f"validation gate `{name}` has invalid development membership")
        development[membership].append(name)
        if gate["full"] != "required" or gate["release"] != "inherited":
            fail(f"validation gate `{name}` weakens full or release membership")
        isolation = require_strings(gate["isolation"], f"gate `{name}` isolation", nonempty=True)
        if not isinstance(gate["input_class"], str) or gate["input_class"] not in {
            "repository",
            "rust-workspace",
            "generated-contracts",
            "declared-msrv",
            "dependency-policy",
            "plan-structure",
            "plan-and-evidence",
        }:
            fail(f"validation gate `{name}` has invalid input class")
        if any(
            requirement
            not in {
                "none",
                "shared-cargo-target",
                "isolated-cargo-target",
                "isolated-coverage-target",
                "isolated-aplan-cache",
                "full-git-history",
                "network-advisory-db",
            }
            for requirement in isolation
        ):
            fail(f"validation gate `{name}` has invalid isolation")
        if "none" in isolation and len(isolation) != 1:
            fail(f"validation gate `{name}` combines `none` with another isolation")
    if any(not names for names in development.values()):
        fail("development must contain always, selectable and deferred gates")

    selection = require_mapping(
        inventory["selection"], "validation selection", {"schema", "rust", "global_gates", "paths"}
    )
    if not is_exact_integer(selection["schema"], 1):
        fail("validation selection schema must be 1")
    rust = require_mapping(
        selection["rust"],
        "Rust selection",
        {"gates", "cross_cutting_packages", "global_basenames"},
    )
    rust_gates = require_strings(rust["gates"], "Rust selection gates", nonempty=True)
    cross_cutting = require_strings(
        rust["cross_cutting_packages"], "cross-cutting packages"
    )
    if any(not NAME.fullmatch(name) for name in cross_cutting):
        fail("cross-cutting package names must be kebab-case")
    global_basenames = require_strings(
        rust["global_basenames"], "global Rust basenames", nonempty=True
    )
    if any(
        "/" in name
        or "\\" in name
        or name in {".", ".."}
        or not name
        or any(ord(character) < 32 or ord(character) == 127 for character in name)
        for name in global_basenames
    ):
        fail("global Rust basenames must be safe basenames")
    global_gates = require_strings(selection["global_gates"], "global selection gates", nonempty=True)
    if set(global_gates) != set(development["selectable"]):
        fail("global selection gates must name every selectable gate exactly once")
    if any(gate not in development["selectable"] for gate in rust_gates):
        fail("Rust selection names a non-selectable gate")

    rules = selection["paths"]
    if not isinstance(rules, list) or not rules:
        fail("validation selection paths must be a nonempty array")
    rule_names: set[str] = set()
    exact_paths: set[str] = set()
    prefixes: set[str] = set()
    for index, rule_value in enumerate(rules):
        rule = require_mapping(
            rule_value,
            f"selection path rule {index}",
            {"name", "disposition", "gates", "exact", "prefixes"},
        )
        name = rule["name"]
        if not isinstance(name, str) or not NAME.fullmatch(name) or name in rule_names:
            fail("selection path rule names must be unique kebab-case names")
        rule_names.add(name)
        if not isinstance(rule["disposition"], str) or rule["disposition"] not in {
            "selected",
            "global",
        }:
            fail(f"selection rule `{name}` has an invalid disposition")
        gates_for_rule = require_strings(rule["gates"], f"selection rule `{name}` gates")
        if any(gate not in development["selectable"] for gate in gates_for_rule):
            fail(f"selection rule `{name}` names a non-selectable gate")
        if rule["disposition"] == "global" and set(gates_for_rule) != set(global_gates):
            fail(f"global selection rule `{name}` must name every selectable gate")
        exact = require_strings(rule["exact"], f"selection rule `{name}` exact paths")
        rule_prefixes = require_strings(rule["prefixes"], f"selection rule `{name}` prefixes")
        if not exact and not rule_prefixes:
            fail(f"selection rule `{name}` matches no paths")
        for path in exact:
            if not safe_relative_path(path, prefix=False) or path in exact_paths:
                fail(f"unsafe or duplicate exact selection path `{path}`")
            exact_paths.add(path)
        for prefix in rule_prefixes:
            if not safe_relative_path(prefix, prefix=True) or prefix in prefixes:
                fail(f"unsafe or duplicate selection prefix `{prefix}`")
            prefixes.add(prefix)

    for path in BOOTSTRAP_GLOBAL_PATHS:
        matching = [
            rule
            for rule in rules
            if path in rule["exact"]
            or any(path.startswith(prefix) for prefix in rule["prefixes"])
        ]
        if len(matching) != 1 or matching[0]["disposition"] != "global":
            fail(f"selector bootstrap path `{path}` must have one global disposition")

    inventory["_development"] = development
    return inventory


def validate_workspace(root: Path, value: Any) -> Workspace:
    document = require_mapping(
        value, "selector workspace graph", {"inputs", "packages", "reverse"}
    )
    inputs = document["inputs"]
    if not isinstance(inputs, list) or not inputs:
        fail("selector workspace inputs must be a nonempty array")
    input_paths: set[str] = set()
    ordered_input_paths: list[str] = []
    for index, value in enumerate(inputs):
        item = require_mapping(value, f"selector workspace input {index}", {"path", "sha256"})
        path = item["path"]
        digest = item["sha256"]
        if (
            not isinstance(path, str)
            or not safe_relative_path(path, prefix=False)
            or path in input_paths
            or not isinstance(digest, str)
            or not SHA256.fullmatch(digest)
        ):
            fail("selector workspace inputs must have unique safe paths and SHA-256 digests")
        input_paths.add(path)
        ordered_input_paths.append(path)
        source = root / path
        try:
            metadata = source.lstat()
        except OSError as error:
            fail(f"selector workspace input `{path}` is unavailable: {error.strerror}")
        if not stat.S_ISREG(metadata.st_mode):
            fail(f"selector workspace input `{path}` must be a regular file")
        if metadata.st_size > MAX_GRAPH_INPUT_BYTES:
            fail(f"selector workspace input `{path}` exceeds the 4 MiB bound")
        try:
            content = source.read_bytes()
        except OSError as error:
            fail(f"selector workspace input `{path}` is unreadable: {error.strerror}")
        if hashlib.sha256(content).hexdigest() != digest:
            fail(
                f"generated selector workspace graph is stale for `{path}`; "
                "run `cargo xtask validation`"
            )
    required_inputs = {"Cargo.toml", "Cargo.lock"}
    if not required_inputs.issubset(input_paths):
        fail("selector workspace graph omits a root Cargo input")
    if ordered_input_paths != sorted(ordered_input_paths):
        fail("selector workspace inputs are not in deterministic path order")

    package_values = document["packages"]
    if not isinstance(package_values, list) or not package_values:
        fail("selector workspace packages must be a nonempty array")
    packages: list[Package] = []
    package_names: set[str] = set()
    package_roots: set[str] = set()
    for index, value in enumerate(package_values):
        item = require_mapping(value, f"selector workspace package {index}", {"name", "root"})
        name = item["name"]
        package_root = item["root"]
        if (
            not isinstance(name, str)
            or not NAME.fullmatch(name)
            or name in package_names
            or not isinstance(package_root, str)
            or not safe_relative_path(package_root, prefix=True)
            or package_root in package_roots
        ):
            fail("selector workspace packages must have unique names and safe roots")
        package_names.add(name)
        package_roots.add(package_root)
        manifest = package_root + "Cargo.toml"
        if manifest not in input_paths:
            fail(f"selector workspace package `{name}` has no bound manifest input")
        packages.append(Package(name, name, package_root.encode("utf-8")))
    if packages != sorted(packages, key=lambda package: package.name):
        fail("selector workspace packages are not in deterministic name order")

    reverse_value = document["reverse"]
    if not isinstance(reverse_value, dict):
        fail("selector workspace reverse graph must be an object")
    reverse: dict[str, set[str]] = {}
    for name, dependents_value in reverse_value.items():
        if name not in package_names:
            fail(f"selector workspace reverse graph names unknown package `{name}`")
        dependents = require_strings(
            dependents_value, f"selector workspace dependents for `{name}`"
        )
        if dependents != sorted(dependents):
            fail(f"selector workspace dependents for `{name}` are not ordered")
        if any(dependent not in package_names or dependent == name for dependent in dependents):
            fail(f"selector workspace reverse graph has an invalid edge from `{name}`")
        reverse[name] = set(dependents)
    return Workspace(
        packages,
        {name: name for name in package_names},
        {name: name for name in package_names},
        reverse,
    )


def run_process(
    program: str, arguments: list[str | bytes], root: Path, metrics: Metrics
) -> subprocess.CompletedProcess[bytes]:
    started = time.perf_counter_ns()
    environment = dict(os.environ)
    environment["LC_ALL"] = "C"
    try:
        output = subprocess.run(
            [program, *arguments],
            cwd=root,
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
    except OSError as error:
        fail(f"could not start {program}: {error.strerror}")
    elapsed = time.perf_counter_ns() - started
    metrics.git_ns += elapsed
    metrics.git_processes += 1
    return output


def bounded_stderr(output: bytes, root: Path) -> str:
    text = output[:MAX_DIAGNOSTIC_BYTES].decode("utf-8", errors="replace").strip()
    return text.replace(str(root), ".")


def run_git(root: Path, arguments: list[str | bytes], metrics: Metrics) -> bytes:
    output = run_process("git", arguments, root, metrics)
    if output.returncode != 0:
        rendered = " ".join(display_path(os.fsencode(argument)) for argument in arguments)
        fail(f"git {rendered} failed: {bounded_stderr(output.stderr, root)}")
    return output.stdout


def nul_records(value: bytes) -> list[bytes]:
    return [record for record in value.split(b"\0") if record]


def display_path(path: bytes) -> str:
    try:
        text = path.decode("utf-8")
    except UnicodeDecodeError:
        return "hex:" + path.hex()
    output: list[str] = []
    for character in text:
        codepoint = ord(character)
        if character == "\\":
            output.append("\\\\")
        elif character == "\n":
            output.append("\\n")
        elif character == "\r":
            output.append("\\r")
        elif character == "\t":
            output.append("\\t")
        elif codepoint < 32 or codepoint == 127:
            output.append(f"\\u{{{codepoint:x}}}")
        else:
            output.append(character)
    return "".join(output)


def display_text(text: str) -> str:
    return display_path(text.encode("utf-8"))


def classify(path: bytes, inventory: dict[str, Any], workspace: Workspace) -> tuple[str, Any]:
    package_matches = [package for package in workspace.packages if path.startswith(package.root)]
    package = max(package_matches, key=lambda candidate: len(candidate.root), default=None)
    rules = inventory["selection"]["paths"]
    matching_rules = [
        (index, rule)
        for index, rule in enumerate(rules)
        if any(path == exact.encode() for exact in rule["exact"])
        or any(path.startswith(prefix.encode()) for prefix in rule["prefixes"])
    ]
    basename = path.rsplit(b"/", 1)[-1]
    global_basename = None
    if package is not None:
        global_basename = next(
            (
                candidate
                for candidate in inventory["selection"]["rust"]["global_basenames"]
                if candidate.encode() == basename
            ),
            None,
        )
    if package is not None and not matching_rules and global_basename is not None:
        return "global-rust", global_basename
    if package is not None and not matching_rules:
        return "package", package
    if package is None and len(matching_rules) == 1:
        return "rule", matching_rules[0][0]
    if package is None and not matching_rules:
        return "unknown", None
    return "ambiguous", None


def audit_census(
    root: Path, inventory: dict[str, Any], workspace: Workspace, metrics: Metrics
) -> None:
    started = time.perf_counter_ns()
    git_before = metrics.git_ns
    for package in inventory["selection"]["rust"]["cross_cutting_packages"]:
        if package not in workspace.name_to_id:
            fail(f"selection names unknown cross-cutting package `{package}`")
    tracked = nul_records(run_git(root, ["ls-files", "-z"], metrics))
    if not tracked:
        fail("tracked-file census found no files")
    rules = inventory["selection"]["paths"]
    exact_hits: list[set[str]] = [set() for _ in rules]
    prefix_hits: list[set[str]] = [set() for _ in rules]
    package_hits: set[str] = set()
    basename_hits: set[str] = set()
    for path in tracked:
        if not path:
            fail("git ls-files returned an empty path")
        try:
            text = path.decode("utf-8")
        except UnicodeDecodeError:
            fail(f"tracked-file census found non-UTF-8 path `{display_path(path)}`")
        kind, subject = classify(path, inventory, workspace)
        if kind == "package":
            package_hits.add(subject.name)
        elif kind == "global-rust":
            basename_hits.add(subject)
        elif kind == "rule":
            rule = rules[subject]
            exact_hits[subject].update(exact for exact in rule["exact"] if text == exact)
            prefix_hits[subject].update(prefix for prefix in rule["prefixes"] if text.startswith(prefix))
        elif kind == "unknown":
            fail(f"tracked path `{display_path(path)}` has no validation-selection disposition")
        else:
            fail(f"tracked path `{display_path(path)}` has more than one validation-selection disposition")
    for package in workspace.packages:
        if package.name not in package_hits:
            fail(f"workspace package `{package.name}` owns no tracked path in the validation census")
    for basename in inventory["selection"]["rust"]["global_basenames"]:
        if basename not in basename_hits:
            fail(f"global Rust basename `{basename}` matches no tracked package file")
    for index, rule in enumerate(rules):
        for exact in rule["exact"]:
            if exact not in exact_hits[index]:
                fail(f"selection rule `{rule['name']}` has stale exact path `{exact}`")
        for prefix in rule["prefixes"]:
            if prefix not in prefix_hits[index]:
                fail(f"selection rule `{rule['name']}` has stale prefix `{prefix}`")
    metrics.census_ns += time.perf_counter_ns() - started - (metrics.git_ns - git_before)


def parse_name_status(value: bytes, source: str, changed: Changed) -> None:
    fields = nul_records(value)
    index = 0
    while index < len(fields):
        try:
            status = fields[index].decode("utf-8")
        except UnicodeDecodeError:
            fail("git returned a non-UTF-8 change status")
        index += 1
        if not status:
            fail("git returned an empty change status")
        if status[0] in {"R", "C"}:
            if index + 1 >= len(fields):
                fail("rename/copy record is missing a path")
            old, new = fields[index], fields[index + 1]
            index += 2
            label = "rename" if status[0] == "R" else "copy"
            changed.observe(old, f"{source}:{label}-old:{status}")
            changed.observe(new, f"{source}:{label}-new:{status}")
        else:
            if index >= len(fields):
                fail("change record has no path")
            changed.observe(fields[index], f"{source}:{status}")
            index += 1


def collect_diff(
    root: Path, arguments: list[str | bytes], source: str, changed: Changed, metrics: Metrics
) -> None:
    try:
        parse_name_status(run_git(root, arguments, metrics), source, changed)
    except SelectionError as error:
        changed.widen(f"could not read or parse {source} change set: {error}")


def parse_sha(value: bytes) -> str:
    try:
        sha = value.decode("utf-8").strip()
    except UnicodeDecodeError:
        fail("git returned a non-UTF-8 object id")
    if len(sha) not in {40, 64} or any(character not in "0123456789abcdefABCDEF" for character in sha):
        fail(f"git returned malformed object id `{display_text(sha)}`")
    return sha.lower()


def resolve_commit(root: Path, revision: str, metrics: Metrics) -> str:
    return parse_sha(run_git(root, ["rev-parse", "--verify", f"{revision}^{{commit}}"], metrics))


def resolve_or_widen(
    root: Path, revision: str, role: str, changed: Changed, metrics: Metrics
) -> str | None:
    try:
        return resolve_commit(root, revision, metrics)
    except SelectionError:
        changed.widen(
            f"could not resolve {role} `{display_text(revision)}` to an immutable commit "
            "(history may be shallow)"
        )
        return None


def collect_revision_diff(
    root: Path, before: str, after: str, source: str, changed: Changed, metrics: Metrics
) -> None:
    collect_diff(
        root,
        ["diff", "--name-status", "-z", "--find-renames", before, after, "--"],
        source,
        changed,
        metrics,
    )


def require_checkout_head(root: Path, selected_head: str, changed: Changed, metrics: Metrics) -> None:
    try:
        checkout = resolve_commit(root, "HEAD", metrics)
        if checkout != selected_head:
            changed.widen(
                f"selected head `{selected_head}` differs from checked-out metadata source `{checkout}`"
            )
    except SelectionError as error:
        changed.widen(f"could not resolve checked-out metadata source: {error}")
    try:
        status = run_git(
            root,
            ["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            metrics,
        )
        if status:
            changed.widen("checked-out metadata source has staged, unstaged or untracked changes")
    except SelectionError as error:
        changed.widen(f"could not confirm the checked-out metadata source is clean: {error}")


def collect_working_tree(root: Path, metrics: Metrics) -> tuple[Changed, dict[str, Any], str]:
    changed = Changed()
    collect_diff(
        root,
        ["diff", "--cached", "--name-status", "-z", "--find-renames"],
        "staged",
        changed,
        metrics,
    )
    collect_diff(
        root,
        ["diff", "--name-status", "-z", "--find-renames"],
        "unstaged",
        changed,
        metrics,
    )
    try:
        for path in nul_records(
            run_git(root, ["ls-files", "--others", "--exclude-standard", "-z"], metrics)
        ):
            changed.observe(path, "untracked")
    except SelectionError as error:
        changed.widen(f"could not enumerate untracked paths: {error}")
    try:
        resolved_head = resolve_commit(root, "HEAD", metrics)
    except SelectionError:
        resolved_head = None
    inputs = {
        "requested_base": None,
        "requested_head": "HEAD",
        "resolved_base": None,
        "resolved_head": resolved_head,
        "merge_base": None,
        "push_updates": [],
    }
    return changed, inputs, "working-tree"


def collect_pre_push(
    root: Path, updates: tuple[tuple[str, str], ...], metrics: Metrics
) -> tuple[Changed, dict[str, Any], str]:
    changed, working_inputs, _ = collect_working_tree(root, metrics)
    checkout = working_inputs["resolved_head"]
    push_updates = []
    for index, (before, after) in enumerate(updates):
        resolved_before = resolve_or_widen(root, before, "push base", changed, metrics)
        resolved_after = resolve_or_widen(root, after, "push head", changed, metrics)
        if resolved_before is not None and resolved_after is not None:
            collect_revision_diff(
                root, resolved_before, resolved_after, f"pre-push:{index + 1}", changed, metrics
            )
            if checkout != resolved_after:
                changed.widen(
                    f"push head `{resolved_after}` differs from checked-out validation source "
                    f"`{checkout or 'unresolved'}`"
                )
        push_updates.append(
            {
                "requested_from": before,
                "requested_to": after,
                "resolved_from": resolved_before,
                "resolved_to": resolved_after,
            }
        )
    return (
        changed,
        {
            "requested_base": None,
            "requested_head": "HEAD",
            "resolved_base": None,
            "resolved_head": checkout,
            "merge_base": None,
            "push_updates": push_updates,
        },
        "pre-push",
    )


def collect_base_head(
    root: Path, base: str, head: str, metrics: Metrics
) -> tuple[Changed, dict[str, Any], str]:
    changed = Changed()
    resolved_base = resolve_or_widen(root, base, "base", changed, metrics)
    resolved_head = resolve_or_widen(root, head, "head", changed, metrics)
    merge_base = None
    if resolved_base is not None and resolved_head is not None:
        try:
            merge_base = parse_sha(run_git(root, ["merge-base", resolved_base, resolved_head], metrics))
            collect_revision_diff(root, merge_base, resolved_head, "base-head", changed, metrics)
        except SelectionError:
            changed.widen(
                f"no merge base is available for `{display_text(base)}` and "
                f"`{display_text(head)}` (history may be shallow)"
            )
        require_checkout_head(root, resolved_head, changed, metrics)
    return (
        changed,
        {
            "requested_base": base,
            "requested_head": head,
            "resolved_base": resolved_base,
            "resolved_head": resolved_head,
            "merge_base": merge_base,
            "push_updates": [],
        },
        "base-head",
    )


def collect_revisions(
    root: Path, before: str, after: str, metrics: Metrics
) -> tuple[Changed, dict[str, Any], str]:
    changed = Changed()
    resolved_before = resolve_or_widen(root, before, "from revision", changed, metrics)
    resolved_after = resolve_or_widen(root, after, "to revision", changed, metrics)
    if resolved_before is not None and resolved_after is not None:
        collect_revision_diff(root, resolved_before, resolved_after, "revisions", changed, metrics)
        require_checkout_head(root, resolved_after, changed, metrics)
    return (
        changed,
        {
            "requested_base": before,
            "requested_head": after,
            "resolved_base": resolved_before,
            "resolved_head": resolved_after,
            "merge_base": None,
            "push_updates": [],
        },
        "revisions",
    )


def reverse_closure(workspace: Workspace, seeds: set[str]) -> set[str]:
    selected = set(seeds)
    queue = deque(sorted(seeds))
    while queue:
        package_id = queue.popleft()
        for dependent in sorted(workspace.reverse.get(package_id, set())):
            if dependent not in selected:
                selected.add(dependent)
                queue.append(dependent)
    return selected


def select(
    inventory: dict[str, Any],
    inventory_sha256: str,
    workspace: Workspace,
    changed: Changed,
    inputs: dict[str, Any],
    mode: str,
    metrics: Metrics,
) -> dict[str, Any]:
    started = time.perf_counter_ns()
    directly_affected: set[str] = set()
    selected_gates: set[str] = set()
    changes = []
    for path in sorted(changed.paths):
        if not is_utf8(path):
            changed.widen(f"path `{display_path(path)}` cannot be represented as UTF-8")
            classification = "unknown:non-utf8"
        else:
            kind, subject = classify(path, inventory, workspace)
            if kind == "package":
                directly_affected.add(subject.package_id)
                selected_gates.update(inventory["selection"]["rust"]["gates"])
                classification = f"package:{subject.name}"
            elif kind == "global-rust":
                changed.widen(
                    f"path `{display_path(path)}` is global Rust topology `{subject}`"
                )
                classification = "global:rust-topology"
            elif kind == "rule":
                rule = inventory["selection"]["paths"][subject]
                if rule["disposition"] == "global":
                    changed.widen(
                        f"path `{display_path(path)}` has global disposition `{rule['name']}`"
                    )
                else:
                    selected_gates.update(rule["gates"])
                classification = f"{rule['disposition']}:{rule['name']}"
            elif kind == "unknown":
                changed.widen(f"path `{display_path(path)}` is outside the validation inventory")
                classification = "unknown"
            else:
                changed.widen(f"path `{display_path(path)}` has ambiguous validation dispositions")
                classification = "ambiguous"
        changes.append(
            {
                "path": display_path(path),
                "observations": sorted(changed.paths[path]),
                "classification": classification,
            }
        )
    selected_ids = reverse_closure(workspace, directly_affected)
    if directly_affected:
        for package in inventory["selection"]["rust"]["cross_cutting_packages"]:
            package_id = workspace.name_to_id.get(package)
            if package_id is None:
                changed.widen(f"cross-cutting package `{package}` is not in the workspace")
            else:
                selected_ids.add(package_id)
    widened = bool(changed.reasons)
    if widened:
        selected_ids.update(workspace.id_to_name)
    selected_packages = sorted(
        workspace.id_to_name[package_id]
        for package_id in selected_ids
        if package_id in workspace.id_to_name
    )
    selected_gate_list = sorted(selected_gates)
    widened_gates = (
        [
            gate
            for gate in inventory["selection"]["global_gates"]
            if gate not in selected_gates
        ]
        if widened
        else []
    )
    covered = selected_gates | set(widened_gates)
    development = inventory["_development"]
    deferred_gates = sorted(
        gate
        for gate in development["selectable"] + development["deferred"]
        if gate not in covered
    )
    metrics.classification_ns += time.perf_counter_ns() - started
    return {
        "schema": REPORT_SCHEMA,
        "mode": mode,
        "inventory_sha256": inventory_sha256,
        "inputs": inputs,
        "changes": changes,
        "workspace_packages": workspace.package_names(),
        "selected_packages": selected_packages,
        "selected_gates": selected_gate_list,
        "widened_gates": widened_gates,
        "always_gates": development["always"],
        "deferred_gates": deferred_gates,
        "widened": widened,
        "widening_reasons": sorted(changed.reasons),
        "no_changes": not changes and not changed.reasons,
    }


def is_utf8(value: bytes) -> bool:
    try:
        value.decode("utf-8")
        return True
    except UnicodeDecodeError:
        return False


def census_report(
    inventory: dict[str, Any], inventory_sha256: str, workspace: Workspace
) -> dict[str, Any]:
    development = inventory["_development"]
    return {
        "schema": REPORT_SCHEMA,
        "mode": "census",
        "inventory_sha256": inventory_sha256,
        "inputs": {
            "requested_base": None,
            "requested_head": None,
            "resolved_base": None,
            "resolved_head": None,
            "merge_base": None,
            "push_updates": [],
        },
        "changes": [],
        "workspace_packages": workspace.package_names(),
        "selected_packages": [],
        "selected_gates": [],
        "widened_gates": [],
        "always_gates": development["always"],
        "deferred_gates": [],
        "widened": False,
        "widening_reasons": [],
        "no_changes": True,
    }


def parse_options(arguments: list[str]) -> Options:
    parser = argparse.ArgumentParser(prog="validation-select.py")
    parser.add_argument("--root", default=".")
    parser.add_argument("--working-tree", action="store_true")
    parser.add_argument("--pre-push", nargs=2, action="append", metavar=("FROM", "TO"))
    parser.add_argument("--base")
    parser.add_argument("--head")
    parser.add_argument("--revisions", nargs=2, metavar=("FROM", "TO"))
    parser.add_argument("--census", action="store_true")
    parser.add_argument("--format", choices=("human", "json", "arguments"), default="human")
    parser.add_argument("--timings", action="store_true")
    parsed = parser.parse_args(arguments)
    root = Path(parsed.root)
    if not root.is_dir():
        parser.error("validation selection root is not a directory")
    base_head = parsed.base is not None or parsed.head is not None
    if base_head and (parsed.base is None or parsed.head is None):
        parser.error("--base and --head must be supplied together")
    modes = sum(
        [
            parsed.working_tree,
            bool(parsed.pre_push),
            base_head,
            parsed.revisions is not None,
            parsed.census,
        ]
    )
    if modes != 1:
        parser.error("exactly one input mode is required")
    if parsed.working_tree:
        mode = "working-tree"
    elif parsed.pre_push:
        mode = "pre-push"
    elif base_head:
        mode = "base-head"
    elif parsed.revisions is not None:
        mode = "revisions"
    else:
        mode = "census"
    return Options(
        root=root.resolve(),
        mode=mode,
        output_format=parsed.format,
        base=parsed.base,
        head=parsed.head,
        revisions=tuple(parsed.revisions) if parsed.revisions is not None else None,
        push_updates=tuple(tuple(update) for update in parsed.pre_push or []),
        timings=parsed.timings,
    )


def collect_changes(options: Options, metrics: Metrics) -> tuple[Changed, dict[str, Any], str]:
    if options.mode == "working-tree":
        return collect_working_tree(options.root, metrics)
    if options.mode == "pre-push":
        return collect_pre_push(options.root, options.push_updates, metrics)
    if options.mode == "base-head":
        assert options.base is not None and options.head is not None
        return collect_base_head(options.root, options.base, options.head, metrics)
    if options.mode == "revisions":
        assert options.revisions is not None
        return collect_revisions(options.root, *options.revisions, metrics)
    raise AssertionError("census has no change set")


def render_human(report: dict[str, Any]) -> str:
    def list_or_none(values: list[str]) -> str:
        return " ".join(values) if values else "(none)"

    lines = [
        f"validation selection: mode={report['mode']}",
        f"inventory sha256: {report['inventory_sha256']}",
    ]
    inputs = report["inputs"]
    for label, key in [
        ("resolved base", "resolved_base"),
        ("resolved head", "resolved_head"),
        ("merge base", "merge_base"),
    ]:
        if inputs[key] is not None:
            lines.append(f"{label}: {inputs[key]}")
    for update in inputs["push_updates"]:
        lines.append(
            "push update: "
            f"{display_text(update['requested_from'])} -> {display_text(update['requested_to'])} "
            f"(resolved {update['resolved_from'] or 'unresolved'} -> "
            f"{update['resolved_to'] or 'unresolved'})"
        )
    for change in report["changes"]:
        lines.append(
            f"change: {change['path']} [{', '.join(change['observations'])}] -> "
            f"{change['classification']}"
        )
    for label, key in [
        ("workspace packages", "workspace_packages"),
        ("selected packages", "selected_packages"),
        ("always gates", "always_gates"),
        ("selected gates", "selected_gates"),
        ("widened gates", "widened_gates"),
        ("deferred gates", "deferred_gates"),
    ]:
        lines.append(f"{label}: {list_or_none(report[key])}")
    lines.append(f"widened: {str(report['widened']).lower()}")
    lines.extend(f"widening reason: {reason}" for reason in report["widening_reasons"])
    return "\n".join(lines) + "\n"


def write_report(report: dict[str, Any], output_format: str) -> None:
    if output_format == "json":
        json.dump(report, sys.stdout, indent=2, ensure_ascii=False)
        sys.stdout.write("\n")
    elif output_format == "human":
        sys.stdout.write(render_human(report))
    else:
        records: list[tuple[str, str]] = [("schema", str(REPORT_SCHEMA))]
        records.extend(("gate", value) for value in report["selected_gates"])
        records.extend(("package", value) for value in report["selected_packages"])
        records.extend(("widen", value) for value in report["widening_reasons"])
        records.append(("no-changes", str(report["no_changes"]).lower()))
        output = sys.stdout.buffer
        for kind, value in records:
            output.write(kind.encode("utf-8") + b"\0" + value.encode("utf-8") + b"\0")


def main(arguments: list[str]) -> int:
    options = parse_options(arguments)
    metrics = Metrics()
    try:
        inventory, inventory_sha256, workspace = load_inventory(options.root, metrics)
        audit_census(options.root, inventory, workspace, metrics)
        if options.mode == "census":
            report = census_report(inventory, inventory_sha256, workspace)
        else:
            changed, inputs, mode = collect_changes(options, metrics)
            report = select(
                inventory,
                inventory_sha256,
                workspace,
                changed,
                inputs,
                mode,
                metrics,
            )
        output_started = time.perf_counter_ns()
        write_report(report, options.output_format)
        metrics.output_ns += time.perf_counter_ns() - output_started
        return 0
    finally:
        if options.timings:
            metrics.emit(time.perf_counter_ns())


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except SelectionError as error:
        message = str(error)
        if len(message.encode("utf-8")) > MAX_DIAGNOSTIC_BYTES:
            message = message.encode("utf-8")[:MAX_DIAGNOSTIC_BYTES].decode(
                "utf-8", errors="replace"
            )
        print(f"validation-select: error: {message}", file=sys.stderr)
        raise SystemExit(1) from None
    except BrokenPipeError:
        try:
            sys.stdout.close()
        finally:
            raise SystemExit(1) from None
