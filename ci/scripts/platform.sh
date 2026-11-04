#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny

# Compile and lint every target so platform-specific cfg branches and test
# harnesses cannot rot on a supported host. Run library and binary unit tests
# without pretending that the historical Unix-only integration fixtures are a
# Windows contract. The independent PTY journey below is intentionally portable.
cargo clippy --workspace --all-targets -- -D warnings
# The authoritative nextest gate serializes process-control tests through its
# workspace mutex. Windows has no admitted nextest artifact yet, so direct
# libtest uses one thread rather than reintroducing those shared-signal races.
# Keep running later workspace test binaries after a failure so one cold hosted
# run reports every portability gap it can reach.
cargo test --workspace --lib --bins --no-fail-fast -- --test-threads=1
cargo test -p aplan --test integration tui_portability::
