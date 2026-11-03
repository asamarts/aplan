#!/usr/bin/env bash
# Wait for the CI run GitHub creates for a pushed commit, and report what it did.
#
# Why this exists. Run creation is not immediate and is not bounded. Measured on
# 2026-08-26, two pushes to `main` waited 40 seconds and 18 minutes before their
# run object existed at all -- with the runner idle and nothing else queued. A
# check that gives up early reports "CI never ran" for a commit that is about to
# be checked, which is what happened twice that day.
#
# It deliberately does not dispatch. Manual dispatch is an independent full
# run, not a substitute for the development verdict created by a push. The API
# filter below admits only the push event so a full run on the same SHA cannot
# be mistaken for the result this monitor promises to observe.
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

if ! [[ "$timeout" =~ ^[0-9]{1,9}$ ]]; then
    echo "pushed: PUSHED_TIMEOUT must be a non-negative integer (got '$timeout')" >&2
    exit 2
fi
if ! [[ "$interval" =~ ^[0-9]{1,9}$ ]] || [ "$((10#$interval))" -eq 0 ]; then
    echo "pushed: PUSHED_INTERVAL must be a positive integer (got '$interval')" >&2
    exit 2
fi
timeout=$((10#$timeout))
interval=$((10#$interval))

if [ "$(git rev-parse "origin/$(git rev-parse --abbrev-ref HEAD)" 2>/dev/null || echo none)" != "$sha" ]; then
    echo "pushed: $short is not what origin has for this branch; push first" >&2
fi

echo "pushed: waiting up to $((timeout / 60))m for a run on $short in $repo"
waited=0
seen_run=0
last_status=""
last_url=""
while :; do
    if ! rows="$(gh api "repos/$repo/actions/runs?head_sha=$sha" \
        -q '[.workflow_runs[] | select(.name == "CI" and .event == "push")] | sort_by(.created_at, .id) | last | if . == null then empty else "\(.status) \(.conclusion // "-") \(.html_url)" end' 2>&1)"; then
        echo "pushed: GitHub API query failed for $repo; CI status is unknown:" >&2
        printf '%s\n' "$rows" >&2
        exit 2
    fi
    row="$rows"
    if [[ "$row" == *$'\n'* ]]; then
        echo "pushed: GitHub API returned more than one selected CI run; status is unknown" >&2
        exit 2
    fi
    if [ -n "$row" ]; then
        read -r status conclusion url <<<"$row"
        if [ -z "${status:-}" ] || [ -z "${conclusion:-}" ] || [ -z "${url:-}" ]; then
            echo "pushed: GitHub API returned a malformed CI run; status is unknown" >&2
            exit 2
        fi
        seen_run=1
        last_status="${status:-unknown}"
        last_url="${url:-no URL}"
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
    remaining=$((timeout - waited))
    sleep_for="$interval"
    if [ "$sleep_for" -gt "$remaining" ]; then
        sleep_for="$remaining"
    fi
    sleep "$sleep_for"
    waited=$((waited + sleep_for))
done

if [ "$seen_run" = 1 ]; then
    echo "pushed: CI run for $short appeared but did not complete within $((timeout / 60))m." >&2
    echo "        Last status: $last_status  (${last_url})" >&2
    exit 3
fi

echo "pushed: no CI run appeared for $short within $((timeout / 60))m." >&2
echo "        That is not proof the push was rejected: creation has been seen to take 18m." >&2
echo "        Check https://github.com/$repo/actions; a manual dispatch is a separate full" >&2
echo "        run and is not evidence for the missing push-triggered development verdict." >&2
exit 3
