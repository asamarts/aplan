#!/usr/bin/env bash
set -euo pipefail

# Run the full CI gate bundle locally, in the same order and with the same
# scripts CI uses, so a push is green before it leaves the machine. Wired to the
# pre-push hook (ci/githooks/pre-push).
#
# Skip a specific gate (debugging a flaky one):
#   PREFLIGHT_SKIP="test dogfood" git push
#
# `coverage` is the slow one: `cargo llvm-cov` rebuilds the workspace
# instrumented into its own target directory, so it shares no artifacts with the
# gates above and adds minutes to a push. It is here anyway, because a bundle
# that is a subset of CI just moves the discovery later. Skip it deliberately
# when you know the change cannot move coverage:
#   PREFLIGHT_SKIP=coverage git push

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SKIP="${PREFLIGHT_SKIP:-}"

# Uniform flag set across all gates (matches the ci/scripts/* exports) so a
# local run shares the same build cache instead of recompiling.
export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

run_gate() {
    local name="$1"
    for skip in $SKIP; do
        if [[ "$skip" == "$name" ]]; then
            echo "==> SKIP ${name} (PREFLIGHT_SKIP)"
            return 0
        fi
    done
    echo "=================== preflight: ${name} ==================="
    "${SCRIPT_DIR}/${name}.sh"
}

run_gate fmt
run_gate clippy
run_gate doc
run_gate test
run_gate spec
run_gate style
run_gate coverage
run_gate dogfood

echo "==> preflight: all gates passed"
