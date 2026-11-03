#!/usr/bin/env bash
set -euo pipefail

# Assert the three repository settings that together make the public-CI
# transition complete. Run this after changing visibility and before treating
# the repository as safely public (the pre-public repository-boundary decision).

repo="${1:-${GITHUB_REPOSITORY:-}}"
gh_bin="${GH_BIN:-gh}"

case "$repo" in
    */*) ;;
    *)
        echo "usage: ci/scripts/check-public-runner-boundary.sh OWNER/REPOSITORY" >&2
        exit 2
        ;;
esac

visibility="$($gh_bin api "repos/${repo}" --jq '.visibility')"
if [[ "$visibility" != "public" ]]; then
    echo "error: ${repo} visibility is '${visibility}', expected 'public'" >&2
    exit 1
fi

# This endpoint lists repository self-hosted runners. Zero is deliberate: a
# fork workflow can type any old fixed label, but there is no box for GitHub to
# assign it to.
runner_count="$($gh_bin api "repos/${repo}/actions/runners" --jq '.total_count')"
if [[ "$runner_count" != "0" ]]; then
    echo "error: ${repo} has ${runner_count} registered self-hosted runner(s), expected 0" >&2
    exit 1
fi

approval="$($gh_bin api "repos/${repo}/actions/permissions/fork-pr-contributor-approval" \
    --jq '.approval_policy')"
if [[ "$approval" != "all_external_contributors" ]]; then
    echo "error: ${repo} fork approval policy is '${approval}', expected" >&2
    echo "       'all_external_contributors'" >&2
    exit 1
fi

echo "public runner boundary holds for ${repo}: hosted only; every external contribution needs approval"
