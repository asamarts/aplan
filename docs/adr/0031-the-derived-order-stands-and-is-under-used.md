# ADR-0031: The derived order stands, and the doubt about it was measured on an outlier

Date: 2026-09-09
Status: Accepted
Tasks: `A1-231`
Extends ADR-0017 (ordering is derived) and ADR-0025 (a discretionary order is a field). Supersedes
nothing: ADR-0017's deferral was already superseded by ADR-0025 for the pairwise case, and this
record answers the *second* question rather than reopening the first.

> **Correction (2026-09-09), from this record's own audit.** The decision stands unchanged; three
> figures in "What the data actually shows" were taken against the wrong tree and are corrected
> below. The `aplan` rows were measured in `a local aplan checkout`, a checkout sitting at an
> older commit and carrying other sessions' unfinished task files, rather than at the revision this
> ADR ships in. Re-measured at `b909e08`, this plan has 45 live non-milestone tasks and 42 ready, not
> 57 and 54. The aggregate becomes **182 of 207 (88%)** rather than 194 of 219 (89%), and the edges
> per task -- the ratio the argument actually rests on -- is **0.04 either way**. One sentence was
> also false as written: `p009` records *less* ordering information than this plan (0.00), so the
> claim is now made about plans of comparable size, where it holds.

## Context

ADR-0017 derived `ready`'s order from three terms -- what a task releases, whether it is on the
critical path, then its effort -- and deferred a declared priority field because two of its inputs
did not discriminate: "unblocking power counts milestone membership, so 35 of 38 open tasks score
identically. An ADR that named a precedence over inputs that do not discriminate would be deciding
on air."

ADR-0025 then added `prefer_after`, a discretionary order that never blocks, has exactly one
consumer, and reports a preference that says nothing as a finding. So whether an unverifiable
judgement may exist in this model is settled. `A1-231` asks the narrower question ADR-0025 does not
answer: `prefer_after` is pairwise, and saying "this one matters" against forty-seven others is
forty-seven edges re-edited on every filing. Should there be a *second* declared signal that is not
pairwise -- `A1-232`'s ordered tag family, read by `ready` and nothing else?

## What the data actually shows

**The first two terms are inert almost everywhere.** Measured on 2026-09-09 across the twelve plans
on this host that offer any ready work -- this one at `b909e08`, the rest as they stand -- 207 ready
tasks in total: **182 of them, 88%, sit in the `(releases = 0, critical = false)` bucket**, where the
precedence falls through to effort descending and then id. Per plan it runs from 50% (`p00025`,
`p0000014`) and 55% (`p003`) to 100% (`aplan`, `p00020`, `p009`).

That is the case for a new signal, and it is the whole of it. Two measurements refuse it.

**This repository's plan is an outlier, and it is the plan the doubt was measured on.** Counting
declared dependency edges between live non-milestone tasks, per live task:

| plan | live tasks | dependency edges | edges per task |
|---|---|---|---|
| `p003` | 894 | 1,883 | 2.11 |
| `p00013` | 129 | 199 | 1.54 |
| `p00016` | 158 | 221 | 1.40 |
| `p0000014` | 148 | 202 | 1.36 |
| `p00002` | 73 | 78 | 1.07 |
| `p00023` | 139 | 96 | 0.69 |
| `p0000007` | 19 | 12 | 0.63 |
| `p0000004` | 31 | 17 | 0.55 |
| `p00020` | 57 | 23 | 0.40 |
| `p00025` | 3 | 1 | 0.33 |
| **`aplan`** | **45** | **2** | **0.04** |
| `p009` | 18 | 0 | 0.00 |

Of the plans with more than twenty live tasks, every one records ten to fifty times more ordering
information than this one does. The two below it are `p009` and `p00025`, at eighteen and three live
tasks, which is small enough that a flat order says nothing either way.
`A1-231` generalised from the least-connected plan on the host to the model, and `A1-233` had
already found the same shape one level down: the term is flat here because this plan declares almost
no dependencies, not because the tool counts the wrong thing.

**And a derived term is sitting unused.** `releases` counts only *direct* dependents for which this
task is the sole remaining blocker. Counting instead how much live work sits transitively behind a
task -- over dependency edges only, which is exactly the distinction `edge_kind` draws (the pre-public repository-boundary decision,
`A1-216`) -- discriminates where the direct count cannot:

| plan | ready | distinct direct values | distinct transitive values | ready tasks with a non-zero transitive count |
|---|---|---|---|---|
| `p00016` | 24 | 2 | 4 | **21** |
| `p00013` | 13 | 1 | 5 | 12 |
| `p003` | 11 | 3 | 9 | 8 |
| `p00023` | 45 | 4 | 9 | 18 |
| `p00002` | 13 | 2 | 7 | 8 |
| `p0000004` | 19 | 2 | 4 | 7 |
| `p0000007` | 11 | 3 | 4 | 4 |
| `aplan` | 42 | 1 | 2 | 1 |
| `p00020` | 15 | 1 | 1 | 0 |
| `p009` | 10 | 1 | 1 | 0 |

Seven of ten plans gain a signal that is *derived*, on plans where the declared one would have been
sold as the only option. `p00016` is the sharpest: 23 of its 24 ready tasks carry no direct signal,
and 21 of 24 carry a transitive one.

## Decision

**The derived order stands, and a second declared signal is deferred again.**

Not refused: ADR-0025 settled that such a signal may exist, and the measurement above is a real
argument that something is missing. Deferred, because deciding it now would be deciding on air in
exactly the sense ADR-0017 meant. The tool has a term it has not counted, and adopting an opinion
field while a derivation is unused would trade the one thing this project can check for the one
thing it cannot, without having tried the first.

The order to do it in:

1. **Count what sits transitively behind a task**, over dependency edges only, as a derived term.
2. **Re-take the measurement above.** If plans of ordinary edge density still show most ready tasks
   sharing a bucket, the derivation has been given its chance and the declared signal is justified
   on evidence rather than on the shape of one sparse plan.

## Consequences

**`A1-232` is dropped**, which is the criterion `A1-231` set for this outcome. Its design is not
wrong and the file is kept for the record: an ordered tag family under ADR-0025's discipline
remains the right shape *if* step 2 says a declared signal is needed. Dropping it is a statement
about sequence, not about quality.

**`ready` keeps saying what it cannot answer.** Where a plan genuinely records no ordering
information, `Rank::because` already returns "nothing to rank it by; ordered by id" rather than
implying a judgement. That is the honest reading of a sparse plan, and it satisfies ADR-0017's own
consequence that a reader can always ask why a task is where it is.

**And this repository should expect its own `ready` to stay flat**, because 0.04 edges per task is
not a defect the ranking can fix. A plan that declares no order has not been asked to.

## Step 2, taken (2026-09-09, `A1-298`)

The term shipped, and the measurement was re-taken with it in place. It is the longest chain of live
work waiting behind a task, over edges where neither end is a milestone -- one rule that excludes
membership in both directions and the constraint a member inherits from the cut its own cut follows.
Depth rather than a count of descendants, because counting distinct descendants exactly needs a set
per node and the two were compared first: inserted into the precedence they put *identically* many
ready tasks in the modal bucket, plan for plan, 143 of 213 either way.

| plan | ready | in one bucket before | after |
|---|---|---|---|
| `p003` | 11 | 54% | **27%** |
| `p00002` | 13 | 84% | **38%** |
| `p00016` | 25 | 96% | **40%** |
| `p00013` | 13 | 92% | **46%** |
| `p00023` | 45 | 82% | 60% |
| `p0000004` | 19 | 78% | 63% |
| `p0000007` | 11 | 81% | 63% |
| `aplan` | 47 | 100% | 97% |
| `p00020`, `p009` | 25 | 100% | 100% |
| **all** | **213** | **88%** | **67%** |

Re-measured after `A1-298`'s audit moved the term to sit *after* the critical path rather than before
it: 88% to 66% of 212 ready tasks, the one-task difference being `A1-298` itself closing. Where the
term sits does not change which bucket a task lands in, only which of two tied tasks is offered
first, so this conclusion does not depend on that placement.

**This answers the condition above, and it answers it against the declared signal.** The test was
whether plans of ordinary edge density would *still* show most ready tasks sharing a bucket. They do
not: the four densest plans read 27%, 38%, 40% and 46%, all under half. The derivation was given its
chance and it took it.

What did not move is the three plans that record no order to derive from -- `aplan` at 0.04 edges per
live task, `p00020` at 0.40, `p009` at 0.00. No derivation can order a plan that has not said, and
`Rank::because` says so rather than implying a judgement. A declared signal would order those three,
and that remains the only case for one.

So the deferral stands, `A1-232` stays dropped, and what would reopen this is unchanged below -- with
one addition: an adopter whose plan *does* record dependencies and who still reports that `ready`
offered the wrong work. That is now a much narrower claim than it was this morning.

## What would reopen this

The measurement in step 2, on plans at ordinary edge density, still putting most ready tasks in one
bucket. Or an adopter reporting that `ready` offered the wrong work -- which no pilot log records
today, and which is the evidence this decision would most like to have.
