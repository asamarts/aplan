#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

# The scale gate (`A1-229`). Deliberately *not* in `preflight.sh` and not in the
# per-commit workflow: it writes 24,000 task files and runs for about twenty
# seconds, which is a cost nobody keeps paying on every push. It runs nightly and
# on demand -- see `ci/README.md`, "What runs where".
#
# It asserts a ratio between cut widths rather than a millisecond budget, so it
# is not flaky on a shared runner. That matters here specifically: this box runs
# the runner, and `A1-228` published an 11% difference between two plan shapes
# that turned out to be contention from a CI run on the same machine. A ratio
# survives that, because every shape is slowed together.
echo "==> cargo test -p aplan-e2e --test scale -- --ignored"
cargo test -q -p aplan-e2e --test scale -- --ignored --nocapture
