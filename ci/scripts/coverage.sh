#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

# Per-crate line-coverage floors (A1-046). The floors live in `coverage.json`,
# committed, so lowering one is a diff a reviewer sees rather than a silent decay;
# the gate also fails when a crate has outgrown its floor by more than the ratchet
# slack, because a floor nobody raises stops being a floor.
#
# This measures which lines ran, not which claims were checked, and it is the
# weaker of the two testing gates on purpose: `APLAN023` shipped with full line
# coverage and three defects in the inputs next to the one it was written for. The
# rule-level audit (`crates/aplan-e2e/tests/rule_tests.rs`, run by the test job)
# and AGENTS.md carry the rest.
#
# Its own job because the instrumented rebuild does not share artifacts with the
# ordinary `cargo test` build: putting it in the test job would double that job's
# time for every push.
echo "==> cargo xtask coverage --check"
cargo run -q -p xtask -- coverage --check
