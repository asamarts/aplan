# ADR-0030: Membership stays an edge, and ADR-0008 stands

Date: 2026-09-08
Status: Accepted
Tasks: `A1-228`
Confirms ADR-0008 (milestones as `blocked_by` meta-tasks) rather than superseding it. Supersedes
nothing.

> **Correction (2026-09-08), after an adversarial audit of `A1-228`.** The decision stands and is
> strengthened; two numbers in "What the data actually shows" were wrong and are corrected below.
> The `graph` column compared a one-line refusal (shape A, whose only edges are grouping and are
> excluded by default) against a 25,010-node render (shape B, which has no edges at all), so it was
> not a like-for-like measurement. And the 11% `check` gap at ten cuts did not reproduce: this host
> runs the project's self-hosted CI runner, the original timings were taken while a run was
> executing on it, and re-measuring the identical fixture with the runner idle gives 755ms for
> shape A against 783ms for shape B -- the opposite sign. Measured on a fair fixture, with real
> dependency edges so both shapes render a comparable graph, the two are within 1.6% on every cell.
> The corrected tables are in "What the data actually shows"; nothing else in this ADR changes.

## Context

`A1-228` was a spike, not a task, because it proposed reopening an accepted ADR. ADR-0008 makes a
cut a milestone meta-task whose `blocked_by` lists every member, and three separately-found
measurements were traced back to that one choice:

| symptom | measurement when filed |
|---|---|
| the graph cannot be drawn (`A1-216`) | three quarters of this plan's edges are membership |
| `check` is quadratic (`A1-227`) | 25,000 tasks in 10 cuts, 24,992ms against 942ms in 2,500 cuts |
| concurrent writes lose work (`A1-226`) | 24 agents against one cut lost 1 and 2 memberships |

The alternative was one line on the task -- `cut: v0.9` -- and a milestone that lists nothing.

## What the data actually shows

**All three symptoms now have shipped fixes, and not one of them moved the model.**

`A1-216` derived grouping from the pair rather than changing the model: `aplan_core::edge_kind`
answers whether an edge is grouping or constraint, and `graph` excludes grouping by default. Measured
on a 25,000-task plan in ten cuts, the default view is *identical* under both shapes: each refuses
with "no task in view has a dependency on another", and the current shape adds the line that
`--membership` draws the 25,000 grouping edges instead. There is nothing left for a model change to
buy here.

`A1-227` found the quadratic and it was a traversal defect: an iterative DFS asked for a node's
blockers once per edge instead of once per node. Fixed, with membership untouched.

`A1-226` fixed the write loss with a compare-and-swap and an exclusive lock. Nothing is lost now; a
loser is refused.

**Both shapes were prototyped and measured**, at 25,000 tasks. Shape B is simulated with a short
scalar field on each task and an empty milestone list, which is structurally what `cut:` is:

On a fixture where every task after the first in its cut waits on the previous one, so `graph`
renders a comparable ~50,000-line graph under both shapes and neither is measuring a refusal:

| tasks | cuts | shape | `check` | `graph` |
|---|---|---|---|---|
| 25,000 | 10 | A: membership (ADR-0008) | 895ms | 831ms |
| 25,000 | 10 | B: `cut` on the task | 881ms | 817ms |
| 25,000 | 40 | A: membership (ADR-0008) | 898ms | 825ms |
| 25,000 | 40 | B: `cut` on the task | 897ms | 831ms |

**Within 1.6% on every cell, at both cut widths, on both commands.** There is no shape here to
prefer on cost. On the edgeless fixture the first version of this ADR used, re-measured with the CI
runner on this host idle, shape A reads 755ms at ten cuts against shape B's 783ms -- so the sign of
the only gap that was ever reported reverses when the machine is quiet.

The simulation is generous to shape B, which is worth stating: it carries a scalar on each task and
an empty milestone list, and does *no* membership derivation at all, while a real implementation
must build the milestone-to-members index from those fields. Shape B is therefore a lower bound and
still shows no advantage.

**And the write contention is not membership.** On a 25,000-task plan, 24 concurrent
`aplan add --cut` against one cut: 1 of 24 succeeds. Against **24 different cuts**, where no two
agents touch the same milestone at all: 1 of 24, identically. Every refusal reads `T-00000-00200.md: already exists`. The contention is the
*sequential id*, which both shapes share and neither fixes: an agent under shape B computes the same
next id and collides on its own task file. That is `A1-230`, and this spike misattributed it.

## Decision

**ADR-0008 stands. A cut's membership is the milestone's `blocked_by`, and it does not move to the
task.**

## Consequences

**The migration that is not being paid**, counted across the fifteen plans on this host rather than
estimated: 2,457 task files touched, 93 milestones losing 2,216 membership entries, a new property
on `schemas/v1/task.json` (generated from the Rust types and drift-gated, so every existing task
file changes shape), and the `cut` field of `aplan tasks --format json` going from derived to
declared under the same name -- which is worse for a consumer than a rename, and which `A1-220`
gates. The measured return on that is 1ms of 827 at this repository's shape.

**What the spike found in favour, recorded because it is real.** Under shape B a milestone's
`blocked_by` would carry only two meanings -- the cut it follows (ADR-0013) and, for a release, the
cuts it closes (ADR-0022) -- instead of three. That is a genuine clarification, and it is not worth
2,457 files and a contract change on its own.

**A single `cut:` field would have been expressible**, which is worth recording since it was the
open question: across 2,457 tasks, **zero** belong to two cuts. The twelve ids listed by more than
one milestone are all themselves *milestones*, each named by the cut that follows it and by the
release that closes it, which is ADR-0013 and ADR-0022 behaving correctly.

**ADR-0013's sequencing edge survives**, trivially, because nothing changes.

**The store stays plaintext in git**, and the spike proposed no alternative. The measurement says
a database would not address the observed cost: `git status` over 10,000 task files is 66ms
against `check`'s 308ms at 10,000 tasks, so the substrate was never the slow part.

## What would reopen this

A plan whose cuts are wide enough for the 11% to matter -- no plan on this host has a cut above
1,000 members, and the gap is invisible at 625 -- or a task that genuinely belongs to two cuts,
which would refute the field being expressible at all rather than argue for it.
