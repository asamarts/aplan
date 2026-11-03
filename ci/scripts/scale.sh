#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny

# The scale gate (`A1-229`). Deliberately *not* in `preflight.sh` and not in the
# per-commit workflow: it writes 24,000 task files and runs for about twenty
# seconds, which is a cost nobody keeps paying on every push. It runs nightly and
# on demand -- see `ci/README.md`, "What runs where".
#
# It asserts a ratio between cut widths rather than a millisecond budget, so it
# is not flaky on a shared runner. `A1-228` published an 11% difference between
# two plan shapes that turned out to be contention from a CI run on the private
# runner. A ratio survives that, and survives public hosted-machine variation,
# because every shape in one run is slowed together.
echo "==> cargo test -p aplan-e2e --test integration scale:: -- --ignored"
cargo test -q -p aplan-e2e --test integration 'scale::' -- --ignored --nocapture

# TUI wall-clock and resident-memory budgets need an optimized binary and a
# real PTY. They are too noisy and too costly for per-task CI, but the same
# nightly/manual lane gives them a stable Linux runner class and a bounded
# thirty-minute job (`A1-447`). The in-process gate isolates reducer, scene and
# viewport work; the serial PTY gate measures cold/warm startup and interaction.
echo "==> cargo test --release -p aplan-tui performance_tests::layered_graph_performance_gate_layout_and_semantic_hits -- --ignored --exact --nocapture"
cargo test -q --release -p aplan-tui \
    'performance_tests::layered_graph_performance_gate_layout_and_semantic_hits' -- \
    --ignored --exact --nocapture

echo "==> cargo test --release -p aplan-tui performance_tests::tui_performance_gate_render_interaction_and_graph_work -- --ignored --exact --nocapture"
cargo test -q --release -p aplan-tui \
    'performance_tests::tui_performance_gate_render_interaction_and_graph_work' -- \
    --ignored --exact --nocapture

echo "==> cargo test --release -p aplan --test integration tui_performance::tui_performance_gate_startup_interaction_and_memory -- --ignored --exact --nocapture --test-threads=1"
cargo test -q --release -p aplan --test integration \
    'tui_performance::tui_performance_gate_startup_interaction_and_memory' -- \
    --ignored --exact --nocapture --test-threads=1
