#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny

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
# **The instrumented objects must not be shared between builds** (A1-457).
# `xtask` resolves Cargo's target and build directories through `cargo metadata`,
# including an external `CARGO_TARGET_DIR`, then atomically allocates private
# `llvm-cov-target` children for this run. Overlapping local measurements cannot
# delete or contribute mappings to one another, and identification, allocation,
# or cleanup failure stops the gate. Keeping this inside `xtask` makes the local
# command and CI the same operation; a shell default here previously cleaned CI
# while leaving local measurements vulnerable to merged mappings.
#
# It costs an instrumented rebuild every run. That is the price of a number that
# means the same thing on two commits, which is the only thing a floor is for.
# The xtask also pins libtest to four threads. Instrumentation makes each test
# and child materially more expensive; host-sized fan-out starved both provider
# readiness and an already-idle real-terminal session. The ordinary test gate is
# deliberately unaffected (`A1-603`).

echo "==> cargo xtask coverage --check"
cargo run -q -p xtask -- coverage --check
