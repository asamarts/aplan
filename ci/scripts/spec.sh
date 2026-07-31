#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export RUSTFLAGS="${RUSTFLAGS:--D warnings}"

# The keystone gates: the committed JSON Schema and ROADMAP.status.md must be
# regenerated from the code and the plan, not hand-written. Fail on any drift.
echo "==> cargo xtask gen-schema --check"
cargo run -q -p xtask -- gen-schema --check

echo "==> cargo xtask gen-roadmap --check"
cargo run -q -p xtask -- gen-roadmap --check
