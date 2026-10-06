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
# **The instrumented objects must not outlive the commit that built them**
# (A1-352). `cargo llvm-cov` keeps them in `llvm-cov-target`, and this runner
# mounts one `CARGO_TARGET_DIR` across every job, so builds of the same crate
# from several commits accumulate there. Each carries its own coverage mapping
# for the same source file, llvm-cov merges them into one entry per filename and
# adds the counts, and the percentage is then wrong rather than missing:
# `aplan-evidence` read 64.5% against 98.1% with every test passing, because
# `src/lib.rs` was credited with 6,761 coverable lines in a 5,594-line file.
#
# `cargo llvm-cov clean` cannot do this: it refuses a target directory outside
# the project, which is exactly the arrangement here. `xtask` refuses the
# impossible count regardless, so a run that skips this is reported rather than
# quietly believed.
#
# It costs an instrumented rebuild every run. That is the price of a number that
# means the same thing on two commits, which is the only thing a floor is for.
rm -rf "${CARGO_TARGET_DIR:-target}/llvm-cov-target"

echo "==> cargo xtask coverage --check"
cargo run -q -p xtask -- coverage --check
