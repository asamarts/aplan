#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

# A plan tool whose own plan does not validate has no business shipping. No
# flags: this must exercise the DEFAULTS, because the defaults are what users
# get. `verify` needs full git history (the workflow checks out fetch-depth: 0;
# a local clone already has it).
#
# All three run even if an earlier one fails (the diagnostic the workflow's old
# `if: always()` steps gave), then the aggregate exit code is returned: the
# `--format github` pass emits inline annotations, the plain `check` prints a
# human summary (github format is silent when clean), and `verify` resolves the
# cited evidence against git.
rc=0

echo "==> aplan check --format github (annotations)"
cargo run -q -p aplan -- check --format github || rc=1

echo "==> aplan check (human summary)"
cargo run -q -p aplan -- check || rc=1

echo "==> aplan verify (resolve the plan's own evidence against git)"
cargo run -q -p aplan -- verify || rc=1

exit "$rc"
