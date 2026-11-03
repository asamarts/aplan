# A plan for the action self-test

`.github/workflows/action-selftest.yml` runs `action.yml` against this directory
rather than against `aplan/`, the repository's real plan, and the reason is worth
stating because the obvious choice is the other one.

The action installs a **pinned release**. This repository's `.aplan.yml` tracks
`main`, and has grown eight keys since the newest tag: pointing the pinned binary
at it makes the job a compatibility test between `main`'s configuration and the
last release, which is a real thing to know and is not what a self-test of
`action.yml` is for. It also decays -- every new key is another way for the job to
go red for a reason nothing in `action.yml` caused, until nobody reads it. That
happened: the workflow was green on 2026-08-01, was not triggered again until
2026-09-05, and failed on `unknown field effort_days_per_week` (`A1-278`).

So this plan is deliberately **written to the oldest release the workflow pins**,
and carries nothing a `v0.2.0` binary cannot parse. What the workflow proves with
it is the adoption path itself: install, checksum, the pinned version, both
commands, an explicit `root`, and a format other than the default.

The repository's own plan is not going unchecked. `ci/scripts/structural.sh` runs
`aplan check` on every push, and the separate full-history `ci/scripts/evidence.sh` runs `aplan
verify` nightly, on dispatch and before release, with the binary built from the commit under test.
Those are the stronger checks and the right place for them.

Raising the pinned version here is safe at any time. Lowering the plan's demands
is not: keep it to fields the pinned release knows.
