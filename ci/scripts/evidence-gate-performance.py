#!/usr/bin/env python3
"""Build and measure the P0000021-shaped evidence-gate corpus.

Wall time is an observation. Provider events are the portable contract: the
fixture records every catalogue, exact-test, and miss-probe process so a change
in work cannot hide behind a faster host. Receipt setup is outside the measured
case; receipt-current verification consumes the production schema and resolver,
then proves the measured gate started no provider process.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import re
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Callable


CORPUS_VERSION = "aplan-evidence-gate-corpus/v1"
RESULT_PROTOCOL = "aplan-evidence-batch-result/v1"
RUNNER_TEST_COUNTS = (140, 16, 21, 9, 6, 1, 5, 1)
REPEATED_CITATIONS = 42
REQUIRED_IDENTITY_FIELDS = {
    "source_sha256",
    "binary_sha256",
    "protocol_sha256",
    "policy_sha256",
    "runner_sha256",
    "environment_sha256",
}
NEGATIVE_CONTROLS = (
    "missing",
    "stale",
    "malformed",
    "incomplete",
    "replayed",
    "mismatched-material",
)
DIGEST = re.compile(r"^[0-9a-f]{64}$")
TIMING_PREFIX = "aplan: timings "
RECEIPT_PHASE_FIELDS = (
    "receipt_manifest_ms",
    "receipt_material_ms",
    "receipt_parse_ms",
    "receipt_identity_ms",
    "receipt_authentication_ms",
    "receipt_catalogue_ms",
)


PROVIDER_SOURCE = r'''#!/usr/bin/env python3
import json
import os
import sys
import time
from pathlib import Path

mode, runner, catalogue, events_arg, state_arg, cold_ms, slow_ms, batch_ms, fail_runner, *rest = sys.argv[1:]
events = events_arg.removeprefix("events=")
state = state_arg.removeprefix("state=")

def record(kind, name=None):
    event = {"kind": kind, "runner": runner}
    if name is not None:
        event["name"] = name
    encoded = (json.dumps(event, sort_keys=True) + "\n").encode("utf-8")
    descriptor = os.open(events, os.O_WRONLY | os.O_CREAT | os.O_APPEND, 0o600)
    try:
        os.write(descriptor, encoded)
    finally:
        os.close(descriptor)

if mode == "list":
    record("catalogue")
    if runner == fail_runner:
        print("injected catalogue failure", file=sys.stderr)
        raise SystemExit(23)
    marker = Path(state) / (runner + ".warm")
    if not marker.exists():
        time.sleep(float(cold_ms) / 1000.0)
        marker.parent.mkdir(parents=True, exist_ok=True)
        marker.write_text("warm\n", encoding="utf-8")
    if runner == "runner-8":
        time.sleep(float(slow_ms) / 1000.0)
    sys.stdout.write(Path(catalogue).read_text(encoding="utf-8"))
elif mode == "run":
    name = rest[0]
    kind = "probe" if name.startswith("aplan-probe-a-name-no-test-can-have-") else "test"
    record(kind, name)
    names = set(Path(catalogue).read_text(encoding="utf-8").splitlines())
    if name in names:
        print("PASS " + name)
    else:
        print("NO MATCH")
elif mode == "batch":
    manifest, result = rest
    record("batch")
    time.sleep(float(batch_ms) / 1000.0)
    request = json.loads(Path(manifest).read_text(encoding="utf-8"))
    answer = {
        "schema": "aplan/test-batch-result/v1",
        "nonce": request["nonce"],
        "provider": request["provider"],
        "complete": True,
        "tests": [
            {"test": name, "outcome": "passed"}
            for name in request["tests"]
        ],
    }
    Path(result).write_text(
        json.dumps(answer, sort_keys=True, separators=(",", ":")),
        encoding="utf-8",
    )
else:
    raise SystemExit("unknown provider mode: " + mode)
'''


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def git_environment() -> dict[str, str]:
    """Run fixture Git commands without a detached maintenance writer.

    A clean hosted runner reproduced `git commit` returning before automatic
    maintenance finished with the fixture repository. Temporary-directory
    cleanup then raced a writer below `.git` and failed with ENOTEMPTY. The
    corpus owns these short-lived repositories and gains nothing from packing
    them, so disable both automatic entry points for every Git observation as
    well as fixture creation.
    """
    environment = os.environ.copy()
    environment.update(
        {
            "GIT_CONFIG_COUNT": "2",
            "GIT_CONFIG_KEY_0": "gc.auto",
            "GIT_CONFIG_VALUE_0": "0",
            "GIT_CONFIG_KEY_1": "maintenance.auto",
            "GIT_CONFIG_VALUE_1": "false",
        }
    )
    return environment


def stable_json(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def source_identity(root: Path) -> str:
    """Hash the Git identity and all worktree changes without trusting mtime."""
    try:
        head = subprocess.check_output(
            ["git", "rev-parse", "HEAD"],
            cwd=root,
            stderr=subprocess.DEVNULL,
            env=git_environment(),
        )
        status = subprocess.check_output(
            ["git", "status", "--porcelain=v1", "-z", "--untracked-files=all"],
            cwd=root,
            stderr=subprocess.DEVNULL,
            env=git_environment(),
        )
        delta = subprocess.check_output(
            ["git", "diff", "--no-ext-diff", "--no-textconv", "--binary", "HEAD", "--"],
            cwd=root,
            stderr=subprocess.DEVNULL,
            env=git_environment(),
        )
        untracked = subprocess.check_output(
            ["git", "ls-files", "--others", "--exclude-standard", "-z"],
            cwd=root,
            stderr=subprocess.DEVNULL,
            env=git_environment(),
        )
    except (OSError, subprocess.CalledProcessError) as error:
        raise RuntimeError(f"benchmark source is not an inspectable Git checkout: {error}") from error
    digest = hashlib.sha256(b"HEAD\0" + head + b"STATUS\0" + status + b"DIFF\0" + delta)
    for raw in untracked.split(b"\0"):
        if not raw:
            continue
        path = root / os.fsdecode(raw)
        digest.update(b"UNTRACKED\0" + raw + b"\0")
        if path.is_symlink():
            digest.update(b"LINK\0" + os.fsencode(os.readlink(path)))
        elif path.is_file():
            with path.open("rb") as source:
                while chunk := source.read(1024 * 1024):
                    digest.update(chunk)
        else:
            digest.update(b"SPECIAL")
    return digest.hexdigest()


def environment_identity() -> str:
    material = {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "python": platform.python_version(),
    }
    return sha256_bytes(stable_json(material))


def runner_identity(root: Path) -> str:
    """Hash every executable/catalogue input that defines provider behaviour."""
    digest = hashlib.sha256()
    paths = [root / "provider.py"]
    paths.extend(sorted((root / "catalogues").glob("*.txt")))
    paths.extend(sorted(root.glob("config-*.template")))
    for path in paths:
        relative = path.relative_to(root).as_posix().encode("utf-8")
        digest.update(relative + b"\0")
        digest.update(path.read_bytes())
        digest.update(b"\0")
    return digest.hexdigest()


def corpus_description() -> dict[str, object]:
    unique = sum(RUNNER_TEST_COUNTS)
    return {
        "corpus_version": CORPUS_VERSION,
        "result_protocol": RESULT_PROTOCOL,
        "runner_count": len(RUNNER_TEST_COUNTS),
        "tests_per_runner": list(RUNNER_TEST_COUNTS),
        "distinct_tests": unique,
        "repeated_citations": REPEATED_CITATIONS,
        "citations": unique + REPEATED_CITATIONS,
        "fixtures": [
            "no-provider",
            "catalogue-cold",
            "catalogue-warm",
            "slow-runner",
            "failing-runner",
            "direct-replay",
            "batched-serial",
            "bounded-concurrent",
            "receipt-current",
        ],
        "receipt_counters": {
            "provider_process_spawns": 0,
            "imported_providers": len(RUNNER_TEST_COUNTS),
            "imported_outcomes": unique,
            "citations_examined": unique + REPEATED_CITATIONS,
        },
        "receipt_negative_controls": list(NEGATIVE_CONTROLS),
        "budgets_ms": {
            "cached_check_p95": 100,
            "local_receipt_verify_p95": 1000,
            "available_receipt_verify_p95": 10000,
            "post_setup_ci_gate": 60000,
        },
    }


def validate_reference_observation(value: object, expected: dict[str, object]) -> str:
    """Reference refusal model for the future receipt benchmark fixtures.

    This is not the production receipt verifier. It makes A1-648's negative
    corpus executable before A1-644 implements the schema: every future
    implementation must make the same controls refuse for the same typed
    reason, then A1-647 mutation-tests the production path independently.
    """
    if value is None:
        return "missing"
    if not isinstance(value, dict):
        return "malformed"
    required = {
        "schema",
        "provider",
        "nonce",
        "policy_sha256",
        "catalogue_sha256",
        "material_sha256",
        "complete",
        "outcomes",
    }
    if required - set(value):
        return "malformed"
    if value["schema"] != expected["schema"] or value["provider"] != expected["provider"]:
        return "stale"
    if value["nonce"] != expected["nonce"]:
        return "replayed"
    if value["material_sha256"] != expected["material_sha256"]:
        return "mismatched-material"
    if (
        value["policy_sha256"] != expected["policy_sha256"]
        or value["catalogue_sha256"] != expected["catalogue_sha256"]
    ):
        return "stale"
    outcomes = value["outcomes"]
    if not isinstance(value["complete"], bool) or not isinstance(outcomes, list):
        return "malformed"
    if not value["complete"] or outcomes != expected["outcomes"]:
        return "incomplete"
    return "current"


def exercise_receipt_negative_controls() -> dict[str, str]:
    expected: dict[str, object] = {
        "schema": RESULT_PROTOCOL,
        "provider": "runner-1",
        "nonce": "invocation-1",
        "policy_sha256": "policy-1",
        "catalogue_sha256": "catalogue-1",
        "material_sha256": "material-1",
        "complete": True,
        "outcomes": ["runner-1::test-001", "runner-1::test-002"],
    }
    valid = dict(expected)
    controls: dict[str, object] = {
        "missing": None,
        "stale": {**valid, "policy_sha256": "older-policy"},
        "malformed": {**valid, "outcomes": "not-an-array"},
        "incomplete": {**valid, "complete": False, "outcomes": ["runner-1::test-001"]},
        "replayed": {**valid, "nonce": "another-invocation"},
        "mismatched-material": {**valid, "material_sha256": "another-revision"},
    }
    result = {
        name: validate_reference_observation(value, expected)
        for name, value in controls.items()
    }
    if validate_reference_observation(valid, expected) != "current":
        raise RuntimeError("reference receipt control rejected its valid observation")
    if tuple(result) != NEGATIVE_CONTROLS or any(
        result[name] != name for name in NEGATIVE_CONTROLS
    ):
        raise RuntimeError(f"receipt negative-control corpus did not fail closed: {result!r}")
    return result


def yaml_array(values: list[str]) -> str:
    return json.dumps(values, ensure_ascii=True)


def write_corpus(root: Path, provider: Path, cold_ms: float) -> tuple[Path, list[str]]:
    plan = root / "plan" / "tasks"
    catalogues = root / "catalogues"
    plan.mkdir(parents=True)
    catalogues.mkdir()
    names: list[str] = []
    next_number = 1
    for runner, count in enumerate(RUNNER_TEST_COUNTS, 1):
        current = []
        for _ in range(count):
            name = f"runner-{runner}::test-{next_number:03d}"
            current.append(name)
            names.append(name)
            next_number += 1
        (catalogues / f"runner-{runner}.txt").write_text(
            "\n".join(current) + "\n", encoding="utf-8"
        )

    for number, name in enumerate(names, 1):
        cited = [name]
        if number <= REPEATED_CITATIONS:
            cited.append(names[-number])
        evidence = "".join(f"  - kind: test\n    ref: {citation}\n" for citation in cited)
        (plan / f"P-{number:03d}.md").write_text(
            "---\n"
            f"id: P-{number:03d}\n"
            f"title: Evidence benchmark task {number}\n"
            "phase: 1\n"
            "status: todo\n"
            "effort_days: 1\n"
            "evidence:\n"
            f"{evidence}"
            "---\n",
            encoding="utf-8",
        )

    events = root / "events.jsonl"
    state = root / "provider-state"
    runner_blocks: list[list[str]] = []
    for runner in range(1, len(RUNNER_TEST_COUNTS) + 1):
        common = [
            str(provider),
            "list",
            f"runner-{runner}",
            str(catalogues / f"runner-{runner}.txt"),
            f"events={events}",
            f"state={state}",
            f"{cold_ms:g}",
            "{slow_ms}",
            "{batch_ms}",
            "{fail_runner}",
        ]
        run = common.copy()
        run[1] = "run"
        run.append("{test}")
        batch = common.copy()
        batch[1] = "batch"
        batch.extend(["{manifest}", "{result}"])
        runner_blocks.append(
            [
                f"name: runner-{runner}",
                f"list: {yaml_array(common)}",
                f"run: {yaml_array(run)}",
                "batch:",
                "  protocol: 1",
                f"  run: {yaml_array(batch)}",
            ]
        )

    def template(*, batched: bool, concurrent: bool) -> str:
        lines = [
            "root: plan",
            "evidence:",
            "  observation_inputs: [.aplan.yml, provider.py, 'catalogues/**']",
            "test:",
        ]
        if concurrent:
            lines.extend(["  schedule: {{capacity: 4}}", "  runners:"])
            item_indent = "    "
            field_indent = "      "
        else:
            item_indent = "  "
            field_indent = "    "
        for block in runner_blocks:
            selected = block if batched else block[:3]
            lines.append(f"{item_indent}- {selected[0]}")
            lines.extend(f"{field_indent}{line}" for line in selected[1:])
        return "\n".join(lines) + "\n"

    templates = {
        "direct": template(batched=False, concurrent=False),
        "batched-serial": template(batched=True, concurrent=False),
        "bounded-concurrent": template(batched=True, concurrent=True),
    }
    config_path = root / ".aplan.yml"
    config_path.write_text(
        templates["direct"].format(
            slow_ms="0",
            batch_ms="0",
            fail_runner="none",
            test="{test}",
        ),
        encoding="utf-8",
    )
    for mode, value in templates.items():
        (root / f"config-{mode}.template").write_text(value, encoding="utf-8")
    return config_path, names


def configure(
    root: Path,
    slow_ms: float,
    fail_runner: str,
    mode: str = "direct",
    batch_ms: float = 0,
) -> Path:
    template = (root / f"config-{mode}.template").read_text(encoding="utf-8")
    config = root / ".aplan.yml"
    config.write_text(
        template.format(
            slow_ms=f"{slow_ms:g}",
            batch_ms=f"{batch_ms:g}",
            fail_runner=fail_runner,
            test="{test}",
            manifest="{manifest}",
            result="{result}",
        ),
        encoding="utf-8",
    )
    return config


def initialize_git(root: Path) -> None:
    (root / ".gitignore").write_text(
        "events.jsonl\nprovider-state/\nreceipts/\nreceipt-import.json\n/aplan\n",
        encoding="utf-8",
    )
    commands = [
        ["git", "init", "-q"],
        ["git", "add", "-A"],
        [
            "git",
            "-c",
            "user.name=aplan benchmark",
            "-c",
            "user.email=benchmark@invalid",
            "commit",
            "-q",
            "-m",
            "benchmark corpus",
        ],
    ]
    for command in commands:
        subprocess.run(
            command,
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=git_environment(),
        )


def clear_events(root: Path) -> None:
    (root / "events.jsonl").unlink(missing_ok=True)


def clear_warm_state(root: Path) -> None:
    shutil.rmtree(root / "provider-state", ignore_errors=True)


def prepare_warm_state(root: Path) -> None:
    """Put every synthetic provider in one explicit, reproducible warm state."""
    clear_warm_state(root)
    state = root / "provider-state"
    state.mkdir()
    for runner in range(1, len(RUNNER_TEST_COUNTS) + 1):
        (state / f"runner-{runner}.warm").write_text("warm\n", encoding="utf-8")


def events(root: Path) -> dict[str, int]:
    counts = {"catalogue": 0, "test": 0, "probe": 0, "batch": 0}
    path = root / "events.jsonl"
    if not path.exists():
        return counts
    for line in path.read_text(encoding="utf-8").splitlines():
        event = json.loads(line)
        kind = event["kind"]
        if kind not in counts:
            raise RuntimeError(f"unknown provider event: {event!r}")
        counts[kind] += 1
    return counts


def run_process(
    argv: list[str],
    cwd: Path,
    timeout: float,
    environment: dict[str, str] | None = None,
) -> tuple[int, bytes, bytes, float]:
    started = time.perf_counter_ns()
    process = subprocess.Popen(
        argv,
        cwd=cwd,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
        env={**(os.environ if environment is None else environment), "NO_COLOR": "1"},
    )
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
        raise RuntimeError(f"command exceeded {timeout:g}s: {' '.join(argv)}") from error
    wall_ms = (time.perf_counter_ns() - started) / 1_000_000
    return process.returncode, stdout, stderr, wall_ms


def percentile(values: list[float], fraction: float) -> float:
    ordered = sorted(values)
    return ordered[max(0, math.ceil(fraction * len(ordered)) - 1)]


def parse_receipt_timings(stderr: bytes) -> tuple[dict[str, float | int], bytes]:
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
            f"receipt-current emitted {diagnostics} timing diagnostics rather than one"
        )
    required = set(RECEIPT_PHASE_FIELDS) | {
        "receipt_accepted",
        "receipt_rejected",
        "receipt_test_provider_spawns",
    }
    missing = required - set(values)
    if missing:
        raise RuntimeError(f"receipt timing fields are missing: {sorted(missing)!r}")
    if any(not math.isfinite(float(value)) or value < 0 for value in values.values()):
        raise RuntimeError(f"timing diagnostic contains an invalid value: {values!r}")
    if (
        values["receipt_accepted"] != len(RUNNER_TEST_COUNTS)
        or values["receipt_rejected"] != 0
        or values["receipt_test_provider_spawns"] != 0
    ):
        raise RuntimeError(f"receipt timing work changed: {values!r}")
    return values, b"".join(retained)


def validate_identities(identities: list[dict[str, str]]) -> dict[str, str]:
    if not identities:
        raise RuntimeError("sample identity set is empty")
    for identity in identities:
        missing = REQUIRED_IDENTITY_FIELDS - set(identity)
        if missing:
            raise RuntimeError(f"sample identity is incomplete: missing {sorted(missing)!r}")
        malformed = sorted(
            field
            for field in REQUIRED_IDENTITY_FIELDS
            if not isinstance(identity[field], str)
            or DIGEST.fullmatch(identity[field]) is None
        )
        if malformed:
            raise RuntimeError(
                f"sample identity has malformed SHA-256 fields: {malformed!r}"
            )
    first = identities[0]
    if any(identity != first for identity in identities[1:]):
        raise RuntimeError("refusing mixed benchmark samples with different identities")
    return first


def case_identity(
    source_sha256: str,
    binary_sha256: str,
    runner_sha256: str,
    policy: bytes,
    scenario: str,
) -> dict[str, str]:
    protocol = {
        "corpus": corpus_description(),
        "scenario": scenario,
    }
    return {
        "source_sha256": source_sha256,
        "binary_sha256": binary_sha256,
        "protocol_sha256": sha256_bytes(stable_json(protocol)),
        "policy_sha256": sha256_bytes(policy),
        "runner_sha256": runner_sha256,
        "environment_sha256": environment_identity(),
    }


def verdict_bytes(stdout: bytes) -> bytes:
    """Return verdict bytes without observation identities or wall-clock time.

    Provider source, state, scope, completeness and the remaining observation
    fields are verdict semantics. Removing the whole observation would let two
    execution modes disagree about what they proved while this audit stayed
    green.
    """
    try:
        document = json.loads(stdout)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise RuntimeError("execution case did not emit one JSON verdict") from error
    if not isinstance(document, dict):
        raise RuntimeError("execution case JSON verdict is not an object")
    observation = document.get("evidence_observation")
    if isinstance(observation, dict):
        for field in (
            "identity",
            "resolution_identity",
            "observed_at_epoch_seconds",
        ):
            observation.pop(field, None)
    return stable_json(document)


def transport_neutral_verdict_bytes(stdout: bytes) -> bytes:
    """Return verdict bytes for receipt-versus-direct transport comparison.

    Unlike execution-mode parity, this comparison deliberately crosses the
    provider-source boundary. It removes that one provenance label while
    retaining the observation's state, scope, completeness and result fields.
    """
    document = json.loads(verdict_bytes(stdout))
    observation = document.get("evidence_observation")
    if isinstance(observation, dict):
        observation.pop("provider_source", None)
    return stable_json(document)


def validate_execution_parity(cases: list[dict[str, object]]) -> dict[str, object]:
    """Compare the three direct execution transports without provenance fields."""
    names = ("direct-replay", "batched-serial", "bounded-concurrent")
    selected = [
        case for case in cases if isinstance(case, dict) and case.get("name") in names
    ]
    indexed = {
        case.get("name"): case
        for case in selected
    }
    missing = [name for name in names if name not in indexed]
    if missing:
        raise RuntimeError(f"execution parity is missing cases: {missing!r}")
    if len(selected) != len(names):
        raise RuntimeError("execution parity contains a duplicate execution case")
    malformed = [
        name
        for name in names
        if not isinstance(indexed[name].get("exit_code"), int)
        or isinstance(indexed[name].get("exit_code"), bool)
        or not isinstance(indexed[name].get("verdict_sha256"), str)
        or DIGEST.fullmatch(indexed[name]["verdict_sha256"]) is None
    ]
    if malformed:
        raise RuntimeError(f"execution parity cases are malformed: {malformed!r}")
    shapes = {
        (
            indexed[name].get("exit_code"),
            indexed[name].get("verdict_sha256"),
        )
        for name in names
    }
    if len(shapes) != 1:
        detail = {
            name: {
                "exit_code": indexed[name].get("exit_code"),
                "verdict_sha256": indexed[name].get("verdict_sha256"),
            }
            for name in names
        }
        raise RuntimeError(f"execution modes disagreed after provenance removal: {detail!r}")
    exit_code, verdict_sha256 = shapes.pop()
    return {
        "cases": list(names),
        "exit_code": exit_code,
        "verdict_sha256": verdict_sha256,
    }


def measure_case(
    *,
    name: str,
    argv: list[str],
    root: Path,
    workspace: Path,
    binary: Path,
    source_sha256: str,
    binary_sha256: str,
    runner_sha256: str,
    runs: int,
    timeout: float,
    expected_success: bool,
    prepare: Callable[[], None],
    expected_events: dict[str, int],
    policy: bytes,
    policy_path: Path | None,
) -> dict[str, object]:
    samples = []
    identities = []
    for _ in range(runs):
        prepare()
        clear_events(root)
        observed_policy = policy_path.read_bytes() if policy_path is not None else policy
        if observed_policy != policy:
            raise RuntimeError(f"{name} started with an unexpected policy identity")
        observed_source = source_identity(workspace)
        if observed_source != source_sha256:
            raise RuntimeError(f"{name} started with an unexpected Git source identity")
        observed_binary = sha256_file(binary)
        if observed_binary != binary_sha256:
            raise RuntimeError(f"{name} started with an unexpected binary identity")
        observed_runner = runner_identity(root)
        if observed_runner != runner_sha256:
            raise RuntimeError(f"{name} started with an unexpected runner identity")
        identity = case_identity(
            observed_source, observed_binary, observed_runner, observed_policy, name
        )
        code, stdout, stderr, wall_ms = run_process(argv, root, timeout)
        timings = None
        stable_stderr = stderr
        if name == "receipt-current":
            timings, stable_stderr = parse_receipt_timings(stderr)
        identities.append(identity)
        observed = events(root)
        if observed != expected_events:
            raise RuntimeError(
                f"{name} changed deterministic work: expected {expected_events}, got {observed}"
            )
        succeeded = code == 0
        if succeeded != expected_success:
            raise RuntimeError(
                f"{name} exited {code}; stdout={stdout[:512]!r}; stderr={stderr[:512]!r}"
            )
        samples.append(
            {
                "wall_ms": wall_ms,
                "exit_code": code,
                "stdout_sha256": sha256_bytes(stdout),
                "verdict_sha256": sha256_bytes(verdict_bytes(stdout)),
                "stderr_sha256": sha256_bytes(stable_stderr),
                "events": observed,
                "timings": timings,
            }
        )
        final_policy = policy_path.read_bytes() if policy_path is not None else policy
        if final_policy != observed_policy:
            raise RuntimeError(f"{name} policy changed while its sample was running")
        if sha256_file(binary) != observed_binary:
            raise RuntimeError(f"{name} binary changed while its sample was running")
        if runner_identity(root) != observed_runner:
            raise RuntimeError(f"{name} runner changed while its sample was running")
        if source_identity(workspace) != observed_source:
            raise RuntimeError(f"{name} Git source changed while its sample was running")
    identity = validate_identities(identities)
    output_shapes = {
        (sample["exit_code"], sample["stdout_sha256"], sample["stderr_sha256"])
        for sample in samples
    }
    if len(output_shapes) != 1:
        raise RuntimeError(f"{name} output changed between benchmark samples")
    wall = [float(sample["wall_ms"]) for sample in samples]
    result = {
        "name": name,
        "identity": identity,
        "runs": runs,
        "wall_min_ms": min(wall),
        "wall_p50_ms": statistics.median(wall),
        "wall_p95_ms": percentile(wall, 0.95),
        "wall_max_ms": max(wall),
        "exit_code": samples[0]["exit_code"],
        "stdout_sha256": samples[0]["stdout_sha256"],
        "stderr_sha256": samples[0]["stderr_sha256"],
        "verdict_sha256": samples[0]["verdict_sha256"],
        "work": {
            "provider_process_spawns": sum(expected_events.values()),
            "catalogue_spawns": expected_events["catalogue"],
            "batch_spawns": expected_events["batch"],
            "selected_tests": (
                sum(RUNNER_TEST_COUNTS)
                if expected_events["batch"]
                else expected_events["test"]
            ),
            "miss_probes": expected_events["probe"],
            "citations_examined": sum(RUNNER_TEST_COUNTS) + REPEATED_CITATIONS,
            "receipt_imports": len(RUNNER_TEST_COUNTS) if name == "receipt-current" else 0,
            "imported_outcomes": (
                sum(RUNNER_TEST_COUNTS) if name == "receipt-current" else 0
            ),
        },
    }
    if name == "receipt-current":
        result["receipt_phase_p50_ms"] = {
            field.removesuffix("_ms"): statistics.median(
                float(sample["timings"][field]) for sample in samples
            )
            for field in RECEIPT_PHASE_FIELDS
        }
    return result


def configured_argv(binary: Path, execute: bool = False) -> list[str]:
    argv = [str(binary), "--config", ".aplan.yml", "--root", "plan", "verify"]
    if execute:
        argv.append("--execute")
    argv.extend(["--format", "json", "--color", "never"])
    return argv


def produce_receipt_set(
    binary: Path,
    root: Path,
    *,
    repository: str = "aplan/performance-corpus",
    workflow: str = "receipt-setup",
    run_id: str = "1",
    event: str = "benchmark",
    privilege: str = "trusted",
    environment: dict[str, str] | None = None,
) -> Path:
    receipts = root / "receipts"
    receipts.mkdir(exist_ok=True)
    entries = []
    for index in range(1, len(RUNNER_TEST_COUNTS) + 1):
        output = receipts / f"runner-{index}.json"
        argv = [
            str(binary),
            "--config",
            ".aplan.yml",
            "--root",
            "plan",
            "test",
            "receipt",
            f"runner-{index}",
            "--output",
            str(output),
            "--producer-repository",
            repository,
            "--producer-workflow",
            workflow,
            "--producer-run-id",
            run_id,
            "--producer-event",
            event,
            "--producer-privilege",
            privilege,
        ]
        code, stdout, stderr, _ = run_process(
            argv, root, 120.0, environment=environment
        )
        if code != 0 or stdout:
            raise RuntimeError(
                f"receipt setup for runner-{index} failed: code={code} "
                f"stdout={stdout[:512]!r} stderr={stderr[:512]!r}"
            )
        entries.append(
            {
                "provider": f"runner-{index}",
                "path": f"receipts/runner-{index}.json",
                "trust": {"mode": "local_file"},
            }
        )
    import_set = root / "receipt-import.json"
    import_set.write_bytes(
        stable_json(
            {
                "schema": "aplan/test-evidence-import-set/v1",
                "receipts": entries,
            }
        )
    )
    return import_set


def receipt_argv(binary: Path, import_set: Path) -> list[str]:
    return [
        str(binary),
        "--timings",
        "--config",
        ".aplan.yml",
        "--root",
        "plan",
        "verify",
        "--test-receipts",
        str(import_set),
        "--format",
        "json",
        "--color",
        "never",
    ]


def benchmark(args: argparse.Namespace) -> dict[str, object]:
    workspace = Path.cwd().resolve()
    binary = args.binary.resolve()
    if not binary.is_file():
        raise RuntimeError(f"benchmark binary does not exist: {binary}")
    execution_cases = {"direct-replay", "batched-serial", "bounded-concurrent"}
    if args.case and execution_cases.intersection(args.case) and not args.include_direct:
        raise RuntimeError(
            "direct-replay, batched-serial and bounded-concurrent require --include-direct"
        )
    scratch = args.scratch_root.expanduser().resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    source_before = source_identity(workspace)
    binary_before = sha256_file(binary)
    protocol_sha256 = sha256_bytes(stable_json(corpus_description()))

    with tempfile.TemporaryDirectory(prefix="aplan-evidence-gate-", dir=scratch) as temporary:
        root = Path(temporary)
        provider = root / "provider.py"
        provider.write_text(PROVIDER_SOURCE, encoding="utf-8")
        provider.chmod(0o755)
        config, _ = write_corpus(root, provider, args.cold_delay_ms)
        initialize_git(root)
        runner_sha256 = runner_identity(root)
        zero = {"catalogue": 0, "test": 0, "probe": 0, "batch": 0}
        catalogue = {**zero, "catalogue": len(RUNNER_TEST_COUNTS)}
        direct = {
            "catalogue": len(RUNNER_TEST_COUNTS),
            "test": sum(RUNNER_TEST_COUNTS),
            "probe": len(RUNNER_TEST_COUNTS),
            "batch": 0,
        }
        batched = {
            "catalogue": len(RUNNER_TEST_COUNTS),
            "test": 0,
            "probe": 0,
            "batch": len(RUNNER_TEST_COUNTS),
        }
        cases = []

        def wanted(name: str) -> bool:
            return not args.case or name in args.case

        configure(root, 0, "none")
        policy = config.read_bytes()
        receipt_import = None
        receipt_policy = None
        if wanted("receipt-current"):
            receipt_policy = configure(
                root, 0, "none", mode="batched-serial", batch_ms=0
            ).read_bytes()
            receipt_import = produce_receipt_set(binary, root)
            clear_events(root)
        if wanted("structural-check"):
            cases.append(
                measure_case(
                    name="structural-check",
                    argv=[
                        str(binary),
                        "--config",
                        ".aplan.yml",
                        "--root",
                        "plan",
                        "check",
                        "--format",
                        "json",
                        "--color",
                        "never",
                    ],
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=True,
                    prepare=lambda: configure(root, 0, "none"),
                    expected_events=zero,
                    policy=policy,
                    policy_path=config,
                )
            )
        if wanted("no-provider"):
            cases.append(
                measure_case(
                    name="no-provider",
                    argv=[
                        str(binary),
                        "--no-config",
                        "--root",
                        "plan",
                        "verify",
                        "--format",
                        "json",
                        "--color",
                        "never",
                    ],
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=True,
                    prepare=lambda: None,
                    expected_events=zero,
                    policy=b"no-config\n",
                    policy_path=None,
                )
            )
        if wanted("catalogue-cold"):
            cases.append(
                measure_case(
                    name="catalogue-cold",
                    argv=configured_argv(binary),
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=True,
                    prepare=lambda: (clear_warm_state(root), configure(root, 0, "none")),
                    expected_events=catalogue,
                    policy=policy,
                    policy_path=config,
                )
            )
        if wanted("catalogue-warm"):
            cases.append(
                measure_case(
                    name="catalogue-warm",
                    argv=configured_argv(binary),
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=True,
                    prepare=lambda: (prepare_warm_state(root), configure(root, 0, "none")),
                    expected_events=catalogue,
                    policy=policy,
                    policy_path=config,
                )
            )
        if wanted("slow-runner"):
            slow_config = configure(root, args.slow_delay_ms, "none").read_bytes()
            cases.append(
                measure_case(
                    name="slow-runner",
                    argv=configured_argv(binary),
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=True,
                    prepare=lambda: (
                        prepare_warm_state(root),
                        configure(root, args.slow_delay_ms, "none"),
                    ),
                    expected_events=catalogue,
                    policy=slow_config,
                    policy_path=config,
                )
            )
        if wanted("failing-runner"):
            failed_config = configure(root, 0, "runner-8").read_bytes()
            cases.append(
                measure_case(
                    name="failing-runner",
                    argv=configured_argv(binary),
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=False,
                    prepare=lambda: (
                        prepare_warm_state(root),
                        configure(root, 0, "runner-8"),
                    ),
                    expected_events=catalogue,
                    policy=failed_config,
                    policy_path=config,
                )
            )
        if wanted("receipt-current"):
            if receipt_import is None or receipt_policy is None:
                raise RuntimeError("receipt-current setup did not produce an import set")
            cases.append(
                measure_case(
                    name="receipt-current",
                    argv=receipt_argv(binary, receipt_import),
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=True,
                    prepare=lambda: configure(
                        root, 0, "none", mode="batched-serial", batch_ms=0
                    ),
                    expected_events=zero,
                    policy=receipt_policy,
                    policy_path=config,
                )
            )
        if args.include_direct and wanted("direct-replay"):
            direct_config = configure(root, 0, "none").read_bytes()
            cases.append(
                measure_case(
                    name="direct-replay",
                    argv=configured_argv(binary, execute=True),
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=True,
                    prepare=lambda: (prepare_warm_state(root), configure(root, 0, "none")),
                    expected_events=direct,
                    policy=direct_config,
                    policy_path=config,
                )
            )
        if args.include_direct and wanted("batched-serial"):
            batched_serial_config = configure(
                root,
                0,
                "none",
                "batched-serial",
                args.batch_delay_ms,
            ).read_bytes()
            cases.append(
                measure_case(
                    name="batched-serial",
                    argv=configured_argv(binary, execute=True),
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=True,
                    prepare=lambda: (
                        prepare_warm_state(root),
                        configure(
                            root,
                            0,
                            "none",
                            "batched-serial",
                            args.batch_delay_ms,
                        ),
                    ),
                    expected_events=batched,
                    policy=batched_serial_config,
                    policy_path=config,
                )
            )
        if args.include_direct and wanted("bounded-concurrent"):
            concurrent_config = configure(
                root,
                0,
                "none",
                "bounded-concurrent",
                args.batch_delay_ms,
            ).read_bytes()
            cases.append(
                measure_case(
                    name="bounded-concurrent",
                    argv=configured_argv(binary, execute=True),
                    root=root,
                    workspace=workspace,
                    binary=binary,
                    source_sha256=source_before,
                    binary_sha256=binary_before,
                    runner_sha256=runner_sha256,
                    runs=args.runs,
                    timeout=args.timeout_seconds,
                    expected_success=True,
                    prepare=lambda: (
                        prepare_warm_state(root),
                        configure(
                            root,
                            0,
                            "none",
                            "bounded-concurrent",
                            args.batch_delay_ms,
                        ),
                    ),
                    expected_events=batched,
                    policy=concurrent_config,
                    policy_path=config,
                )
            )

    if sha256_file(binary) != binary_before:
        raise RuntimeError("benchmark binary changed while samples were running")
    if source_identity(workspace) != source_before:
        raise RuntimeError("Git source state changed while samples were running")
    execution_names = {
        case["name"] for case in cases if case["name"] in execution_cases
    }
    execution_parity = None
    if execution_names == execution_cases:
        execution_parity = validate_execution_parity(cases)
    return {
        "profile": args.profile,
        "recorded_at": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "corpus": corpus_description(),
        "protocol_sha256": protocol_sha256,
        "source_sha256": source_before,
        "binary_sha256": binary_before,
        "cases": cases,
        "execution_parity": execution_parity,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--describe", action="store_true")
    parser.add_argument("--check-receipt-negative-controls", action="store_true")
    parser.add_argument("--validate-sample-identities", type=Path)
    parser.add_argument("--validate-execution-parity", type=Path)
    parser.add_argument("--canonicalize-verdict", type=Path)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--profile", default="local")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--cold-delay-ms", type=float, default=50.0)
    parser.add_argument("--slow-delay-ms", type=float, default=250.0)
    parser.add_argument("--batch-delay-ms", type=float, default=100.0)
    parser.add_argument("--timeout-seconds", type=float, default=120.0)
    parser.add_argument(
        "--include-direct",
        action="store_true",
        help="include legacy, batched-serial and bounded-concurrent execution cases",
    )
    parser.add_argument(
        "--case",
        action="append",
        choices=(
            "structural-check",
            "no-provider",
            "catalogue-cold",
            "catalogue-warm",
            "slow-runner",
            "failing-runner",
            "direct-replay",
            "batched-serial",
            "bounded-concurrent",
            "receipt-current",
        ),
        help="measure only this case; repeat for more than one",
    )
    parser.add_argument("--scratch-root", type=Path, default=Path("~/.tmp"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.describe:
        print(json.dumps(corpus_description(), indent=2, sort_keys=True))
        return 0
    if args.check_receipt_negative_controls:
        print(json.dumps(exercise_receipt_negative_controls(), indent=2, sort_keys=True))
        return 0
    if args.validate_sample_identities:
        value = json.loads(args.validate_sample_identities.read_text(encoding="utf-8"))
        if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
            raise RuntimeError("sample identity input must be an array of objects")
        validate_identities(value)
        return 0
    if args.validate_execution_parity:
        value = json.loads(args.validate_execution_parity.read_text(encoding="utf-8"))
        if isinstance(value, dict):
            value = value.get("cases")
        if not isinstance(value, list) or not all(isinstance(item, dict) for item in value):
            raise RuntimeError("execution parity input must be a report or array of cases")
        validate_execution_parity(value)
        return 0
    if args.canonicalize_verdict:
        sys.stdout.buffer.write(verdict_bytes(args.canonicalize_verdict.read_bytes()))
        return 0
    if args.binary is None:
        parser.error(
            "--binary is required unless a describe, receipt-control, identity-validation, "
            "execution-parity-validation, or verdict-canonicalization "
            "operation is used"
        )
    if (
        args.runs < 1
        or not math.isfinite(args.timeout_seconds)
        or args.timeout_seconds <= 0
        or not math.isfinite(args.cold_delay_ms)
        or args.cold_delay_ms < 0
        or not math.isfinite(args.slow_delay_ms)
        or args.slow_delay_ms < 0
        or not math.isfinite(args.batch_delay_ms)
        or args.batch_delay_ms < 0
    ):
        parser.error("runs and timeout must be positive; fixture delays must be finite and non-negative")
    report = benchmark(args)
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(encoded, encoding="utf-8")
    else:
        sys.stdout.write(encoded)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"evidence-gate-performance: {error}", file=sys.stderr)
        raise SystemExit(1) from error
