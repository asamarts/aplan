#!/usr/bin/env python3
"""Build and audit a fresh, manifest-selected public Git repository.

The manifest is JSON, which is a strict YAML subset, so parsing needs only the
Python standard library. Source bytes are always read through Git from named
commits. The working tree is never an input.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
from typing import Any


class PublicationError(RuntimeError):
    """A fail-closed publication refusal."""


def refuse(message: str) -> None:
    raise PublicationError(message)


def run(
    argv: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    input_bytes: bytes | None = None,
) -> subprocess.CompletedProcess[bytes]:
    completed = subprocess.run(
        argv,
        cwd=cwd,
        env=env,
        input=input_bytes,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        detail = completed.stderr.decode("utf-8", "replace").strip()
        refuse(f"command failed ({' '.join(argv)}): {detail}")
    return completed


def git(repo: Path, *args: str, env: dict[str, str] | None = None) -> bytes:
    return run(["git", *args], cwd=repo, env=env).stdout


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def normalized_relative(value: str, *, field: str) -> str:
    if not isinstance(value, str) or not value:
        refuse(f"{field} must be a non-empty string")
    path = PurePosixPath(value)
    if path.is_absolute() or value != path.as_posix() or ".." in path.parts:
        refuse(f"{field} is not a normalized relative path: {value!r}")
    if path.parts[0] == ".git" or "\x00" in value:
        refuse(f"{field} names forbidden Git metadata: {value!r}")
    return value


def load_manifest(path: Path) -> tuple[dict[str, Any], bytes]:
    try:
        raw = path.read_bytes()
    except OSError as error:
        refuse(f"could not read manifest {path}: {error}")
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        refuse(f"manifest {path} is not JSON-compatible YAML: {error}")
    if not isinstance(value, dict) or value.get("schema") != 1:
        refuse("manifest schema must be exactly 1")

    paths = value.get("paths")
    if not isinstance(paths, dict) or not paths:
        refuse("manifest paths must be a non-empty object")
    normalized: dict[str, str] = {}
    for source_path, classification in paths.items():
        source_path = normalized_relative(source_path, field="manifest source path")
        if classification not in {"public", "private"}:
            refuse(f"classification for {source_path!r} must be public or private")
        normalized[source_path] = classification
    value["paths"] = normalized

    releases = value.get("releases", [])
    if not isinstance(releases, list):
        refuse("manifest releases must be a list")
    seen_tags: set[str] = set()
    for release in releases:
        if not isinstance(release, dict):
            refuse("each release must be an object")
        ref = release.get("ref")
        tag = release.get("tag")
        if not isinstance(ref, str) or not ref or not isinstance(tag, str) or not tag:
            refuse("each release needs non-empty ref and tag strings")
        if tag in seen_tags or tag.startswith("-") or "/" in tag:
            refuse(f"release tag is duplicate or unsafe: {tag!r}")
        seen_tags.add(tag)
        public_commit = release.get("public_commit")
        public_tag_object = release.get("public_tag_object")
        if (public_commit is None) != (public_tag_object is None):
            refuse(f"release {tag} must pin both public commit and tag object or neither")
        if public_commit is not None and (
            not isinstance(public_commit, str)
            or not re.fullmatch(r"[0-9a-f]{40}", public_commit)
            or not isinstance(public_tag_object, str)
            or not re.fullmatch(r"[0-9a-f]{40}", public_tag_object)
        ):
            refuse(f"release {tag} has an invalid pinned public ref")
        assets = release.get("assets", [])
        if not isinstance(assets, list):
            refuse(f"release {tag} assets must be a list")
        seen_assets: set[str] = set()
        for asset in assets:
            if not isinstance(asset, dict):
                refuse(f"release {tag} has a non-object asset")
            name = normalized_relative(asset.get("name"), field="release asset name")
            asset_digest = asset.get("sha256")
            size = asset.get("size")
            if "/" in name or name in seen_assets:
                refuse(f"release {tag} has a duplicate or nested asset name: {name!r}")
            if not isinstance(asset_digest, str) or not re.fullmatch(
                r"[0-9a-f]{64}", asset_digest
            ):
                refuse(f"release {tag} asset {name} has an invalid SHA-256")
            if not isinstance(size, int) or size < 0:
                refuse(f"release {tag} asset {name} has an invalid size")
            seen_assets.add(name)

    secret_scan = value.get("secret_scan")
    if secret_scan is not None:
        if not isinstance(secret_scan, dict) or secret_scan.get("tool") != "gitleaks":
            refuse("secret_scan must name the gitleaks tool")
        version = secret_scan.get("version")
        config_path = normalized_relative(
            secret_scan.get("config_path"), field="secret scan config path"
        )
        config_digest = secret_scan.get("config_sha256")
        if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
            refuse("secret_scan has an invalid Gitleaks version")
        if config_path not in normalized or normalized[config_path] != "private":
            refuse("secret scan config must be an explicitly private control path")
        if not isinstance(config_digest, str) or not re.fullmatch(
            r"[0-9a-f]{64}", config_digest
        ):
            refuse("secret_scan has an invalid config SHA-256")

    overlays = value.get("overlays", [])
    if not isinstance(overlays, list):
        refuse("manifest overlays must be a list")
    for overlay in overlays:
        if not isinstance(overlay, dict):
            refuse("each overlay must be an object")
        source = normalized_relative(overlay.get("source"), field="overlay source")
        output = normalized_relative(overlay.get("output"), field="overlay output")
        if source not in normalized:
            refuse(f"overlay source is not classified: {source}")
        snapshots = overlay.get("snapshots", ["current"])
        if not isinstance(snapshots, list) or not snapshots or not all(
            isinstance(item, str) and item for item in snapshots
        ):
            refuse(f"overlay {output!r} needs non-empty snapshot names")
        mode = overlay.get("mode", "100644")
        if mode not in {"100644", "100755", "120000"}:
            refuse(f"overlay {output!r} has unsupported mode {mode!r}")

    transforms = value.get("transforms", [])
    if not isinstance(transforms, list):
        refuse("manifest transforms must be a list")
    for transform in transforms:
        if not isinstance(transform, dict):
            refuse("each transform must be an object")
        source = normalized_relative(transform.get("source"), field="transform source")
        output = normalized_relative(transform.get("output"), field="transform output")
        if source not in normalized:
            refuse(f"transform source is not classified: {source}")
        if transform.get("kind") not in {
            "public_config",
            "public_dependabot",
            "public_dependency_policy",
            "public_prose",
            "public_validation",
        }:
            refuse(f"transform {output!r} has an unknown kind")
        snapshots = transform.get("snapshots", ["current"])
        if not isinstance(snapshots, list) or not snapshots or not all(
            isinstance(item, str) and item for item in snapshots
        ):
            refuse(f"transform {output!r} needs non-empty snapshot names")

    prefixes = value.get("private_output_prefixes", [])
    if not isinstance(prefixes, list):
        refuse("private_output_prefixes must be a list")
    for prefix in prefixes:
        normalized_relative(prefix.rstrip("/"), field="private output prefix")

    rules = value.get("forbidden_content", [])
    if not isinstance(rules, list):
        refuse("forbidden_content must be a list")
    seen_rule_ids: set[str] = set()
    for rule in rules:
        if not isinstance(rule, dict):
            refuse("each forbidden content rule must be an object")
        rule_id = rule.get("id")
        literal = rule.get("literal")
        if (
            not isinstance(rule_id, str)
            or not rule_id
            or rule_id in seen_rule_ids
            or not isinstance(literal, str)
            or not literal
        ):
            refuse("forbidden content rules need unique ids and non-empty literals")
        seen_rule_ids.add(rule_id)

    identities = value.get("private_identities", [])
    if not isinstance(identities, list):
        refuse("private_identities must be a list")
    seen_identity_literals: set[str] = set()
    seen_identity_replacements: set[str] = set()
    for identity in identities:
        if not isinstance(identity, dict):
            refuse("each private identity must be an object")
        literal = identity.get("literal")
        replacement = identity.get("replacement")
        if (
            not isinstance(literal, str)
            or not re.fullmatch(r"[a-z0-9][a-z0-9.-]*", literal)
            or not isinstance(replacement, str)
            or not re.fullmatch(r"[a-z][a-z0-9_]*", replacement)
        ):
            refuse(
                "private identities need a lowercase repository-like literal "
                "and a lowercase identifier-safe replacement"
            )
        if len(literal.encode("utf-8")) != len(replacement.encode("utf-8")):
            refuse(
                "private identity replacements must have the same byte length "
                "as their literals so source layout remains stable"
            )
        folded = literal.casefold()
        if folded in seen_identity_literals or replacement in seen_identity_replacements:
            refuse("private identity literals and replacements must each be unique")
        seen_identity_literals.add(folded)
        seen_identity_replacements.add(replacement)
    for identity in identities:
        replacement = identity["replacement"].casefold()
        if any(literal in replacement for literal in seen_identity_literals):
            refuse("a private identity replacement contains a private identity")
    pilot_stems = {
        PurePosixPath(source_path).stem.casefold()
        for source_path, classification in normalized.items()
        if classification == "private"
        and source_path.startswith("docs/pilots/")
        and source_path.endswith(".md")
        and PurePosixPath(source_path).name
        not in {"README.md", "release-tooling-spike.md"}
    }
    missing_pilot_identities = sorted(pilot_stems - seen_identity_literals)
    if missing_pilot_identities:
        refuse(
            "private pilot log has no private identity registry entry: "
            + ", ".join(missing_pilot_identities)
        )

    required = value.get("required_output_paths", [])
    if not isinstance(required, list):
        refuse("required_output_paths must be a list")
    for required_path in required:
        normalized_relative(required_path, field="required output path")

    return value, raw


def resolve_commit(repo: Path, revision: str) -> str:
    return git(repo, "rev-parse", "--verify", f"{revision}^{{commit}}").decode().strip()


def commit_date(repo: Path, commit: str) -> str:
    return git(repo, "show", "-s", "--format=%cI", commit).decode().strip()


def tree_entries(repo: Path, commit: str) -> dict[str, dict[str, str]]:
    raw = git(repo, "ls-tree", "-rz", "--full-tree", commit)
    entries: dict[str, dict[str, str]] = {}
    for record in raw.split(b"\0"):
        if not record:
            continue
        try:
            metadata, encoded_path = record.split(b"\t", 1)
            mode, kind, object_id = metadata.decode("ascii").split(" ")
            path = encoded_path.decode("utf-8")
        except (ValueError, UnicodeDecodeError) as error:
            refuse(f"could not parse Git tree entry: {error}")
        normalized_relative(path, field="Git tree path")
        if kind != "blob" or mode not in {"100644", "100755", "120000"}:
            refuse(f"unsupported Git tree entry {mode} {kind} {path}")
        entries[path] = {"mode": mode, "object": object_id}
    return entries


def blob(repo: Path, object_id: str) -> bytes:
    return git(repo, "cat-file", "blob", object_id)


def selected_overlays(manifest: dict[str, Any], snapshot: str) -> list[dict[str, Any]]:
    return [
        item
        for item in manifest.get("overlays", [])
        if snapshot in item.get("snapshots", ["current"])
    ]


def selected_transforms(manifest: dict[str, Any], snapshot: str) -> list[dict[str, Any]]:
    return [
        item
        for item in manifest.get("transforms", [])
        if snapshot in item.get("snapshots", ["current"])
        or "*" in item.get("snapshots", ["current"])
    ]


def apply_transform(kind: str, source_path: str, data: bytes) -> bytes:
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        refuse(f"{kind} source is not UTF-8: {source_path}")
    if kind == "public_prose":
        prose_file = source_path.endswith((".md", ".json"))
        comment_prefixes = ("//", "#")

        def sanitize(value: str, *, code: bool = False) -> str:
            adoption_evidence = (
                "pre-public-adoption-evidence" if code else "pre-public adoption evidence"
            )
            technical_evaluation = (
                "pre-public-technical-evaluation"
                if code
                else "pre-public technical evaluation"
            )
            replacements = [
                (r"\[`?docs/pilots/[^]]+`?\]\([^)]+\)", adoption_evidence),
                (r"`docs/pilots/[^`]+`", adoption_evidence),
                (r"docs/pilots/[A-Za-z0-9._/-]+", adoption_evidence),
                (r"docs/pilots/", "private pre-public adoption records"),
                (r"\[[^]]+\]\([^)]+evaluations/[^)]+\)", technical_evaluation),
                (r"docs/evaluations/[A-Za-z0-9._/-]+", technical_evaluation),
                (r"\[`?RESEARCH\.md`?\]\([^)]+\)", "pre-public product research"),
                (r"\bRESEARCH\.md\b", "pre-public product research"),
                (r"\bFIELD-RESEARCH\.md\b", "pre-public field research"),
                (r"\bADOPTION-ANALYSIS\.md\b", "pre-public adoption analysis"),
                (r"`?docs/internal/publication-cutover\.md`?", "the private publication runbook"),
                (r"`asamarts/aplan[.]dev`", "the private maintainer repository"),
                (r"asamarts/aplan[.]dev", "the private maintainer repository"),
                (r"/home/[^/]+/projects/aplan", "a local aplan checkout"),
                (
                    r"ADR-(?:0011|0016|0018|0045|0064|0065)",
                    "the pre-public repository-boundary decision",
                ),
            ]
            for pattern, replacement in replacements:
                value = re.sub(pattern, replacement, value)
            return value

        if source_path.endswith(".md"):
            lines = []
            fenced = False
            for line in text.splitlines(keepends=True):
                if line.lstrip().startswith("```"):
                    fenced = not fenced
                    lines.append(line)
                    continue
                lines.append(sanitize(line, code=fenced))
            text = "".join(lines)
        elif prose_file:
            text = sanitize(text)
        else:
            lines = []
            for line in text.splitlines(keepends=True):
                if line.lstrip().startswith(comment_prefixes):
                    line = sanitize(line)
                lines.append(line)
            text = "".join(lines)
        return text.encode("utf-8")
    if kind == "public_dependency_policy":
        try:
            document = json.loads(text)
        except json.JSONDecodeError as error:
            refuse(f"dependency policy is not JSON: {source_path}: {error}")
        document["binary_budget"]["owner"] = "A2-002"
        for admission in document["admissions"]:
            admission["task"] = "A2-002"
        return (json.dumps(document, indent=2) + "\n").encode("utf-8")
    if kind == "public_dependabot":
        marker = "    open-pull-requests-limit:"
        if marker in text:
            refuse(
                "public Dependabot source already declares an open pull request limit; "
                "review the publication transform instead of overriding it"
            )
        lines = text.splitlines(keepends=True)
        ecosystems = sum(line.startswith("  - package-ecosystem:") for line in lines)
        if ecosystems == 0:
            refuse("public Dependabot source declares no package ecosystems")
        output: list[str] = []
        inserted = 0
        in_ecosystem = False
        for line in lines:
            if line.startswith("  - package-ecosystem:"):
                in_ecosystem = True
            output.append(line)
            if in_ecosystem and line.startswith("    directory:"):
                newline = "\r\n" if line.endswith("\r\n") else "\n"
                output.append(f"{marker} 0{newline}")
                inserted += 1
                in_ecosystem = False
        if inserted != ecosystems:
            refuse(
                "public Dependabot transform needs exactly one directory in every "
                f"package ecosystem; found {inserted} for {ecosystems}"
            )
        return "".join(output).encode("utf-8")
    if kind == "public_validation":
        private_exact_paths = {
            "        - ADOPTION-ANALYSIS.md\n",
            "        - FIELD-RESEARCH.md\n",
            "        - RESEARCH.md\n",
        }
        return "".join(
            line
            for line in text.splitlines(keepends=True)
            if line not in private_exact_paths
        ).encode("utf-8")
    if kind != "public_config":
        refuse(f"unknown transform kind: {kind}")
    start = text.find("\ndocs:\n")
    end = text.find("\ndeny_warnings:", start + 1)
    if start < 0 or end < 0:
        refuse("public_config could not find the exact docs policy block")
    text = (
        text[: start + 1]
        + "docs:\n  paths:\n    - aplan/README.md\n"
        + text[end + 1 :]
    )
    old_owners = "owners: [asamarts, kaminsod, t]"
    if text.count(old_owners) != 1:
        refuse("public_config could not find the exact owner vocabulary")
    text = text.replace(old_owners, "owners: [asamarts]", 1)
    return text.encode("utf-8")


def identity_pattern(literal: str, *, binary: bool = False) -> re.Pattern[Any]:
    """Match a repository identity as a token, including its case variants.

    Identifier characters deliberately count as part of the token. This keeps a
    generic identity such as ``p00011`` from matching JSON Schema's unrelated
    ``resource_baseid`` while still catching exact identifiers such as
    ``P0000004`` and repository names containing a hyphen.
    """

    boundary = rb"[A-Za-z0-9_]" if binary else r"[A-Za-z0-9_]"
    escaped = re.escape(literal.encode("ascii") if binary else literal)
    return re.compile(
        (rb"(?<!" if binary else r"(?<!")
        + boundary
        + (rb")" if binary else ")")
        + escaped
        + (rb"(?!" if binary else r"(?!")
        + boundary
        + (rb")" if binary else ")"),
        re.IGNORECASE,
    )


def public_identity(value: str, replacement: str) -> str:
    if value.isupper():
        return replacement.upper()
    if value[:1].isupper():
        return "".join(part.capitalize() for part in replacement.split("_"))
    return replacement


def sanitize_private_identities(
    data: bytes, manifest: dict[str, Any]
) -> tuple[bytes, list[dict[str, Any]]]:
    """Replace reviewed private identities in every UTF-8 output payload.

    Non-UTF-8 output is never rewritten. The independent byte-level audit below
    still refuses a private identity in it, so a binary is admitted only when it
    is already clean rather than being patched in place.
    """

    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return data, []
    redactions: list[dict[str, Any]] = []
    identities = sorted(
        manifest.get("private_identities", []),
        key=lambda item: (-len(item["literal"]), item["literal"]),
    )
    for identity in identities:
        replacement = identity["replacement"]
        text, count = identity_pattern(identity["literal"]).subn(
            lambda match: public_identity(match.group(0), replacement), text
        )
        if count:
            redactions.append(
                {
                    "literal": identity["literal"],
                    "replacement": replacement,
                    "count": count,
                }
            )
    return text.encode("utf-8"), redactions


def validate_classification(
    source: Path,
    commit: str,
    manifest: dict[str, Any],
    *,
    snapshot: str,
) -> dict[str, dict[str, str]]:
    entries = tree_entries(source, commit)
    unclassified = sorted(set(entries) - set(manifest["paths"]))
    if unclassified:
        shown = ", ".join(unclassified[:8])
        suffix = "" if len(unclassified) <= 8 else f" (+{len(unclassified) - 8} more)"
        refuse(f"unclassified path(s) in {snapshot}: {shown}{suffix}")
    return entries


def clear_worktree(candidate: Path) -> None:
    for child in candidate.iterdir():
        if child.name == ".git":
            continue
        if child.is_symlink() or child.is_file():
            child.unlink()
        else:
            shutil.rmtree(child)


def write_output(root: Path, path: str, mode: str, data: bytes) -> None:
    destination = root.joinpath(*PurePosixPath(path).parts)
    destination.parent.mkdir(parents=True, exist_ok=True)
    if mode == "120000":
        try:
            target = data.decode("utf-8")
        except UnicodeDecodeError:
            refuse(f"symlink target for {path} is not UTF-8")
        os.symlink(target, destination)
        return
    destination.write_bytes(data)
    destination.chmod(0o755 if mode == "100755" else 0o644)


def snapshot_files(
    source: Path,
    control_commit: str,
    source_commit: str,
    manifest: dict[str, Any],
    *,
    snapshot: str,
    destination: Path,
) -> list[dict[str, Any]]:
    entries = validate_classification(
        source, source_commit, manifest, snapshot=snapshot
    )
    control_entries = tree_entries(source, control_commit)
    output: dict[str, dict[str, Any]] = {}
    for path, entry in entries.items():
        if manifest["paths"][path] == "private":
            continue
        data = blob(source, entry["object"])
        output[path] = {
            "output_path": path,
            "mode": entry["mode"],
            "size": len(data),
            "sha256": digest(data),
            "source_path": path,
            "source_blob": entry["object"],
            "origin": "source",
            "data": data,
        }

    for overlay in selected_overlays(manifest, snapshot):
        source_path = overlay["source"]
        output_path = overlay["output"]
        source_entry = control_entries.get(source_path)
        if source_entry is None:
            refuse(f"overlay source is absent from control revision: {source_path}")
        data = blob(source, source_entry["object"])
        output[output_path] = {
            "output_path": output_path,
            "mode": overlay.get("mode", "100644"),
            "size": len(data),
            "sha256": digest(data),
            "source_path": source_path,
            "source_blob": source_entry["object"],
            "origin": "overlay",
            "data": data,
        }

    for transform in selected_transforms(manifest, snapshot):
        source_path = transform["source"]
        output_path = transform["output"]
        source_entries = (
            entries if transform.get("source_revision") == "snapshot" else control_entries
        )
        source_entry = source_entries.get(source_path)
        if source_entry is None:
            if transform.get("optional", False):
                continue
            refuse(f"transform source is absent from selected revision: {source_path}")
        source_data = blob(source, source_entry["object"])
        data = apply_transform(transform["kind"], source_path, source_data)
        output[output_path] = {
            "output_path": output_path,
            "mode": transform.get("mode", source_entry["mode"]),
            "size": len(data),
            "sha256": digest(data),
            "source_path": source_path,
            "source_blob": source_entry["object"],
            "origin": f"transform:{transform['kind']}",
            "data": data,
        }

    for record in output.values():
        data, redactions = sanitize_private_identities(record["data"], manifest)
        record["data"] = data
        record["size"] = len(data)
        record["sha256"] = digest(data)
        if redactions:
            record["identity_redactions"] = redactions

    audit_file_payloads(output, manifest, require_paths=snapshot == "current")
    clear_worktree(destination)
    report_files: list[dict[str, Any]] = []
    for path in sorted(output):
        record = output[path]
        write_output(destination, path, record["mode"], record["data"])
        report_files.append({key: value for key, value in record.items() if key != "data"})
    return report_files


def commit_environment(date: str) -> dict[str, str]:
    env = os.environ.copy()
    env.update(
        {
            "GIT_AUTHOR_NAME": "Aplan Maintainers",
            "GIT_AUTHOR_EMAIL": "aplan-maintainers@users.noreply.github.com",
            "GIT_AUTHOR_DATE": date,
            "GIT_COMMITTER_NAME": "Aplan Maintainers",
            "GIT_COMMITTER_EMAIL": "aplan-maintainers@users.noreply.github.com",
            "GIT_COMMITTER_DATE": date,
        }
    )
    return env


def commit_snapshot(candidate: Path, message: str, date: str) -> str:
    git(candidate, "add", "-A")
    git(candidate, "commit", "-q", "--allow-empty", "-m", message, env=commit_environment(date))
    return resolve_commit(candidate, "HEAD")


def tag_snapshot(candidate: Path, tag: str, commit: str, date: str) -> str:
    git(
        candidate,
        "tag",
        "-a",
        tag,
        commit,
        "-m",
        f"Reconstructed public {tag}",
        env=commit_environment(date),
    )
    return git(candidate, "rev-parse", f"refs/tags/{tag}").decode().strip()


def ref_inventory(repo: Path) -> dict[str, str]:
    raw = git(repo, "for-each-ref", "--format=%(refname)%00%(objectname)", "refs/heads", "refs/tags")
    refs: dict[str, str] = {}
    for line in raw.splitlines():
        if not line:
            continue
        name, object_id = line.decode("ascii").split("\x00", 1)
        refs[name] = object_id
    return dict(sorted(refs.items()))


def atomic_json(path: Path, value: Any) -> None:
    if path.exists():
        refuse(f"refusing to overwrite report: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    staging = path.with_name(f".{path.name}.new")
    if staging.exists():
        refuse(f"report staging path already exists: {staging}")
    staging.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(staging, path)


def audit_policy(manifest: dict[str, Any]) -> dict[str, Any]:
    return {
        "release_assets": [
            {
                "tag": release["tag"],
                "public_commit": release.get("public_commit"),
                "public_tag_object": release.get("public_tag_object"),
                "assets": release.get("assets", []),
            }
            for release in manifest.get("releases", [])
        ],
        "secret_scan": manifest.get("secret_scan"),
    }


def build(args: argparse.Namespace) -> None:
    source = Path(args.source).resolve()
    candidate = Path(args.output).resolve()
    report_path = Path(args.report).resolve()
    manifest_path = Path(args.manifest).resolve()
    manifest, manifest_raw = load_manifest(manifest_path)
    if candidate.exists():
        refuse(f"output already exists: {candidate}")
    if report_path.exists():
        refuse(f"report already exists: {report_path}")
    if candidate == source or source in candidate.parents or candidate in source.parents:
        refuse("source and candidate must be separate directory trees")
    control_commit = resolve_commit(source, args.revision)

    manifest_source_path = manifest.get("manifest_source_path")
    if manifest_source_path is not None:
        manifest_source_path = normalized_relative(
            manifest_source_path, field="manifest_source_path"
        )
        control_entry = tree_entries(source, control_commit).get(manifest_source_path)
        if control_entry is None:
            refuse(f"committed manifest is absent: {manifest_source_path}")
        if blob(source, control_entry["object"]) != manifest_raw:
            refuse("manifest bytes differ from the reviewed control revision")

    revisions: list[tuple[str, str, str | None]] = []
    for release in manifest.get("releases", []):
        revisions.append((release["tag"], resolve_commit(source, release["ref"]), release["tag"]))
    revisions.append(("current", control_commit, None))
    for snapshot, commit, _tag in revisions:
        validate_classification(source, commit, manifest, snapshot=snapshot)

    candidate.parent.mkdir(parents=True, exist_ok=True)
    candidate.mkdir()
    git(candidate, "init", "-q", "-b", "main")
    git(candidate, "config", "user.name", "Aplan Maintainers")
    git(
        candidate,
        "config",
        "user.email",
        "aplan-maintainers@users.noreply.github.com",
    )
    snapshots: list[dict[str, Any]] = []
    releases_by_tag = {
        release["tag"]: release for release in manifest.get("releases", [])
    }
    for snapshot, source_commit, tag in revisions:
        files = snapshot_files(
            source,
            control_commit,
            source_commit,
            manifest,
            snapshot=snapshot,
            destination=candidate,
        )
        date = commit_date(source, source_commit)
        message = (
            f"Reconstruct aplan {tag} public snapshot"
            if tag is not None
            else "Import reviewed aplan public baseline"
        )
        public_commit = commit_snapshot(candidate, message, date)
        public_tag_object = None
        if tag is not None:
            public_tag_object = tag_snapshot(candidate, tag, public_commit, date)
            release = releases_by_tag[tag]
            if release.get("public_commit") not in {None, public_commit}:
                refuse(f"reconstructed public commit differs for {tag}")
            if release.get("public_tag_object") not in {None, public_tag_object}:
                refuse(f"reconstructed public tag object differs for {tag}")
        snapshots.append(
            {
                "name": snapshot,
                "source_commit": source_commit,
                "source_date": date,
                "public_commit": public_commit,
                "public_tag_object": public_tag_object,
                "files": files,
            }
        )

    git(candidate, "branch", "-M", "main")
    report = {
        "schema": 1,
        "source": str(source),
        "control_commit": control_commit,
        "manifest_sha256": digest(manifest_raw),
        "candidate": str(candidate),
        "refs": ref_inventory(candidate),
        "snapshots": snapshots,
        "audit_policy": audit_policy(manifest),
    }
    atomic_json(report_path, report)
    audit_paths(
        source,
        candidate,
        manifest_path,
        report_path,
        gitleaks=Path(args.gitleaks).resolve() if args.gitleaks else None,
        release_assets=(
            Path(args.release_assets).resolve() if args.release_assets else None
        ),
    )
    print(
        f"public candidate built: {len(snapshots)} snapshot(s), "
        f"{len(snapshots[-1]['files'])} current file(s), manifest {digest(manifest_raw)}"
    )


def audit_file_payloads(
    records: dict[str, dict[str, Any]],
    manifest: dict[str, Any],
    *,
    require_paths: bool,
) -> None:
    prefixes = tuple(manifest.get("private_output_prefixes", []))
    for path, record in records.items():
        if any(path == prefix.rstrip("/") or path.startswith(prefix) for prefix in prefixes):
            refuse(f"private output path: {path}")
        data = record["data"]
        for identity in manifest.get("private_identities", []):
            literal = identity["literal"]
            if identity_pattern(literal).search(path):
                refuse(f"private identity {literal!r} in output path {path}")
            if identity_pattern(literal, binary=True).search(data):
                refuse(f"private identity {literal!r} in {path}")
        for rule in manifest.get("forbidden_content", []):
            needle = rule["literal"].encode("utf-8")
            if needle in data:
                refuse(f"forbidden content {rule['id']} in {path}")
    if require_paths:
        for required in manifest.get("required_output_paths", []):
            if required not in records:
                refuse(f"required output path is absent: {required}")


def private_identity_in_stream(
    stream: Any, manifest: dict[str, Any]
) -> str | None:
    identities = manifest.get("private_identities", [])
    if not identities:
        return None
    patterns = [
        (identity["literal"], identity_pattern(identity["literal"], binary=True))
        for identity in identities
    ]
    overlap = max(len(literal.encode("ascii")) for literal, _pattern in patterns) + 2
    tail = b""
    while chunk := stream.read(1024 * 1024):
        window = tail + chunk
        for literal, pattern in patterns:
            if pattern.search(window):
                return literal
        tail = window[-overlap:]
    return None


def forbidden_content_in_stream(
    stream: Any, manifest: dict[str, Any]
) -> str | None:
    rules = [
        (rule["id"], rule["literal"].encode("utf-8"))
        for rule in manifest.get("forbidden_content", [])
    ]
    if not rules:
        return None
    overlap = max(len(needle) for _rule_id, needle in rules) - 1
    tail = b""
    while chunk := stream.read(1024 * 1024):
        window = tail + chunk
        for rule_id, needle in rules:
            if needle in window:
                return rule_id
        tail = window[-overlap:] if overlap else b""
    return None


def worktree_records(root: Path) -> dict[str, dict[str, Any]]:
    records: dict[str, dict[str, Any]] = {}
    for directory, names, files in os.walk(root, topdown=True, followlinks=False):
        names[:] = sorted(name for name in names if name != ".git")
        for name in sorted(files):
            path = Path(directory, name)
            relative = path.relative_to(root).as_posix()
            metadata = path.lstat()
            if stat.S_ISLNK(metadata.st_mode):
                data = os.readlink(path).encode("utf-8")
                mode = "120000"
            elif stat.S_ISREG(metadata.st_mode):
                data = path.read_bytes()
                mode = "100755" if metadata.st_mode & stat.S_IXUSR else "100644"
            else:
                refuse(f"unsupported output file type: {relative}")
            records[relative] = {
                "output_path": relative,
                "mode": mode,
                "size": len(data),
                "sha256": digest(data),
                "data": data,
            }
        for name in list(names):
            path = Path(directory, name)
            if path.is_symlink():
                names.remove(name)
                relative = path.relative_to(root).as_posix()
                data = os.readlink(path).encode("utf-8")
                records[relative] = {
                    "output_path": relative,
                    "mode": "120000",
                    "size": len(data),
                    "sha256": digest(data),
                    "data": data,
                }
    return records


def object_inventory(repo: Path) -> dict[str, str]:
    raw = git(
        repo,
        "cat-file",
        "--batch-all-objects",
        "--batch-check=%(objectname) %(objecttype)",
    )
    objects: dict[str, str] = {}
    for line in raw.decode("ascii").splitlines():
        object_id, kind = line.split(" ", 1)
        objects[object_id] = kind
    return objects


def audit_release_assets(manifest: dict[str, Any], root: Path | None) -> None:
    expected: dict[str, dict[str, Any]] = {}
    for release in manifest.get("releases", []):
        tag = release["tag"]
        for asset in release.get("assets", []):
            expected[f"{tag}/{asset['name']}"] = asset
    if not expected:
        return
    if root is None or not root.is_dir():
        refuse("release assets are required by the manifest")
    observed: dict[str, Path] = {}
    for path in sorted(root.rglob("*")):
        if path.is_symlink() or (path.exists() and not (path.is_dir() or path.is_file())):
            refuse(f"unsupported release asset entry: {path}")
        if path.is_file():
            observed[path.relative_to(root).as_posix()] = path
    if set(observed) != set(expected):
        extra = sorted(set(observed) - set(expected))
        missing = sorted(set(expected) - set(observed))
        refuse(f"release asset file set differs; unexpected={extra[:4]} missing={missing[:4]}")
    for relative, asset in expected.items():
        path = observed[relative]
        data = path.read_bytes()
        if len(data) != asset["size"] or digest(data) != asset["sha256"]:
            refuse(f"release asset digest differs: {relative}")
        if relative.endswith(".tar.gz"):
            with tarfile.open(path, "r:gz") as archive:
                members = archive.getmembers()
                if not members:
                    refuse(f"release archive is empty: {relative}")
                for member in members:
                    member_path = PurePosixPath(member.name)
                    if (
                        member_path.is_absolute()
                        or ".." in member_path.parts
                        or not (member.isfile() or member.isdir())
                    ):
                        refuse(f"unsafe release archive member in {relative}: {member.name}")
                    for identity in manifest.get("private_identities", []):
                        literal = identity["literal"]
                        if identity_pattern(literal).search(member.name):
                            refuse(
                                f"private identity {literal!r} in release archive "
                                f"member name {relative}:{member.name}"
                            )
                    for rule in manifest.get("forbidden_content", []):
                        if rule["literal"] in member.name:
                            refuse(
                                f"forbidden content {rule['id']} in release archive "
                                f"member name {relative}:{member.name}"
                            )
                    if member.isfile():
                        extracted = archive.extractfile(member)
                        if extracted is None:
                            refuse(
                                f"release archive member could not be read in "
                                f"{relative}: {member.name}"
                            )
                        literal = private_identity_in_stream(extracted, manifest)
                        if literal is not None:
                            refuse(
                                f"private identity {literal!r} in release archive "
                                f"payload {relative}:{member.name}"
                            )
                        extracted = archive.extractfile(member)
                        if extracted is None:
                            refuse(
                                f"release archive member could not be read in "
                                f"{relative}: {member.name}"
                            )
                        rule_id = forbidden_content_in_stream(extracted, manifest)
                        if rule_id is not None:
                            refuse(
                                f"forbidden content {rule_id} in release archive "
                                f"payload {relative}:{member.name}"
                            )
        if relative.endswith(".tar.gz.sha256"):
            archive_name = Path(relative).name.removesuffix(".sha256")
            archive_relative = f"{Path(relative).parent.as_posix()}/{archive_name}"
            archive_asset = expected.get(archive_relative)
            if archive_asset is None:
                refuse(f"checksum has no matching release archive: {relative}")
            wanted = f"{archive_asset['sha256']}  {archive_name}\n".encode()
            if data != wanted:
                refuse(f"release checksum content differs: {relative}")


def audit_secrets(
    source: Path,
    control_commit: str,
    candidate: Path,
    manifest: dict[str, Any],
    gitleaks: Path | None,
) -> None:
    policy = manifest.get("secret_scan")
    if policy is None:
        return
    if gitleaks is None or not gitleaks.is_file() or not os.access(gitleaks, os.X_OK):
        refuse("the pinned Gitleaks executable is required")
    version = run([str(gitleaks), "version"]).stdout.decode("utf-8", "replace").strip()
    if version != policy["version"]:
        refuse(f"Gitleaks version differs: expected {policy['version']}, observed {version}")
    config_entry = tree_entries(source, control_commit).get(policy["config_path"])
    if config_entry is None:
        refuse("secret scan config is absent from the control revision")
    config = blob(source, config_entry["object"])
    if digest(config) != policy["config_sha256"]:
        refuse("secret scan config digest differs from the manifest")
    try:
        config_text = config.decode("utf-8")
    except UnicodeDecodeError:
        refuse("secret scan config is not UTF-8")
    with tempfile.TemporaryDirectory(prefix="aplan-gitleaks-", dir=candidate.parent) as temporary:
        report = Path(temporary) / "report.json"
        env = os.environ.copy()
        env.pop("GITLEAKS_CONFIG", None)
        env["GITLEAKS_CONFIG_TOML"] = config_text
        completed = subprocess.run(
            [
                str(gitleaks),
                "git",
                "--no-banner",
                "--redact=100",
                "--report-format",
                "json",
                "--report-path",
                str(report),
                "--log-opts=--all",
                str(candidate),
            ],
            cwd=candidate,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        findings: Any = None
        if report.is_file():
            try:
                findings = json.loads(report.read_text(encoding="utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as error:
                refuse(f"Gitleaks report is not valid JSON: {error}")
        if isinstance(findings, list) and findings:
            refuse(f"secret scanner found {len(findings)} leak(s)")
        if completed.returncode != 0:
            detail = completed.stderr.decode("utf-8", "replace").strip()
            refuse(f"Gitleaks failed without a reported finding: {detail}")
        if findings != []:
            refuse("Gitleaks did not emit an explicit empty JSON findings list")


def audit_paths(
    source: Path,
    candidate: Path,
    manifest_path: Path,
    report_path: Path,
    *,
    gitleaks: Path | None,
    release_assets: Path | None,
) -> None:
    manifest, manifest_raw = load_manifest(manifest_path)
    try:
        report = json.loads(report_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        refuse(f"could not read audit report {report_path}: {error}")
    if not isinstance(report, dict) or report.get("schema") != 1:
        refuse("audit report schema must be exactly 1")
    if report.get("manifest_sha256") != digest(manifest_raw):
        refuse("audit report manifest digest does not match")
    if report.get("audit_policy") != audit_policy(manifest):
        refuse("audit report policy differs from the manifest")
    if Path(report.get("source", "")).resolve() != source:
        refuse("audit source differs from the build report")
    if resolve_commit(source, report["control_commit"]) != report["control_commit"]:
        refuse("reported source control commit no longer resolves")

    actual_refs = ref_inventory(candidate)
    if actual_refs != report.get("refs"):
        refuse("candidate refs differ from the reviewed report")
    if git(candidate, "remote").strip():
        refuse("candidate has a remote before staging")
    common = Path(git(candidate, "rev-parse", "--git-common-dir").decode().strip())
    if not common.is_absolute():
        common = candidate / common
    if (common / "objects/info/alternates").exists():
        refuse("candidate uses an alternate object store")

    fsck = run(
        ["git", "fsck", "--full", "--strict", "--unreachable", "--no-reflogs"],
        cwd=candidate,
    )
    fsck_text = (fsck.stdout + fsck.stderr).decode("utf-8", "replace")
    if "unreachable " in fsck_text or "dangling " in fsck_text:
        refuse(f"candidate contains an unreachable object: {fsck_text.strip()}")

    source_objects = object_inventory(source)
    candidate_objects = object_inventory(candidate)
    forbidden_source_ids = {
        object_id
        for object_id, kind in source_objects.items()
        if kind in {"commit", "tag"}
    }
    shared_history = sorted(forbidden_source_ids & set(candidate_objects))
    if shared_history:
        refuse(f"candidate contains private commit or tag object: {shared_history[0]}")

    snapshots = report.get("snapshots")
    if not isinstance(snapshots, list) or not snapshots:
        refuse("audit report has no snapshots")
    by_commit = {item.get("public_commit"): item for item in snapshots}
    releases = manifest.get("releases", [])
    if [item.get("name") for item in snapshots] != [
        *[release["tag"] for release in releases],
        "current",
    ]:
        refuse("snapshot order or names differ from the manifest")
    for release, snapshot in zip(releases, snapshots[:-1], strict=True):
        tag_ref = f"refs/tags/{release['tag']}"
        if actual_refs.get(tag_ref) != snapshot.get("public_tag_object"):
            refuse(f"public tag object differs for {release['tag']}")
        if resolve_commit(candidate, tag_ref) != snapshot.get("public_commit"):
            refuse(f"public tag target differs for {release['tag']}")
        if release.get("public_tag_object") not in {
            None,
            snapshot.get("public_tag_object"),
        }:
            refuse(f"public tag object differs from its pin for {release['tag']}")
        if release.get("public_commit") not in {None, snapshot.get("public_commit")}:
            refuse(f"public tag target differs from its pin for {release['tag']}")
    for public_commit, snapshot in by_commit.items():
        if not isinstance(public_commit, str):
            refuse("snapshot has no public commit")
        tree = tree_entries(candidate, public_commit)
        expected = {item["output_path"]: item for item in snapshot["files"]}
        if set(tree) != set(expected):
            refuse(f"tree file set differs for snapshot {snapshot.get('name')}")
        observed_records: dict[str, dict[str, Any]] = {}
        for path, entry in tree.items():
            data = blob(candidate, entry["object"])
            wanted = expected[path]
            if (
                entry["mode"] != wanted["mode"]
                or len(data) != wanted["size"]
                or digest(data) != wanted["sha256"]
            ):
                refuse(f"tree content differs for {snapshot.get('name')}:{path}")
            observed_records[path] = {"data": data, **wanted}
        audit_file_payloads(
            observed_records,
            manifest,
            require_paths=snapshot.get("name") == "current",
        )

    current = snapshots[-1]
    if current.get("name") != "current" or resolve_commit(candidate, "main") != current.get(
        "public_commit"
    ):
        refuse("main does not name the reported current snapshot")
    expected_current = {item["output_path"]: item for item in current["files"]}
    observed = worktree_records(candidate)
    audit_file_payloads(observed, manifest, require_paths=True)
    if set(observed) != set(expected_current):
        unexpected = sorted(set(observed) - set(expected_current))
        missing = sorted(set(expected_current) - set(observed))
        refuse(f"working tree file set differs; unexpected={unexpected[:4]} missing={missing[:4]}")
    for path, wanted in expected_current.items():
        got = observed[path]
        if (
            got["mode"] != wanted["mode"]
            or got["size"] != wanted["size"]
            or got["sha256"] != wanted["sha256"]
        ):
            refuse(f"working tree differs from reviewed output: {path}")
    if git(candidate, "status", "--porcelain=v1", "--untracked-files=all").strip():
        refuse("candidate working tree is dirty")

    audit_release_assets(manifest, release_assets)
    audit_secrets(source, report["control_commit"], candidate, manifest, gitleaks)

    for check in manifest.get("audit_commands", []):
        if not isinstance(check, list) or not check or not all(
            isinstance(item, str) and item for item in check
        ):
            refuse("each audit command must be a non-empty argv list")
        argv = [item.replace("{candidate}", str(candidate)) for item in check]
        run(argv, cwd=candidate)


def audit(args: argparse.Namespace) -> None:
    source = Path(args.source).resolve()
    candidate = Path(args.candidate).resolve()
    manifest_path = Path(args.manifest).resolve()
    report_path = Path(args.report).resolve()
    audit_paths(
        source,
        candidate,
        manifest_path,
        report_path,
        gitleaks=Path(args.gitleaks).resolve() if args.gitleaks else None,
        release_assets=(
            Path(args.release_assets).resolve() if args.release_assets else None
        ),
    )
    print("public candidate audit passed")


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)
    build_parser = commands.add_parser("build", help="construct a fresh candidate repository")
    build_parser.add_argument("--source", required=True)
    build_parser.add_argument("--revision", required=True)
    build_parser.add_argument("--manifest", required=True)
    build_parser.add_argument("--output", required=True)
    build_parser.add_argument("--report", required=True)
    build_parser.add_argument("--gitleaks")
    build_parser.add_argument("--release-assets")
    build_parser.set_defaults(action=build)
    audit_parser = commands.add_parser("audit", help="audit a constructed candidate repository")
    audit_parser.add_argument("--source", required=True)
    audit_parser.add_argument("--candidate", required=True)
    audit_parser.add_argument("--manifest", required=True)
    audit_parser.add_argument("--report", required=True)
    audit_parser.add_argument("--gitleaks")
    audit_parser.add_argument("--release-assets")
    audit_parser.set_defaults(action=audit)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        args.action(args)
    except PublicationError as error:
        print(f"publication: refused: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
