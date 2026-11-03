#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny

# A plan tool whose own plan does not validate has no business shipping. Keep
# policy in `.aplan.yml` and invoke the adopter surface without overrides.
# GitHub format is deliberately the only structural pass: its complete finding
# lines are readable in a terminal and become inline annotations in Actions.
# Running the human renderer as well used to parse and validate the same plan a
# second time merely to add a clean summary. The explicit result below supplies
# that summary without duplicating the check.
echo "==> aplan check --format github (structural findings and annotations)"
if cargo run -q -p aplan -- check --format github; then
    echo "aplan structural check passed; evidence resolution was not run by this gate"
else
    status=$?
    echo "aplan structural check failed; the complete findings are listed above" >&2
    exit "$status"
fi
