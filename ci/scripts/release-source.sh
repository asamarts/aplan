#!/usr/bin/env bash
# Resolve and verify the immutable source identity shared by release validation
# and artifact construction. Output is the committed HEAD SHA and nothing else.
set -euo pipefail

repo_root="${1:-}"
if [[ "$#" -ne 1 || ! -d "$repo_root" ]]; then
    echo "release-source: expected one Git worktree root" >&2
    exit 2
fi

source_sha="$(git -C "$repo_root" rev-parse --verify 'HEAD^{commit}')" || {
    echo "release-source: release source has no committed HEAD identity" >&2
    exit 2
}

status="$(git -C "$repo_root" status --porcelain=v1 --untracked-files=all)" || exit 2
if [[ -n "$status" ]]; then
    echo "release-source: release requires a clean checkout of one committed source identity" >&2
    exit 2
fi

expected_sha="${APLAN_RELEASE_SHA:-$source_sha}"
if ! [[ "$expected_sha" =~ ^([0-9a-fA-F]{40}|[0-9a-fA-F]{64})$ ]]; then
    echo "release-source: APLAN_RELEASE_SHA must be one full commit ID, not a ref or abbreviated SHA" >&2
    exit 2
fi
expected_sha="${expected_sha,,}"
if [[ "$source_sha" != "$expected_sha" ]]; then
    echo "release-source: checked-out source $source_sha does not match expected release source $expected_sha" >&2
    exit 2
fi

printf '%s\n' "$source_sha"
