# ADR-0023: A duration is derived, not declared

Date: 2026-08-17
Status: Accepted
Tasks: `A1-147`
Extends ADR-0017 (ordering is derived, not declared) to a second quantity, and stays inside ADR-0025
(a discretionary order is a field), which is the one place a declaration beat a derivation.
Supersedes nothing.

## Context

The sixth pilot is managed by a number `aplan` could not hold: **85 weeks**. It is not asserted, it
is arrived at -- 649 engineer-days divided by 7.5 effective days a week, where 7.5 is five days
times an assumed 1.5x AI multiplier -- and every cut's week range comes from the same division
(pre-public adoption evidence item 2).

The inputs were all in the model already. `effort_days` is a field, cut membership is a field, and
`critical-path` is effort-weighted. The *conversion* was not, so the schedule lived in a prose table
that nothing gated, inside a document whose whole argument is that ungated facts drift. **It drifted
three times in nine audits**: cut ranges that contradicted the dependency graph, an effort total
that implied a two-person team in a one-person project, and a calendar never rescaled after scope
grew 20%. The project wrote its own gate (`DN0-18`) to parse the ranges back out of the document,
which is the third time an adopter has built something this tool could own.

The log proposed an arithmetic check: *given `effort_days`, a declared throughput and cut
membership, does a declared cut duration match the effort it contains?* Falsifiable today, needs no
dates.

## Decision

**Derive the duration. Check nothing.**

One number is declared, in `.aplan.yml`:

```yaml
effort_days_per_week: 7.5
```

Every surface then divides the effort it is already printing by it. `summary` reports the plan's
weeks and the weeks remaining; `roadmap` states the throughput once and gives each cut its own
`effort` and `weeks (n left)`, in the human view, the JSON and the markdown table. Nothing is
stored, so there is nothing to check and no rule is added.

Three consequences follow, and they are the decision:

1. **A duration cannot disagree with the effort beside it**, because it is not a second copy of it.
   The pilot's third drift -- a calendar never rescaled after 20% more scope -- is not caught, it is
   unrepresentable: growing the plan moves the number by exactly the growth, unattended
   (`growing_the_plan_rescales_the_duration_with_no_edit`).
2. **The one assumption is named and visible.** The pilot's second drift, an effort total implying a
   two-person team, is a claim about throughput hiding inside a schedule that was reached by
   dividing by it silently. It is now the single declared number, and a wrong one is wrong *loudly*,
   across every row at once.
3. **The first drift is refused rather than fixed.** Cut ranges contradicting the dependency graph
   requires start weeks, and this tool does not model when a cut starts. A plan's order comes from
   `ready` (ADR-0017, ADR-0025) and the answer to "when" is not one this tool can hold. An adopter
   who still publishes a calendar keeps their own gate, and it has less to gate.

**Against ADR-0017, explicitly.** That decision refused a declared rank because a rank is an opinion
nothing can check, and ranking is derivable from the graph. A declared duration is the same shape
with a sharper edge: it is not merely an unverifiable opinion, it is a *derivable quantity written
down by hand*, so it can be checked -- and the version of this feature that checks it would be a
rule that fires whenever someone edits the plan and not the number, whose only remedy is to copy the
derived value back in. That is a gate on a chore this tool created. ADR-0025 is not a
counter-example: `prefer_after` declares a preference between two orderings the graph leaves free,
which is information nothing can derive, where a duration is information nothing needs to declare.

**A week is a unit of capacity, not a position in a calendar.** "Eleven weeks of work at your
declared throughput" is arithmetic over the plan. "Week 46" is a claim about the future that nothing
can verify until it arrives, and a tool whose thesis is that a status is a claim rather than a proof
does not get to ship a second, weaker kind of claim. No dates, no start weeks, no ranges, and no
scheduler.

**A throughput nothing can be divided by is refused at load** -- zero, negative, non-finite -- with
its own error naming the key. Silently ignoring a declared key is the drift this project exists to
name, and an `inf` rendered as a duration is worse than none. Declare nothing and every surface says
nothing about time, while still reporting the effort, which was always knowable.

## Consequences

This repository declares one, measured rather than assumed: **27** effort days a week, from 151.0
effort days closed over the 5.6 calendar weeks between the first commit and this one. What that
number says is that the estimates are calibrated to a human week and this repository is not one --
the pilot assumed 1.5x over five days, and this measures 5.4x its own estimate scale. It is a
capacity, not a promise, the window is short, and it is re-derivable by the same division at any
time. A repository that has not measured its own should declare nothing rather than borrow this one.

`roadmap` reports a cut's effort now, and `summary --format json` the effort already done, whether
or not a throughput is declared. That is a widening no config key proves, and it carries the name
`rollup.carries_effort`.

Rendering rounds to one decimal, and one decimal cannot say "some, but under a tenth of a week". It
says `<0.1` rather than `0.0`, because the audit found a cut with a half-day of work left reporting
`0.0w (0.0w left)` at this repository's own throughput, which is the answer a *finished* cut gives.
The JSON is unrounded.

The effort column has to add up to the plan, which it did not when this shipped: 21.0 of this
repository's 185.5 effort days were held by 18 tasks in no cut and appeared in no row. Cross-cutting
work carries its own effort and duration now, and `roadmap`'s header states the total, so the two
rollups reconcile (`the_effort_column_adds_up_to_the_plan`).

A release borrows the effort of the cuts it closes, as it already borrowed their task counts
(ADR-0022), so summing the column across every row double-counts. `release` is on all three surfaces
for exactly that reason.

What this does not do, and what would need its own decision: a cut's weeks are its total effort
divided by throughput, which assumes that effort is serialisable at that capacity. Where a cut has
several independent chains and a team to run them, the honest floor is its critical path, which
`roadmap` already prints beside it. Both numbers are on the row; the tool does not pretend to know
which one binds.
