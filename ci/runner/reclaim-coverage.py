#!/usr/bin/env python3
"""Reclaim only coverage allocations whose complete recorded lease is gone."""

from __future__ import annotations

import json
import os
import re
import shutil
import stat
import sys
from pathlib import Path


NAME = re.compile(r"^\.aplan-coverage-([1-9][0-9]*)-(0|[1-9][0-9]*)$")
LEASE = re.compile(r"^[A-Za-z0-9_.-]{1,256}$")
BOOT_ID = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
)
OWNER = ".aplan-coverage-owner-v1.json"
LEASE_ENV = b"APLAN_COVERAGE_LEASE="
MAX_OWNER_BYTES = 16 * 1024
U32_MAX = 2**32 - 1
U64_MAX = 2**64 - 1


class Unverifiable(Exception):
    """The safe answer is to retain this allocation."""


class MissingOwner(Unverifiable):
    """A pre-owner-record allocation needs an explicitly quiescent migration."""


def boot_id() -> str:
    value = Path("/proc/sys/kernel/random/boot_id").read_text(encoding="ascii").strip()
    if not valid_boot_id(value):
        raise Unverifiable("malformed boot identity")
    return value


def valid_boot_id(value: object) -> bool:
    return isinstance(value, str) and BOOT_ID.fullmatch(value) is not None


def stat_fields(value: str) -> tuple[int, int]:
    try:
        fields = value.rsplit(") ", 1)[1].split()
        return int(fields[19]), int(fields[2])
    except (IndexError, ValueError) as error:
        raise Unverifiable("malformed /proc stat") from error


def real_uid(value: str) -> int:
    for line in value.splitlines():
        if line.startswith("Uid:"):
            try:
                return int(line.split()[1])
            except (IndexError, ValueError) as error:
                raise Unverifiable("malformed /proc Uid") from error
    raise Unverifiable("missing /proc Uid")


def process_details(pid: int, current_boot: str) -> tuple[dict[str, object], int] | None:
    directory = Path("/proc") / str(pid)
    try:
        process_stat = (directory / "stat").read_text(encoding="utf-8")
        process_status = (directory / "status").read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    except OSError as error:
        raise Unverifiable(f"cannot inspect process {pid}") from error
    start, group = stat_fields(process_stat)
    return (
        {
            "pid": pid,
            "boot_id": current_boot,
            "start_ticks": start,
            "uid": real_uid(process_status),
        },
        group,
    )


def exact_integer(value: object, *, positive: bool = False) -> bool:
    return type(value) is int and (not positive or value > 0)


def strict_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    value: dict[str, object] = {}
    for key, member in pairs:
        if key in value:
            raise Unverifiable(f"duplicate owner-record field: {key}")
        value[key] = member
    return value


def identity(value: object) -> dict[str, object]:
    if not isinstance(value, dict) or set(value) != {"pid", "boot_id", "start_ticks", "uid"}:
        raise Unverifiable("malformed process identity")
    if not exact_integer(value["pid"], positive=True) or value["pid"] > U32_MAX:
        raise Unverifiable("malformed process PID")
    if (
        not exact_integer(value["start_ticks"], positive=True)
        or value["start_ticks"] > U64_MAX
    ):
        raise Unverifiable("malformed process start identity")
    if not exact_integer(value["uid"]) or not 0 <= value["uid"] <= U32_MAX:
        raise Unverifiable("malformed process UID")
    if not valid_boot_id(value["boot_id"]):
        raise Unverifiable("malformed process boot identity")
    return value


def owner_record(directory: Path, name_pid: int) -> dict[str, object]:
    path = directory / OWNER
    try:
        metadata = path.lstat()
        if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
            raise Unverifiable("owner record is not a real file")
        if metadata.st_size > MAX_OWNER_BYTES:
            raise Unverifiable("owner record is too large")
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(descriptor, "rb") as stream:
            opened = os.fstat(stream.fileno())
            if (metadata.st_dev, metadata.st_ino) != (opened.st_dev, opened.st_ino):
                raise Unverifiable("owner record changed while it was opened")
            if opened.st_size > MAX_OWNER_BYTES:
                raise Unverifiable("owner record grew before it was opened")
            encoded = stream.read(MAX_OWNER_BYTES + 1)
            if len(encoded) > MAX_OWNER_BYTES:
                raise Unverifiable("owner record grew while it was read")
            record = json.loads(encoded, object_pairs_hook=strict_object)
    except FileNotFoundError as error:
        raise MissingOwner("owner record is missing") from error
    except (OSError, UnicodeError, ValueError, RecursionError) as error:
        raise Unverifiable("owner record cannot be read") from error

    expected = {"version", "phase", "owner", "child", "process_group", "lease"}
    if (
        not isinstance(record, dict)
        or set(record) != expected
        or not exact_integer(record["version"])
        or record["version"] != 1
    ):
        raise Unverifiable("unknown owner record")
    owner = identity(record["owner"])
    if owner["pid"] != name_pid:
        raise Unverifiable("directory and owner PID disagree")
    child = None if record["child"] is None else identity(record["child"])
    group = record["process_group"]
    if group is not None and (
        not exact_integer(group, positive=True) or group > U32_MAX
    ):
        raise Unverifiable("malformed process group")
    phase = record["phase"]
    if phase == "prepared" and (child is not None or group is not None):
        raise Unverifiable("prepared record contains a child")
    if phase == "running" and (child is None or group is None):
        raise Unverifiable("running record has no child")
    if phase not in ("prepared", "running"):
        raise Unverifiable("unknown owner phase")
    if not isinstance(record["lease"], str) or LEASE.fullmatch(record["lease"]) is None:
        raise Unverifiable("malformed lease")
    if child is not None and (
        child["boot_id"] != owner["boot_id"]
        or child["uid"] != owner["uid"]
        or group != child["pid"]
    ):
        raise Unverifiable("child identity and process group disagree")
    return record


def process_state(expected: dict[str, object], current_boot: str) -> str:
    if expected["boot_id"] != current_boot:
        return "gone"
    actual = process_details(int(expected["pid"]), current_boot)
    if actual is None:
        return "gone"
    return "matches" if actual[0] == expected else "reused"


def descendants_state(record: dict[str, object], current_boot: str) -> str:
    owner = identity(record["owner"])
    if owner["boot_id"] != current_boot:
        return "gone"
    wanted_lease = LEASE_ENV + str(record["lease"]).encode("ascii")
    wanted_group = record["process_group"]
    uncertain = False
    try:
        entries = list(Path("/proc").iterdir())
    except OSError:
        return "unverifiable"
    for entry in entries:
        if not entry.name.isdecimal():
            continue
        try:
            details = process_details(int(entry.name), current_boot)
        except Unverifiable:
            uncertain = True
            continue
        if details is None:
            continue
        actual, group = details
        if actual["uid"] != owner["uid"]:
            continue
        if wanted_group == group:
            return "live"
        try:
            environment = (entry / "environ").read_bytes()
        except FileNotFoundError:
            continue
        except OSError:
            # ptrace policy may hide an unrelated same-UID process's environ.
            # Child identity plus the complete process-group scan are the
            # authority; readable inherited leases catch group escapees too.
            continue
        if wanted_lease in environment.split(b"\0"):
            return "live"
    return "unverifiable" if uncertain else "gone"


def tree_state(record: dict[str, object], current_boot: str) -> str:
    owner = identity(record["owner"])
    state = process_state(owner, current_boot)
    if state != "gone":
        return "live" if state == "matches" else "unverifiable"
    child = record["child"]
    if child is not None:
        state = process_state(identity(child), current_boot)
        if state != "gone":
            return "live" if state == "matches" else "unverifiable"
    return descendants_state(record, current_boot)


def remove_same_directory(candidate: Path, before: os.stat_result) -> bool:
    try:
        after = candidate.lstat()
    except FileNotFoundError:
        return False
    if (
        stat.S_ISLNK(after.st_mode)
        or not stat.S_ISDIR(after.st_mode)
        or (before.st_dev, before.st_ino) != (after.st_dev, after.st_ino)
    ):
        return False
    shutil.rmtree(candidate)
    return True


def reclaim(root: Path, *, legacy_quiescent: bool = False) -> int:
    if not shutil.rmtree.avoids_symlink_attacks:
        raise RuntimeError("platform cannot provide symlink-safe recursive removal")
    root_metadata = root.lstat()
    if stat.S_ISLNK(root_metadata.st_mode) or not stat.S_ISDIR(root_metadata.st_mode):
        raise RuntimeError(f"refusing non-directory target mount: {root}")
    current_boot = boot_id()
    removed = 0
    for candidate in root.iterdir():
        match = NAME.fullmatch(candidate.name)
        if match is None:
            continue
        name_pid = int(match.group(1))
        if name_pid > U32_MAX:
            continue
        try:
            before = candidate.lstat()
            if stat.S_ISLNK(before.st_mode) or not stat.S_ISDIR(before.st_mode):
                continue
            try:
                record = owner_record(candidate, name_pid)
            except MissingOwner:
                if not legacy_quiescent or not remove_same_directory(candidate, before):
                    continue
                print(f"job-started: removing quiescent legacy {candidate.name}")
                removed += 1
                continue
            if tree_state(record, current_boot) != "gone":
                continue
            if not remove_same_directory(candidate, before):
                continue
        except FileNotFoundError:
            continue
        except Unverifiable:
            continue
        print(f"job-started: removing stale {candidate.name}")
        removed += 1
    return removed


def main(arguments: list[str]) -> int:
    if len(arguments) == 2:
        legacy_quiescent = False
        target = arguments[1]
    elif len(arguments) == 3 and arguments[1] == "--legacy-quiescent":
        legacy_quiescent = True
        target = arguments[2]
    else:
        print(
            "usage: reclaim-coverage.py [--legacy-quiescent] TARGET_MOUNT",
            file=sys.stderr,
        )
        return 2
    try:
        reclaim(Path(target), legacy_quiescent=legacy_quiescent)
    except (OSError, RuntimeError, Unverifiable) as error:
        print(f"job-started: coverage reclamation failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
