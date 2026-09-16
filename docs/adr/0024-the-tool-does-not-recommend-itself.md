# ADR-0024: The tool does not recommend itself

Date: 2026-08-17
Status: Accepted
Tasks: `A1-149`
Extends ADR-0017 (ordering is derived, not declared) by drawing its other edge: what a plan's shape
does *not* contain. Supersedes nothing.

## Context

The third pilot read three shipped features in full and declined all three -- `tag_families`,
`roadmap.group_by` and cut sequencing -- and was right about each. Its tags are descriptive words
rather than a second grouping level, and its cuts are parallel slices rather than a pipeline. The
log's closing line is the finding: "an adopter reads a *what is new* list as a list of things to do,
so a guide that also says when a change is not for you would have saved reading three features to
decline them" (pre-public adoption evidence item 25).

Two answers were possible. A column in the upgrade guide, which is prose that nothing keeps true.
Or a **derived** answer: the plan already knows whether its milestones block one another, whether
its tags cluster, whether its paths fall into components, so a command could report which optional
features would do something for this plan. That is the same class of answer as `ready` -- derived,
not declared -- and it cannot go stale the way a table can.

The task set the bar: the four cases from that pilot are the test, and a candidate that does not get
all four right is not worth having.

## Decision

**No command recommends features. The guide answers instead, and a test keeps it honest.**

The derived version was built far enough to measure, over five real plans (this repository,
`p0000004`, `p003`, `p00025`, `p00020`), with these signals: do tags share a family prefix, do cuts
depend on each other's work, do declared `paths` fall into more than one component.

On the pilot's four cases it scores 4 of 4. On five plans and four features it scores 16 of 20 --
three clear misses and one arguable, and **the misses are the ones that matter**. The arguable one
is this repository: told that its cuts depend on each other's work (28 cross-cut edges against 25
within), the signal advises sequencing, which this plan has never declared and which its cuts,
being chronological, would arguably suit. Counting that one as correct gives 17 of 20 and changes
nothing, because a single confident wrong answer about the plan that *adopted* a feature is what
fails the bar:

| feature | signal | verdict |
|---|---|---|
| `tag_families`, `roadmap.group_by` | tags share a family prefix | 5 of 5, and circular: the prefix exists *because* the repository adopted the feature |
| cut sequencing | cuts depend on each other's work | tells `p00020` to skip it. `p00020` sequences all seven of its cuts, and has 6 cross-cut edges against 70 within-cut ones |
| `test` runners with `covers` | `paths` fall into components | tells `p00020` and `p0000004` to adopt it. Both have components and neither configures a runner |

The failure is not tuning. Whether cuts are *stages* or *slices* is intent, and a plan records the
same edges either way: `p00020`'s stages are sequenced because that is what its author meant, not
because its tasks reach across cuts. ADR-0017 refused a declared rank because ranking is derivable;
this is the mirror image, and the same rule applied honestly refuses to derive what is not there.

A recommender that is confidently wrong about the plan that *adopted* a feature is worse than no
recommender, because the reader cannot tell which of its answers is the wrong one. Note which way
the errors fall: every miss is a **"not for you"** told to a repository that wanted the feature, or
a **"take this"** told to one that did not, so the failure is not conservative in either direction.
And a tool that recommends its own features is a shape to be suspicious of on its own account.

**What ships instead**: a table in `docs/upgrading.md`, "Which of these are for you", written in the
plan's terms rather than in benefits, whose right-hand column is what lets a reader skip a row. The
gate is `every_optional_feature_says_who_it_is_not_for`: every top-level config key in `facts.json`
must have a row, and every row must answer both halves. A new opt-in cannot ship without saying who
it is not for.

That gate checks presence, not truth, and this decision does not pretend otherwise. It is the same
trade `check-docs` makes about a document naming a task: what can be mechanised is that the question
was answered, and the answer's quality stays a review's job.

## Consequences

The measurement stays reusable: the signals are three queries over a plan, and if a future feature
turns out to be predictable from shape alone -- one where the plan records the intent rather than
implying it -- this decision does not forbid reporting that. What it forbids is a general
recommender that speaks with one voice about features it can and cannot judge.

`aplan facts` remains the machine answer to "what does this build have", and it says nothing about
whether a feature suits a plan, which is now a documented boundary rather than an absence.
