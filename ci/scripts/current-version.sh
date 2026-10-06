#!/usr/bin/env bash
set -euo pipefail

# Print the version this workspace ships today, and nothing else.
#
# The single source of truth is `[workspace.package].version` (HLD section 17).
# One reader, because there are now three callers -- `ci/scripts/release.sh`,
# which checks a tag against it, `version.current` in `.aplan.yml`, which is how
# `aplan version` learns the number it does arithmetic on, and the release
# script's own plan check -- and three copies of this awk would be the drift
# `ci/scripts/check-version-pins.sh` exists to catch, one file further in.

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

version="$(awk '
    /^\[workspace\.package\]/ { in_block = 1; next }
    /^\[/                     { in_block = 0 }
    in_block && /^version[[:space:]]*=/ {
        gsub(/[",]/, "", $3); print $3; exit
    }
' "${REPO_ROOT}/Cargo.toml")"

if [[ -z "$version" ]]; then
    echo "ERROR: could not read [workspace.package].version from Cargo.toml" >&2
    exit 1
fi

echo "$version"
