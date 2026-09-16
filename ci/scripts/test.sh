#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

# The CLI-surface audit resolves against invocations that actually happened
# (`A1-208`), and the spawn helper records them when told where (`A1-207`). It is
# recorded from the suite run this gate already does rather than from a second
# one: this repository pays for its own CI (the pre-public repository-boundary decision), and a gate that doubles
# the test time to measure the test time is one nobody keeps.
# Absolute, because a test binary runs with its *package* as the working
# directory and a relative path would land under `crates/aplan/`.
#
# And unique per run. A fixed path is shared state: two runs in one checkout --
# a second terminal, or a preflight beside a bare `test.sh` -- have one truncate
# the other's log mid-flight, and the audit then reports two thirds of the
# surface unexercised. Which is exactly what it did, once, and a gate that can
# accuse the suite of something the suite did not do is worse than no gate.
INVOCATIONS_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/target}"
mkdir -p "$INVOCATIONS_DIR"
INVOCATIONS="$(mktemp "$INVOCATIONS_DIR/cli-invocations.XXXXXXXX.jsonl")"
trap 'rm -f "$INVOCATIONS"' EXIT
export APLAN_TEST_INVOCATIONS="$INVOCATIONS"

echo "==> cargo test --workspace"
cargo test --workspace

echo "==> cargo xtask cli-surface --check"
cargo run -q -p xtask -- cli-surface --check
