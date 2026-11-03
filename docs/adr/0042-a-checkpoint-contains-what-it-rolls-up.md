# ADR-0042: A checkpoint contains what it rolls up, and says so in a field

Date: 2026-09-23

Status: Accepted. Generalises ADR-0022's release; supersedes its decision 4 ("read one level
through"); superseded in part by ADR-0046, which keeps one task-membership level but recognises
the built-in phase reporting coordinate.

## Context

`A1-317`'s audit built a four-level plan -- a release over a cut over a sub-cut over the
work -- and reported three defects: an `APLAN021` on the intermediate cut, a rollup reading
`0.0d` above the sub-cut, and a mislabelled release. Auditing `A1-320` found the first two
were a misreading of ADR-0013, which decided that **a milestone listing another milestone in
`blocked_by` sequences the two cuts** and that the edge is "already excluded from cut
membership by the roadmap's member filter". Measured: two sequenced cuts keep their own work
(2.0d and 3.0d) and do not merge, which is that decision working.

So there was no traversal bug. What there is, is worse and quieter: **the same edge means
two things and nothing says which.** An author who writes `M-C1 blocked_by [M-S1]` intending
containment gets sequencing, a rollup of `0.0d` over real work, and a warning that their cut
is empty. Nothing in the plan, the output or the diagnostics uses the word "sequence". That
is the shape ADR-0025 settled once: when one edge means two things, add a field.

## Decision

**1. A milestone may declare `contains: [<milestone id>...]`.** Its own list, alongside
`blocked_by` and `ratified_by`, exactly as ADR-0036 split ratification out rather than
overloading a list. Listing a milestone there makes it a **child**; listing it in
`blocked_by` leaves it a **predecessor**, which is what ADR-0013 decided and what stays true
of every plan written so far.

**2. A checkpoint's rollup is the work beneath it, at any depth, counted once.** This
supersedes ADR-0022's "read one level through". That restriction existed because a release
of releases would double-count; a walk that visits each descendant once cannot, and the
recursion terminates because `contains` is acyclic and refused otherwise.

**3. A release is now the special case rather than the mechanism.** ADR-0022 gave a release
members by reading through its `blocked_by` milestones. That was containment without a name
for it. A release declares `contains` like anything else, and the `release` tag keeps its
other jobs -- the `APLAN021` exemption, exclusion from cut totals, and `closes`.

**4. A container is not empty.** `APLAN021` exempts a milestone that declares `contains`,
for the reason it already exempts a release: the shape is endorsed by this ADR, and warning
on it would be the rule being wrong about a model the project chose.

## What this is not

**It is not a second modelled grouping level, and ADR-0014 stands.** That decision is that
"the cut, a milestone meta-task, remains the only grouping the model knows", and it is
untouched here: **a task still belongs to exactly one cut**, the one that lists it. A
checkpoint does not group tasks; it *borrows* the work of what it contains, which is the
word ADR-0022 already used for a release and the reason `Cut::release` tells a consumer
which rows to skip when totalling. Nesting checkpoints changes what can be rolled up, not
what a task belongs to.

The test of that claim is mechanical: after this, `aplan why <task>` must still name exactly
one cut for any task, at any depth.

## What `contains` must satisfy, found by attacking it

The decision above says containment is walked and rolled up. It did not say what makes a
containment *walkable*, and four shapes passed `check` cleanly on the day this shipped
(`A1-333`, audit). `APLAN047` refuses each, and the reasons are not interchangeable:

- **It may not name itself.** `contains_cycle` skips a self-edge, copied from
  `ratification_cycle`, where the skip is right because `APLAN006` reports a `blocked_by`
  self-loop. A `contains` self-edge is not a `blocked_by` edge, so nothing reported it.
- **It must also be in `blocked_by`.** A part unfinished means the whole is unfinished, and
  the gate that says so is `APLAN007`'s, which keys on `blocked_by`. Without this a
  checkpoint sits at `done` over an open child and prints `[done] 0/1 done` beside itself --
  the shape `A1-306` found for `ratified_by`, and it needs a rule for the same reason.
- **A container may not also hold work**, which extends to every checkpoint the sentence
  ADR-0022 already made about a release. This is the one that makes the arithmetic wrong
  rather than merely untidy: the rows overlap, the header skips a borrowing row whole, and
  work the container owned alone vanishes. Measured at `1 task(s), 1.0d effort` for a plan
  with two tasks and three days.

**So every number this tool prints about a plan is correct for a plan that passes `check`,
and these rules are what make that true here.** `Cut::borrows` is a property of a row, and it
is only a sound one because a row cannot both borrow and own.

## Consequences

A plan may state "this quarter contains these three workstreams" and get a real number for
the quarter. The cost is a third edge list on the one node kind that may carry it, a rule
refusing a `contains` cycle, and a rollup that is a walk rather than a lookup -- and every
consumer that sums rows now has two kinds of borrowing row to skip rather than one, which
`Cut::release` alone can no longer express.
