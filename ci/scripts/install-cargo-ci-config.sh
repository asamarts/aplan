#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
committed_config="$REPO_ROOT/ci/cargo-ci-config.toml"
destination="${1:-${CARGO_HOME:-$HOME/.cargo}/config.toml}"
legacy="${destination%.toml}"

mkdir -p "$(dirname "$destination")"
if [[ "$legacy" != "$destination" && ( -e "$legacy" || -L "$legacy" ) ]]; then
    echo "error: legacy Cargo config shadows ci/cargo-ci-config.toml" >&2
    exit 1
fi
if [[ -e "$destination" || -L "$destination" ]]; then
    if [[ ! -f "$destination" || -L "$destination" ]] \
        || ! cmp -s "$committed_config" "$destination"; then
        echo "error: hosted Cargo config differs from ci/cargo-ci-config.toml" >&2
        exit 1
    fi
else
    install -m 0644 "$committed_config" "$destination"
fi
