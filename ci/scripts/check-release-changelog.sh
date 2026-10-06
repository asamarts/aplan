#!/usr/bin/env bash
set -euo pipefail

# A release version is not published until the changelog has a real top-level
# section for it (`A1-321`). The optional path exists so the parser can be
# exercised against fixtures; release.sh always checks this repository's file.

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
version="${1:-}"
changelog="${2:-${REPO_ROOT}/CHANGELOG.md}"
display="${2:-CHANGELOG.md}"

if [[ -z "$version" ]]; then
    echo "ERROR: check-release-changelog needs the release version." >&2
    exit 2
fi

if [[ ! -f "$changelog" ]]; then
    echo "ERROR: release '${version}' cannot find ${display}." >&2
    echo "       Write its '## [${version}]' section before publishing." >&2
    exit 1
fi

# A grep for the heading would accept one inside a fenced example. Track fenced
# blocks so the gate answers the Markdown question it states: whether the file
# has a section, not whether those bytes occur somewhere.
if ! awk -v wanted="## [${version}]" '
    function marker_length(text,    char, i) {
        char = substr(text, 1, 1)
        if (char != "`" && char != "~") return 0
        for (i = 1; substr(text, i, 1) == char; i++) {}
        return i - 1
    }

    {
        line = $0
        sub(/\r$/, "", line)
        first = match(line, /[^ ]/)
        text = first == 0 ? "" : substr(line, first)
        markers = first > 0 && first <= 4 ? marker_length(text) : 0

        if (!in_fence && markers >= 3) {
            in_fence = 1
            fence_char = substr(text, 1, 1)
            fence_length = markers
            next
        }
        if (in_fence) {
            tail = substr(text, markers + 1)
            if (markers >= fence_length && substr(text, 1, 1) == fence_char &&
                tail ~ /^[[:space:]]*$/) {
                in_fence = 0
            }
            next
        }
        if (substr(line, 1, length(wanted)) == wanted) {
            suffix = substr(line, length(wanted) + 1)
            dated = suffix ~ /^ - [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/
            if (suffix == "" || dated) found = 1
        }
    }

    END { exit found ? 0 : 1 }
' "$changelog"; then
    echo "ERROR: release '${version}' has no '## [${version}]' section in ${display}." >&2
    echo "       Close the Unreleased entries into that section before publishing." >&2
    exit 1
fi

echo "==> ${display} has a [${version}] release section"
