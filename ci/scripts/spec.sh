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

# `gen-roadmap` gates the generated companion; this gates the hand-written
# prose beside it: ROADMAP.md's narrative and the pilot logs in pre-public adoption evidence v0.6 closed while its prose claimed seven deliverables that
# were never tasks, because nothing compared the two (A1-042, ADR-0009's deferred
# follow-up).
echo "==> cargo xtask check-docs"
cargo run -q -p xtask -- check-docs

# And the half of that gate the shipped command owns. Since `A1-165` moved cut
# membership out of `xtask` and into `aplan check-docs`, running only the xtask
# left the membership check unrun here -- the `A1-164` failure in the other
# direction, with the shipped copy as the untested one. Found when a cut gained a
# task and `preflight` stayed green (`A1-136`'s audit).
# **Every document of ours that names a task**, not the three that happened to be
# listed. `README.md`, `HLD.md`, `AGENTS.md` and `CONTRIBUTING.md` carry 105
# distinct task references between them -- 82 in the HLD alone -- and nothing read
# any of them: `xtask check-docs` above checks *ADR* citations in those files,
# which is a different question. An id that stops resolving reads as *handled*,
# which is the state this command exists to catch, and the README is the document
# an adopter meets first (`A1-283`, audit).
echo "==> aplan check-docs (references and cut membership)"
cargo run -q -p aplan -- check-docs \
    README.md HLD.md AGENTS.md CONTRIBUTING.md ROADMAP.md CHANGELOG.md docs/upgrading.md

# The agent contract's generated half. This repository's own AGENTS.md told
# agents that `.aplan.yml` accepts `commit`, `test` or `file` evidence for three
# tasks after it stopped accepting anything but `test`, because the policy was
# written out by hand in two places (`A1-038`).
echo "==> aplan export-agents-md --check AGENTS.md"
cargo run -q -p aplan -- export-agents-md --check AGENTS.md
