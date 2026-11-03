#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

export CARGO_BUILD_WARNINGS=deny

# The validation manifest is the one source for hook and workflow gate
# membership. Its generated shell table and both consumers must agree.
echo "==> cargo xtask validation --check"
cargo run -q -p xtask -- validation --check

# The keystone gates: the committed JSON Schema and ROADMAP.status.md must be
# regenerated from the code and the plan, not hand-written. Fail on any drift.
echo "==> cargo xtask gen-schema --check"
cargo run -q -p xtask -- gen-schema --check

echo "==> cargo xtask gen-roadmap --check"
cargo run -q -p xtask -- gen-roadmap --check

# The TUI's user reference is a generated view of the same closed action,
# mutation and CLI-boundary catalogues used by dispatch, help and the palette.
# A hand-maintained key table would immediately become the second copy this
# project exists to reject.
echo "==> cargo xtask tui-reference --check"
cargo run -q -p xtask -- tui-reference --check

# `rust-version` is a consumer contract and nothing checked it: it read 1.85
# while the locked graph needed 1.88, so the reviewed revision did not build
# with the compiler its own metadata promised (A1-327, pre-public adoption evidence
# item 15). This is the half that needs no toolchain -- every package declares
# the compiler it wants, so the floor is arithmetic over `cargo metadata`.
#
# It does NOT prove this workspace's own source compiles at that floor, which
# needs the compiler itself and so needs the runner image to carry it. A1-330
# is that half; saying so here rather than letting the gate's name imply more
# than it checks.
echo "==> cargo xtask msrv"
cargo run -q -p xtask -- msrv

# Do the install pins in the documents that instruct an adopter name the newest
# release? HLD section 17 promised this and it did not exist (A1-318,
# ADR-0039). Narrower than that promise on purpose: the unit is an occurrence,
# not a file, and a record of an instruction is not an instruction -- both
# measured on this repository's own text, and both written up in the script.
echo "==> ci/scripts/check-version-pins.sh"
bash ci/scripts/check-version-pins.sh

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
# **Every document this repository declared**, expanded from `docs.paths` and
# `docs.membership[].doc`. The old argument list was a second configuration:
# adding a pilot or another plan-bearing document did not add it to the gate.
# The no-argument invocation is the adopter surface and the self-hosted proof of
# `A1-203`; explicit paths remain the narrow one-off form.
echo "==> aplan check-docs (declared references and cut membership)"
cargo run -q -p aplan -- check-docs

# The agent contract's generated half. This repository's own AGENTS.md told
# agents that `.aplan.yml` accepts `commit`, `test` or `file` evidence for three
# tasks after it stopped accepting anything but `test`, because the policy was
# written out by hand in two places (`A1-038`).
echo "==> aplan export-agents-md --check AGENTS.md"
cargo run -q -p aplan -- export-agents-md --check AGENTS.md
