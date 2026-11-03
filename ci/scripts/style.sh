#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny

# House style: no typographic tells (fancy dashes, curly quotes, ellipsis), no
# non-ASCII dash variants, no invisible/exotic-space characters.
echo "==> cargo xtask lint"
cargo run -q -p xtask -- lint
