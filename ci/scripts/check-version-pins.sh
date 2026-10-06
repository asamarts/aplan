#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Do the install pins in this repository's documentation name a release that
# exists, and the newest one? (`A1-318`)
#
# HLD section 17 promises "a CI gate that fails when any documented version
# string disagrees", and it did not exist. What follows is narrower than that
# sentence, deliberately, because the sentence is not implementable as written.
#
# **The unit is an occurrence, not a file.** Measured on `docs/upgrading.md`:
# line 4 says a repository "needs a feature newer than `v0.3.0`", which must
# move when v0.4.0 ships, and line 5 says most of this "shipped in `v0.3.0`",
# which must not. Same string, same file, opposite obligations. A file-level
# scope list cannot separate them and neither can a regex over bare versions.
#
# **So the gate checks the one form that is unambiguous**: a GitHub Actions
# pin, `uses: <owner>/<repo>@vX.Y.Z`. That is an instruction to install a
# specific release, wherever it appears, and it is wrong the moment a newer
# release exists. Prose about what shipped is left alone, because it is history
# and sweeping it would falsify a record.
#
# **The comparison is against the newest release tag, not the workspace
# version.** This differs from the sibling tool `alint`, whose equivalent gate
# asserts install snippets equal `[workspace.package].version` -- correct there
# and wrong here, because this workspace carries `X.Y.Z-dev` between releases
# (`A1-060`) and no published release ever matches it. Copying that rule would
# have failed on every commit.
#
# Tag selection matches ADR-0038's window rule: reachable from HEAD, matching
# the release pattern, newest by history rather than by date. This repository
# carries `archive/*` tags that are not ancestors of `main`, and a date-ordered
# pick would have chosen one of them.

pattern='v[0-9]\+\.[0-9]\+\.[0-9]\+'

latest_release() {
    git for-each-ref --format='%(refname:short)' refs/tags \
        | grep -x "$pattern" \
        | while read -r t; do
            git merge-base --is-ancestor "$t" HEAD 2>/dev/null && echo "$t"
        done \
        | sort -V | tail -1
}

LATEST="$(latest_release || true)"
if [[ -z "$LATEST" ]]; then
    echo "==> check-version-pins: no release tag is reachable from HEAD, so there is" \
         "nothing for an install pin to be wrong about yet"
    exit 0
fi

# **Scoped by what a document is for, because syntax was not enough.** The
# first cut of this gate matched every `uses: ...@vX.Y.Z` in tracked text and
# found four, all of them quotations rather than instructions: a CHANGELOG
# entry about the v0.2.0 pin problem, two task bodies quoting a snippet, and a
# pilot log recording what an adopter actually ran. A record of an instruction
# is not an instruction, and nothing in the string says which it is.
#
# So the scope is the documents that *tell an adopter what to run*. Everything
# else is a record, and a record that moves is a falsified record.
#
# **Excluded, not included**, which is the second thing measurement changed.
# The first cut listed the three documents that instruct -- README.md,
# docs/upgrading.md, action.yml -- and a fourth was then invisible: a
# `docs/quickstart.md` carrying a stale `@v0.1.0` passed the gate in silence,
# because a hand-maintained inclusion list cannot know about a file nobody
# added to it. That is `A1-210`'s shape, a check that cannot fail on the case
# it exists for.
#
# So everything is in scope and records are named out. A new instructing
# document is covered the day it is written; a new *record* has to be excluded
# deliberately, and the failure mode is a gate that asks rather than one that
# sleeps.
#
#   excluded   CHANGELOG.md      per-release entries, by definition historical
#              aplan/tasks/**    what a task said at the time it was written
#              private pre-public adoption records**    what an adopter ran, which is the measurement
#              docs/adr/**       decisions, dated and immutable
EXCLUDED=(':(exclude)CHANGELOG.md' ':(exclude)aplan/tasks/**' ':(exclude)docs/pilots/**'
          ':(exclude)docs/adr/**')

mapfile -t found < <(
    git grep -n -E "uses: *[A-Za-z0-9_.-]+/aplan@${pattern//\\/}" -- . "${EXCLUDED[@]}" \
        2>/dev/null || true
)

stale=()
for line in "${found[@]}"; do
    pin="$(sed -n "s/.*@\(${pattern}\).*/\1/p" <<<"$line")"
    [[ -n "$pin" && "$pin" != "$LATEST" ]] && stale+=("$line")
done

if ((${#stale[@]})); then
    printf 'error: %d install pin(s) name a release other than %s, the newest one:\n' \
        "${#stale[@]}" "$LATEST" >&2
    printf '  %s\n' "${stale[@]}" >&2
    printf 'Point them at %s, or cut the release they name.\n' "$LATEST" >&2
    # Two different wrongs, and saying "an older binary" for both was itself
    # wrong: a pin to a version that was never released installs nothing at
    # all, which is a louder failure than an outdated binary and a different
    # thing to go and fix.
    for line in "${stale[@]}"; do
        pin="$(sed -n "s/.*@\(${pattern}\).*/\1/p" <<<"$line")"
        if git rev-parse -q --verify "refs/tags/${pin}" >/dev/null 2>&1; then
            printf '  %s exists: an adopter copying that line installs it instead of %s.\n' \
                "$pin" "$LATEST" >&2
        else
            printf '  %s is not a tag in this repository: an adopter copying that line\n' \
                "$pin" >&2
            printf '    installs nothing, and the action fails at resolve time.\n' >&2
        fi
    done
    exit 1
fi

echo "==> check-version-pins: ${#found[@]} install pin(s) name ${LATEST}, the newest release"
