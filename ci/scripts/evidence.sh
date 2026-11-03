#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny

# Resolve the whole plan at L1/L2 against full Git history and every configured
# test catalogue. This is authoritative for the resolution it performs, but it
# is not L3 direct execution (`verify --execute`) and it does not consume or
# create the receipt-backed provider observations designed under A1-590.
echo "==> aplan verify (whole-plan L1/L2 evidence resolution)"
if cargo run -q -p aplan -- verify; then
    echo "aplan whole-plan L1/L2 evidence verification passed; L3 execution was not requested"
else
    status=$?
    echo "aplan whole-plan L1/L2 evidence verification failed; the findings are listed above" >&2
    exit "$status"
fi
