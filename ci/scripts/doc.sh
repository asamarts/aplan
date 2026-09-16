#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Uniform across every compiling gate so the shared CARGO_TARGET_DIR cache is
# not invalidated by a differing flag set between jobs.
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

# Intra-doc links, lint-checked. Nothing else here catches a broken one: rustdoc
# resolves `[`Type`]` against the real item graph, so a renamed type leaves a
# link that compiles, passes clippy, and is dead on the rendered page.
#
# This gate arrived late, and the evidence for it is that seven broken links had
# accumulated across four crates by the time anyone ran the command (found while
# reviewing A1-030). Every one of them dated from a rename or a module move that
# every other gate was happy with.
echo "==> cargo doc --workspace --no-deps (RUSTDOCFLAGS=-D warnings)"
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
