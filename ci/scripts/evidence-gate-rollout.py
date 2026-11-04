#!/usr/bin/env python3
"""Produce and consume the scheduled evidence-rollout audit artifact.

The producer records a direct full oracle and portable receipts from one exact
fixture. The dependent consumer validates the transport, reconstructs
same-workflow trust from its own workflow context, and compares the receipt or
rollback route with that oracle. No artifact bytes are trusted merely because
they arrived through the artifact service.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import os
import shutil
import statistics
import sys
import tarfile
import tempfile
import time
from pathlib import Path


sys.dont_write_bytecode = True

SCHEMA = "aplan/evidence-rollout-bundle/v1"
MAX_MEMBERS = 8192
MAX_MANIFEST_BYTES = 1024 * 1024
MAX_ARCHIVE_BYTES = 500 * 1024 * 1024
MAX_EXPANDED_BYTES = 750 * 1024 * 1024


def load_performance_module():
    path = Path(__file__).with_name("evidence-gate-performance.py")
    spec = importlib.util.spec_from_file_location("aplan_evidence_performance", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load performance harness: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PERF = load_performance_module()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def rollout_environment() -> dict[str, str]:
    """Return the complete portable provider environment admitted by the audit."""
    return {
        "PATH": "/usr/local/bin:/usr/bin:/bin",
        "LC_ALL": "C",
        "TZ": "UTC",
        "NO_COLOR": "1",
    }


def make_runner_paths_portable(root: Path) -> None:
    prefix = f"{root}/"
    for path in [root / ".aplan.yml", *sorted(root.glob("config-*.template"))]:
        text = path.read_text(encoding="utf-8")
        if prefix not in text:
            raise RuntimeError(f"runner template has no producer root to relativize: {path}")
        path.write_text(text.replace(prefix, ""), encoding="utf-8")


def producer(args: argparse.Namespace) -> dict[str, object]:
    workspace = Path.cwd().resolve()
    source_sha256 = PERF.source_identity(workspace)
    binary = args.binary.resolve()
    if not binary.is_file():
        raise RuntimeError(f"aplan binary does not exist: {binary}")
    output = args.output.resolve()
    if output.exists():
        raise RuntimeError(f"refusing to replace rollout bundle: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    scratch = args.scratch_root.expanduser().resolve()
    scratch.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory(prefix="aplan-rollout-producer-", dir=scratch) as temporary:
        temporary_path = Path(temporary)
        root = temporary_path / "fixture"
        root.mkdir()
        provider = root / "provider.py"
        provider.write_text(PERF.PROVIDER_SOURCE, encoding="utf-8")
        provider.chmod(0o755)
        # Keep the command relocatable while retaining a path separator, so it
        # resolves from the configured runner directory rather than through PATH.
        PERF.write_corpus(root, "./provider.py", 0)
        make_runner_paths_portable(root)
        PERF.initialize_git(root)
        policy = PERF.configure(root, 0, "none", "batched-serial", 0).read_bytes()
        fixture_binary = root / "aplan"
        shutil.copy2(binary, fixture_binary)
        fixture_binary.chmod(0o755)

        PERF.clear_events(root)
        code, stdout, stderr, direct_ms = PERF.run_process(
            PERF.configured_argv(fixture_binary, execute=True),
            root,
            args.timeout_seconds,
            environment=rollout_environment(),
        )
        expected_events = {
            "catalogue": len(PERF.RUNNER_TEST_COUNTS),
            "test": 0,
            "probe": 0,
            "batch": len(PERF.RUNNER_TEST_COUNTS),
        }
        observed_events = PERF.events(root)
        if code != 0 or observed_events != expected_events:
            raise RuntimeError(
                "direct rollout oracle failed: "
                f"exit={code} events={observed_events!r} stderr={stderr[:512]!r}"
            )
        direct_verdict_sha256 = PERF.sha256_bytes(
            PERF.transport_neutral_verdict_bytes(stdout)
        )

        PERF.produce_receipt_set(
            fixture_binary,
            root,
            repository=args.repository,
            workflow=args.workflow,
            run_id=args.run_id,
            event=args.event,
            privilege=args.privilege,
            environment=rollout_environment(),
        )
        PERF.clear_events(root)
        shutil.rmtree(root / "provider-state", ignore_errors=True)
        (root / "receipt-import.json").unlink(missing_ok=True)

        payload = temporary_path / "payload.tar.gz"
        with tarfile.open(payload, "w:gz") as archive:
            archive.add(root, arcname="fixture", recursive=True)

        staging = Path(
            tempfile.mkdtemp(prefix=f".{output.name}.", dir=output.parent)
        )
        try:
            staged_payload = staging / "payload.tar.gz"
            shutil.copy2(payload, staged_payload)
            manifest = {
                "schema": SCHEMA,
                "producer": {
                    "repository": args.repository,
                    "workflow": args.workflow,
                    "run_id": args.run_id,
                    "event": args.event,
                    "privilege": args.privilege,
                },
                "payload_sha256": sha256_file(staged_payload),
                "binary_sha256": sha256_file(fixture_binary),
                "source_sha256": source_sha256,
                "fixture_source_sha256": PERF.source_identity(root),
                "protocol_sha256": PERF.sha256_bytes(
                    PERF.stable_json(PERF.corpus_description())
                ),
                "policy_sha256": PERF.sha256_bytes(policy),
                "runner_sha256": PERF.runner_identity(root),
                "environment_sha256": PERF.environment_identity(),
                "provider_environment_sha256": PERF.sha256_bytes(
                    PERF.stable_json(rollout_environment())
                ),
                "direct": {
                    "exit_code": code,
                    "verdict_sha256": direct_verdict_sha256,
                    "wall_ms": direct_ms,
                    "events": observed_events,
                },
                "receipts": len(PERF.RUNNER_TEST_COUNTS),
                "outcomes": sum(PERF.RUNNER_TEST_COUNTS),
            }
            write_json(staging / "manifest.json", manifest)
            staging.rename(output)
        except BaseException:
            shutil.rmtree(staging, ignore_errors=True)
            raise
    return manifest


def safe_extract(archive_path: Path, destination: Path) -> None:
    with tarfile.open(archive_path, "r:gz") as archive:
        members = archive.getmembers()
        if len(members) > MAX_MEMBERS:
            raise RuntimeError(f"rollout payload has too many members: {len(members)}")
        expanded = 0
        names: dict[str, str] = {}
        for member in members:
            path = Path(member.name)
            if path.is_absolute() or ".." in path.parts:
                raise RuntimeError(f"rollout payload has unsafe path: {member.name!r}")
            normalized = path.as_posix()
            if not path.parts or path.parts[0] != "fixture":
                raise RuntimeError(f"rollout payload escapes its fixture root: {member.name!r}")
            if normalized in names:
                raise RuntimeError(f"rollout payload repeats a path: {member.name!r}")
            if member.issym() or member.islnk() or member.isdev() or member.isfifo():
                raise RuntimeError(f"rollout payload has unsafe member: {member.name!r}")
            if not (member.isdir() or member.isfile()):
                raise RuntimeError(f"rollout payload has unsupported member: {member.name!r}")
            names[normalized] = "directory" if member.isdir() else "file"
            expanded += member.size
            if expanded > MAX_EXPANDED_BYTES:
                raise RuntimeError("rollout payload exceeds the expanded-size limit")
        for normalized, kind in names.items():
            parts = Path(normalized).parts
            for depth in range(1, len(parts)):
                ancestor = Path(*parts[:depth]).as_posix()
                if names.get(ancestor) == "file":
                    raise RuntimeError(
                        "rollout payload places a member below a file: "
                        f"{ancestor!r} contains {normalized!r}"
                    )
        # Do not use TarFile.extractall even after the checks above. Its safe
        # extraction filters differ across supported Python releases, while
        # legacy extraction would reintroduce host metadata and
        # path handling that this bounded format does not need. Materialize
        # only the two admitted member kinds with normalized permissions.
        destination.mkdir(parents=True, exist_ok=True)
        for member in members:
            path = destination.joinpath(*Path(member.name).parts)
            if member.isdir():
                path.mkdir(parents=True, exist_ok=True, mode=0o755)
                continue
            path.parent.mkdir(parents=True, exist_ok=True, mode=0o755)
            source = archive.extractfile(member)
            if source is None:
                raise RuntimeError(f"rollout payload cannot read member: {member.name!r}")
            with source, path.open("xb") as target:
                shutil.copyfileobj(source, target, length=1024 * 1024)
            if path.stat().st_size != member.size:
                raise RuntimeError(f"rollout payload member changed size: {member.name!r}")
            path.chmod(0o755 if member.mode & 0o111 else 0o644)


def manifest_digest(label: str, manifest: dict[str, object]) -> str:
    expected = manifest.get(label)
    if not isinstance(expected, str) or PERF.DIGEST.fullmatch(expected) is None:
        raise RuntimeError(f"rollout manifest has invalid {label}")
    return expected


def expect_digest(label: str, actual: str, manifest: dict[str, object]) -> None:
    expected = manifest_digest(label, manifest)
    if actual != expected:
        raise RuntimeError(f"rollout {label} mismatch: expected {expected}, got {actual}")


def consumer(args: argparse.Namespace) -> dict[str, object]:
    workspace = Path.cwd().resolve()
    bundle = args.bundle.resolve()
    manifest_path = bundle / "manifest.json"
    payload = bundle / "payload.tar.gz"
    if not manifest_path.is_file() or not payload.is_file():
        raise RuntimeError("rollout bundle must contain manifest.json and payload.tar.gz")
    entries = sorted(path.name for path in bundle.iterdir())
    if entries != ["manifest.json", "payload.tar.gz"]:
        raise RuntimeError(f"rollout bundle has unexpected entries: {entries!r}")
    if manifest_path.stat().st_size > MAX_MANIFEST_BYTES:
        raise RuntimeError("rollout manifest exceeds the byte limit")
    if payload.stat().st_size > MAX_ARCHIVE_BYTES:
        raise RuntimeError("rollout payload exceeds the archive byte limit")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if not isinstance(manifest, dict) or manifest.get("schema") != SCHEMA:
        raise RuntimeError("rollout manifest has an unsupported schema")
    producer_identity = {
        "repository": args.repository,
        "workflow": args.workflow,
        "run_id": args.run_id,
        "event": args.event,
        "privilege": args.privilege,
    }
    if manifest.get("producer") != producer_identity:
        raise RuntimeError(
            "rollout producer context does not match the authenticated consumer context"
        )
    direct = manifest.get("direct")
    if not isinstance(direct, dict):
        raise RuntimeError("rollout manifest has no direct oracle")
    expected_exit = direct.get("exit_code")
    if not isinstance(expected_exit, int) or isinstance(expected_exit, bool):
        raise RuntimeError("rollout manifest has an invalid direct exit code")
    expected_verdict = direct.get("verdict_sha256")
    if (
        not isinstance(expected_verdict, str)
        or PERF.DIGEST.fullmatch(expected_verdict) is None
    ):
        raise RuntimeError("rollout manifest has an invalid direct verdict_sha256")
    producer_environment_sha256 = manifest_digest("environment_sha256", manifest)
    expect_digest("payload_sha256", sha256_file(payload), manifest)
    expect_digest("source_sha256", PERF.source_identity(workspace), manifest)
    expect_digest(
        "provider_environment_sha256",
        PERF.sha256_bytes(PERF.stable_json(rollout_environment())),
        manifest,
    )

    scratch = args.scratch_root.expanduser().resolve()
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="aplan-rollout-consumer-", dir=scratch) as temporary:
        extracted = Path(temporary)
        safe_extract(payload, extracted)
        root = extracted / "fixture"
        binary = root / "aplan"
        if not binary.is_file():
            raise RuntimeError("rollout payload has no aplan binary")
        expect_digest("binary_sha256", sha256_file(binary), manifest)
        expect_digest("fixture_source_sha256", PERF.source_identity(root), manifest)
        expect_digest("runner_sha256", PERF.runner_identity(root), manifest)
        expect_digest(
            "policy_sha256", PERF.sha256_bytes((root / ".aplan.yml").read_bytes()), manifest
        )
        expect_digest(
            "protocol_sha256",
            PERF.sha256_bytes(PERF.stable_json(PERF.corpus_description())),
            manifest,
        )

        receipts = []
        receipt_count = manifest.get("receipts")
        if not isinstance(receipt_count, int) or isinstance(receipt_count, bool):
            raise RuntimeError("rollout manifest has an invalid receipt count")
        if receipt_count != len(PERF.RUNNER_TEST_COUNTS):
            raise RuntimeError("rollout manifest has the wrong receipt count")
        for index in range(1, receipt_count + 1):
            receipt = root / "receipts" / f"runner-{index}.json"
            if not receipt.is_file():
                raise RuntimeError(f"rollout payload is missing {receipt.name}")
            receipts.append(
                {
                    "provider": f"runner-{index}",
                    "path": f"receipts/{receipt.name}",
                    "trust": {
                        "mode": "same_workflow",
                        "artifact_sha256": sha256_file(receipt),
                        "producer": producer_identity,
                    },
                }
            )
        import_set = root / "receipt-import.json"
        import_set.write_bytes(
            PERF.stable_json(
                {"schema": "aplan/test-evidence-import-set/v1", "receipts": receipts}
            )
        )

        samples = []
        phase_samples = []
        expected_events = (
            {"catalogue": 0, "test": 0, "probe": 0, "batch": 0}
            if args.route == "receipt"
            else {
                "catalogue": len(PERF.RUNNER_TEST_COUNTS),
                "test": 0,
                "probe": 0,
                "batch": len(PERF.RUNNER_TEST_COUNTS),
            }
        )
        for _ in range(args.runs):
            PERF.clear_events(root)
            argv = (
                PERF.receipt_argv(binary, import_set)
                if args.route == "receipt"
                else PERF.configured_argv(binary, execute=True)
            )
            code, stdout, stderr, wall_ms = PERF.run_process(
                argv,
                root,
                args.timeout_seconds,
                environment=rollout_environment(),
            )
            observed = PERF.events(root)
            if observed != expected_events:
                raise RuntimeError(
                    f"{args.route} route changed deterministic work: {observed!r}"
                )
            if code != expected_exit:
                raise RuntimeError(
                    f"{args.route} route exited {code}, direct oracle exited {expected_exit}; "
                    f"stdout={stdout[:1024]!r} stderr={stderr[:1024]!r}"
                )
            verdict = PERF.sha256_bytes(PERF.transport_neutral_verdict_bytes(stdout))
            if verdict != expected_verdict:
                raise RuntimeError(
                    f"{args.route} route disagreed with direct oracle: {verdict}"
                )
            if args.route == "receipt":
                phases, retained = PERF.parse_receipt_timings(stderr)
                expected_summary = (
                    f"evidence receipts: {len(PERF.RUNNER_TEST_COUNTS)} accepted, 0 rejected, "
                    "0 test-provider spawns; fallback=current-only\n"
                ).encode("utf-8")
                if retained != expected_summary:
                    raise RuntimeError(
                        f"receipt route emitted unexpected stderr: {retained[:512]!r}"
                    )
                phase_samples.append(phases)
            samples.append(wall_ms)

        p95 = PERF.percentile(samples, 0.95)
        completed_epoch_ms = time.time_ns() // 1_000_000
        post_setup_ms = (
            completed_epoch_ms - args.gate_start_epoch_ms
            if args.gate_start_epoch_ms is not None
            else None
        )
        if args.route == "receipt" and p95 >= 10_000:
            raise RuntimeError(f"artifact-available receipt p95 missed 10s: {p95:.3f}ms")
        if post_setup_ms is not None and post_setup_ms >= 60_000:
            raise RuntimeError(f"post-setup evidence gate missed 60s: {post_setup_ms}ms")
        report: dict[str, object] = {
            "schema": "aplan/evidence-rollout-result/v1",
            "route": args.route,
            "runs": args.runs,
            "wall_p50_ms": statistics.median(samples),
            "wall_p95_ms": p95,
            "wall_max_ms": max(samples),
            "post_setup_ms": post_setup_ms,
            "provider_process_spawns_per_run": sum(expected_events.values()),
            "verdict_sha256": expected_verdict,
            "direct_oracle_ms": direct.get("wall_ms"),
            "source_sha256": manifest["source_sha256"],
            "binary_sha256": manifest["binary_sha256"],
            "protocol_sha256": manifest["protocol_sha256"],
            "policy_sha256": manifest["policy_sha256"],
            "runner_sha256": manifest["runner_sha256"],
            "producer_environment_sha256": producer_environment_sha256,
            "consumer_environment_sha256": PERF.environment_identity(),
            "provider_environment_sha256": manifest["provider_environment_sha256"],
        }
        if phase_samples:
            report["receipt_phase_p50_ms"] = {
                field.removesuffix("_ms"): statistics.median(
                    float(sample[field]) for sample in phase_samples
                )
                for field in PERF.RECEIPT_PHASE_FIELDS
            }
        return report


def finite_positive(value: float, name: str) -> None:
    if not math.isfinite(value) or value <= 0:
        raise RuntimeError(f"{name} must be finite and positive")


def main() -> int:
    parser = argparse.ArgumentParser()
    subparsers = parser.add_subparsers(dest="operation", required=True)

    produce = subparsers.add_parser("produce")
    produce.add_argument("--binary", type=Path, required=True)
    produce.add_argument("--output", type=Path, required=True)
    produce.add_argument("--repository", required=True)
    produce.add_argument("--workflow", required=True)
    produce.add_argument("--run-id", required=True)
    produce.add_argument("--event", required=True)
    produce.add_argument("--privilege", required=True)
    produce.add_argument("--scratch-root", type=Path, default=Path("~/.tmp"))
    produce.add_argument("--timeout-seconds", type=float, default=120.0)

    consume = subparsers.add_parser("consume")
    consume.add_argument("--bundle", type=Path, required=True)
    consume.add_argument("--route", choices=("receipt", "direct"), required=True)
    consume.add_argument("--repository", required=True)
    consume.add_argument("--workflow", required=True)
    consume.add_argument("--run-id", required=True)
    consume.add_argument("--event", required=True)
    consume.add_argument("--privilege", required=True)
    consume.add_argument("--runs", type=int, default=5)
    consume.add_argument("--gate-start-epoch-ms", type=int)
    consume.add_argument("--scratch-root", type=Path, default=Path("~/.tmp"))
    consume.add_argument("--timeout-seconds", type=float, default=120.0)
    consume.add_argument("--output", type=Path)

    args = parser.parse_args()
    finite_positive(args.timeout_seconds, "timeout-seconds")
    if args.operation == "produce":
        report = producer(args)
    else:
        if args.runs < 1:
            raise RuntimeError("runs must be positive")
        report = consumer(args)
    encoded = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if getattr(args, "output", None) and args.operation == "consume":
        args.output.write_text(encoded, encoding="utf-8")
    else:
        sys.stdout.write(encoded)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, ValueError, tarfile.TarError) as error:
        print(f"aplan evidence rollout: {error}", file=sys.stderr)
        raise SystemExit(1)
