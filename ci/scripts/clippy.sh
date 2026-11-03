#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Deny workspace compiler warnings through Cargo policy, which does not change
# rustc's artifact fingerprint. Clippy keeps its own explicit lint cap below.
export CARGO_BUILD_WARNINGS=deny

declare -a scope=(--workspace)
declare -a packages=()
while [[ "$#" -gt 0 ]]; do
    [[ "$1" == "--package" && "$#" -ge 2 && -n "$2" ]] || {
        echo "usage: ci/scripts/clippy.sh [--package NAME]..." >&2
        exit 2
    }
    packages+=("$2")
    shift 2
done
if [[ "${#packages[@]}" -gt 0 ]]; then
    scope=()
    for package in "${packages[@]}"; do
        scope+=(-p "$package")
    done
fi

echo "==> cargo clippy ${scope[*]} --all-targets -- -D warnings"
cargo clippy "${scope[@]}" --all-targets -- -D warnings
