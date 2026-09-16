# ADR-0013: A cut is sequenced by an edge it already had, and labelled by its own tag

Date: 2026-08-08
Status: Accepted
Tasks: `A1-073`, `A1-037`

## Context

Two findings, from two pilots, that turn out to be one question: what identifies
a cut, and what orders one.

**Sequencing.** ADR-0008 models a cut as a milestone meta-task `blocked_by` its
members. Members therefore cannot be `blocked_by` their own milestone, and
nothing said that one cut follows another. On a plan whose cuts are strictly
sequential this made two commands wrong rather than merely unhelpful
(pre-public adoption evidence, item 2):

```
$ aplan ready
ready: 28          # offering cut 3 and cut 4 work on day one of cut 0
$ aplan critical-path
critical path: 11.0 days across 8 task(s)      # for a 105.5-day plan
```

`ready` contradicted that plan's most important structural claim, since a
decision gate in cut 1 could cancel cuts 3 through 6, and `critical-path`
understated the project by 3.4x, which is the harder error to notice because the
number looks like an answer. It was expressible only by adding the predecessor
milestone to the `blocked_by` of every task in every later cut: 81 edges, each
derivable from the cut the task was already in.

**Labels.** Separately, a cut whose tag is not `vN.N` rendered `?` and fell back
to ordering by id. Two pilots, two vocabularies (`phase-N` and `sN`), the same
question mark (pre-public adoption evidence item 2, pre-public adoption evidence item 7).
The second occurrence inverted the reading of the first: `vN.N` is the *unusual*
case among the projects actually adopting this tool.

## Decision

**A milestone listing another milestone in `blocked_by` sequences the two cuts,
and the members of the later cut inherit the edge.**

No new field. That edge is already legal, already excluded from cut membership by
the roadmap's member filter, and today carries no meaning beyond the milestone's
own completion. What it did not do was reach the members, and members are what
`ready` offers and what `critical_path` measures. Seven such edges express what
81 hand-maintained ones did.

**A cut is labelled by its own tag**: the `vN.N` tag when there is one, otherwise
the first tag that is not the `milestone` marker. Cuts with release versions
order numerically exactly as before; cuts without them order by a natural
comparison of the label, so `s2` precedes `s10` and `phase-2` precedes
`phase-10`.

## Consequences

**Opt-in by construction.** A plan whose milestones do not block one another,
including this repository's own, is unchanged: same `ready`, same critical path,
same labels, same order. Nothing needs migrating and no existing plan changes
meaning.

**The strengthening is real and deliberate.** `M-s1: blocked_by: [M-s0]` used to
mean only "cut 1 cannot *complete* before cut 0". It now also means "no member of
cut 1 may *start* before cut 0 completes". That is the reading the vocabulary
implies and the one a sequential plan wants, but it is stronger than the words
strictly required, and a plan that wanted the weaker reading has no way to ask
for it. We accept that: a milestone edge that constrains only the milestone
constrained nothing anybody could act on, which is why it had no observable
effect before this.

**ADR-0008's hostility to new surface is upheld, and that is the point.** The
alternative considered first was an `after:` field on the milestone. It reads
well and it is one more thing to model, one more schema field, one more thing an
importer must map and a consumer must understand. Reusing `blocked_by` costs a
sentence of documentation instead, and the sentence is one a reader of ADR-0008
already half knows.

**A `--cut` filter on `ready` was rejected.** It was the cheap option and it
fixes only the noise: `critical-path` would still report a fragment, and the
number that looks like an answer is the more dangerous of the two.

**Two concepts, decided once, deliberately kept separate.** A cut's *label* is
what it calls itself and orders the roadmap view. A cut's *sequence* is a graph
edge and orders the work. Deriving one from the other was considered and refused:
a project may number cuts `s0..s6` and still run two of them in parallel, and a
project may sequence cuts whose labels sort in no useful order at all.

## Alternatives

| Option | Why not |
|---|---|
| `after:` on a milestone | New model surface for a relation `blocked_by` already expresses; ADR-0008 |
| `--cut` filter on `ready` | Leaves `critical-path` wrong, which is the error that hides |
| Derive sequence from label order | Assumes every numbered cut is sequential; wrong for parallel cuts |
| Do nothing | Two commands stay wrong on any sequential plan, and 81 hand edges is the workaround |
