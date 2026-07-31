#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Uniform across every compiling gate so the shared CARGO_TARGET_DIR cache is
# not invalidated by a differing flag set between jobs.
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

echo "==> cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings
