#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Deny compiler warnings without changing rustc's artifact fingerprint.
# Rustdoc warnings remain a separate explicit policy below.
export CARGO_BUILD_WARNINGS=deny

# Intra-doc links, lint-checked. Nothing else here catches a broken one: rustdoc
# resolves `[`Type`]` against the real item graph, so a renamed type leaves a
# link that compiles, passes clippy, and is dead on the rendered page.
#
# This gate arrived late, and the evidence for it is that seven broken links had
# accumulated across four crates by the time anyone ran the command (found while
# reviewing A1-030). Every one of them dated from a rename or a module move that
# every other gate was happy with.
declare -a scope=(--workspace)
declare -a packages=()
while [[ "$#" -gt 0 ]]; do
    [[ "$1" == "--package" && "$#" -ge 2 && -n "$2" ]] || {
        echo "usage: ci/scripts/doc.sh [--package NAME]..." >&2
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

echo "==> cargo doc ${scope[*]} --no-deps (RUSTDOCFLAGS=-D warnings)"
RUSTDOCFLAGS="-D warnings" cargo doc "${scope[@]}" --no-deps
