#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny
cargo_command="${APLAN_CARGO:-cargo}"

echo "==> pinned cargo-deny"
ci/scripts/install-cargo-deny.sh check

echo "==> cargo-deny advisories, bans, licenses and sources"
cargo-deny --locked check advisories bans licenses sources

echo "==> metadata-derived product dependency audit"
"$cargo_command" run -q -p xtask -- dependency-audit
