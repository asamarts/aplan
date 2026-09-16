#!/usr/bin/env bash
# Wait for the CI run GitHub creates for a pushed commit, and report what it did.
#
# Why this exists. Run creation is not immediate and is not bounded. Measured on
# 2026-08-26, two pushes to `main` waited 40 seconds and 18 minutes before their
# run object existed at all -- with the runner idle and nothing else queued. A
# check that gives up early reports "CI never ran" for a commit that is about to
# be checked, which is what happened twice that day.
#
# It deliberately does not dispatch. A manual `workflow_dispatch` races the push
# run, and the `concurrency` group then cancels one of the two: on `2f327d1` the
# dispatched run was cancelled twenty seconds after it started, by the very run
# it was standing in for.
#
#   bash ci/scripts/pushed.sh              # HEAD
#   bash ci/scripts/pushed.sh <sha>
#   PUSHED_TIMEOUT=300 bash ci/scripts/pushed.sh
set -euo pipefail

sha="$(git rev-parse "${1:-HEAD}")"
short="${sha:0:7}"
repo="${APLAN_CI_REPO:-$(git remote get-url origin | sed -E 's#(\.git)?/?$##; s#.*[:/]([^/]+/[^/]+)$#\1#')}"
timeout="${PUSHED_TIMEOUT:-1500}"
interval="${PUSHED_INTERVAL:-20}"

command -v gh >/dev/null || { echo "pushed: needs the gh CLI" >&2; exit 2; }

if [ "$(git rev-parse "origin/$(git rev-parse --abbrev-ref HEAD)" 2>/dev/null || echo none)" != "$sha" ]; then
    echo "pushed: $short is not what origin has for this branch; push first" >&2
fi

echo "pushed: waiting up to $((timeout / 60))m for a run on $short in $repo"
waited=0
while :; do
    row="$(gh api "repos/$repo/actions/runs?head_sha=$sha" \
        -q '.workflow_runs[]|select(.name=="CI")|"\(.status) \(.conclusion // "-") \(.html_url)"' 2>/dev/null | head -1 || true)"
    if [ -n "$row" ]; then
        read -r status conclusion url <<<"$row"
        if [ "${status:-}" = "completed" ]; then
            echo "pushed: $short -> $conclusion  (${url:-no url})"
            if [ "$conclusion" = "success" ]; then exit 0; fi
            exit 1
        fi
        printf 'pushed: %s is %s after %ds\n' "$short" "${status:-unknown}" "$waited"
    elif [ "$((waited % 120))" = 0 ]; then
        printf 'pushed: no run object for %s yet, %ds elapsed\n' "$short" "$waited"
    fi
    [ "$waited" -ge "$timeout" ] && break
    sleep "$interval"
    waited=$((waited + interval))
done

echo "pushed: no CI run appeared for $short within $((timeout / 60))m." >&2
echo "        That is not proof the push was rejected: creation has been seen to take 18m." >&2
echo "        Check https://github.com/$repo/actions before dispatching by hand, because a" >&2
echo "        dispatch races the push run and the concurrency group cancels one of them." >&2
exit 3
