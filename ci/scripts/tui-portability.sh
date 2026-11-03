#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny

# Deliberately smaller than the workspace gate. These two targets contain the
# platform-neutral reducer/fault-injection contract and the independent real-
# PTY module in the consolidated integration binary. That binary platform-gates
# its historical Unix fixture modules instead of making Windows compile them.
cargo test -p aplan-tui --lib
cargo test -p aplan --test integration tui_portability::
