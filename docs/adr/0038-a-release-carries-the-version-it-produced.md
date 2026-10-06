# ADR-0038: A release carries the version it produced, and the window is the tag when it does not

Date: 2026-09-20

Status: accepted. Decision 3 was **completed** on the same day, not changed: it said
"the last release tag" without saying which tags count as releases, how the nearest one
is found, or what the window is when there is none. This repository's own tag set makes
all three answerable rather than theoretical, so they are written into decision 3 below.
The decision -- that the window is the tag when no release node exists -- is the one
that was made and is unchanged.

Supersedes nothing. Extends ADR-0022, which introduced the release node.

## Context

ADR-0022 models a release: a milestone carrying the `release` tag, whose members are
cuts. `A1-313` wants to derive the next version from "the tasks a release closes".

**The concept is inert in the repository that ships it.** Eight cuts are done,
`M-v0.1` through `M-v0.8`; three tags exist, `v0.1.0`, `v0.2.0`, `v0.3.0`; and no task
in this plan carries the `release` tag. So the derivation has nothing to read here, and
the same is true of every adopter on their first day.

Four questions, each answered against measurement rather than intent.

## What this repository's own releases actually were

Read out of git, by asking what each tag's tree said:

| release | cuts done at the tag | cuts newly closed | tasks newly `done` |
|---|---|---|---|
| `v0.1.0` (2026-07-31) | `M-v0.1`..`M-v0.5` | -- | 28 total |
| `v0.2.0` (2026-07-31) | `M-v0.1`..`M-v0.5` | **none** | **4** |
| `v0.3.0` (2026-09-16) | `M-v0.1`..`M-v0.8` | 3 | 240 |

**`v0.2.0` closed no cut at all.** It shipped four tasks -- `A1-003`, `A1-023`,
`A1-024`, `A1-028` -- out of a cut that was still open, on the same day as `v0.1.0`.

That single row decides two of the four questions below, and it is the reason this ADR
is written from history rather than from the model.

## Decision

**1. A release node carries the version it produced, recorded when it closes.**

Derived while open, so the plan decides the number rather than grading a human's guess
(`A1-313`); written into the node at close, as a `version` field, so a later run can
ask what `v0.2.0` contained without re-deriving from a plan that has since moved. The
field is named here rather than left to the implementation, because an ADR that says
"a field" and nothing more is how two of them get written. A closed release whose
answer changes when someone edits a task it closed is not a record.

**The existing tag vocabulary cannot carry it**, which is measured, not assumed.
`parse_cut_version` reads `vN.N`, and a release needs `vN.N.P`; a node tagged `v1.0.0`
falls through to "first tag that is not `milestone`" and renders as **`release`**. So a
release tagged with its version displays as the word "release". ADR-0008 resists new
fields and this is the argument against it here: the alternative is a tag that does not
survive being read back.

**2. What closes a release is a `commit` citation, and no new evidence kind.** A tag
points at a commit, the ladder already resolves one, and a release closing on its own
release commit is proved exactly as every other task is. `A1-060`'s rule that an
artifact must name itself is satisfied by the commit, not by a new kind.

**3. With no release node, the window is everything closed since the last release tag** --
where "the last release tag" means the nearest tag **reachable from the commit being
asked about** whose name matches the release-tag pattern, and the pattern is
configurable with a default of `v*`.

Both halves of that are load-bearing, and this repository is the demonstration.

*Reachability, not recency.* These tags exist here today:

```
v0.3.0                          2026-09-16
archive/k0-037-p003-pilot-log   2026-09-16
archive/k0-010-p003-pilot-log   2026-09-05
v0.2.0                          2026-07-31
v0.1.0                          2026-07-31
```

The two `archive/` tags are **not ancestors of `main`**, so a history walk skips
them; a date-ordered "latest tag" ranks one of them second, on the same day as
`v0.3.0`, and would have ranked it first had it been cut an hour later. The window
must be found by walking history, the way `git describe` does, and never by sorting
tags on their dates.

*A pattern, not every tag.* Even walking history, a repository that tags things other
than releases needs to say which tags are releases. `v*` is the default because it is
what this project and its pilots use; a project tagging `release-2026-09` says so in
`.aplan.yml` rather than being told its vocabulary is wrong (`A1-037`'s lesson about
`vN.N`).

*The first release has no prior tag*, and the window is then everything the plan has
ever closed. That is the honest answer -- a first release does contain all of it --
and it is stated here because "since the last tag" silently means "since the
beginning of time" exactly once in a repository's life, which is the run nobody
tests.


Not "require a release node": that makes the feature unusable until an adopter
restructures their plan, and this repository would have to file retroactive release
nodes before it could use its own feature.

Not "the open cut": a cut is not a delivery. This repository shipped `v0.3.0` out of the
middle of the `v0.9` cut, and `v0.2.0` out of the middle of an open one. That option
would give a confidently wrong answer on two of three real releases, which is worse than
refusing.

The window needs git, so it resolves in `verify` and not in `check` -- the same boundary
every other rule that reads outside the plan sits on. In a shallow clone it cannot run,
and it must say so once rather than pass silently, exactly as `APLAN045` does
(`A1-301`).

**4. This plan's own releases are deliberately not modelled**, and ADR-0022's release
node is why. Its members are *cuts*, and two of this repository's three releases closed
no cut. Modelling `v0.2.0` as a release node would mean either a node that closes
nothing, or one that re-closes the same five cuts `v0.1.0` already closed and
double-counts them. Both are records of something that did not happen.

So `A1-313` reads the tag window here, which decision 3 makes the supported answer
rather than a fallback. A plan whose releases *do* align with cuts models them and gets
the stronger answer; this one does not, and says so.

## Four levels, tried rather than assumed

ADR-0034's floors key on kind rather than depth, so a plan may nest further than
release / cut / task. Built and run: a release closing a cut, the cut holding a
sub-cut, the sub-cut holding the work.

**Two of the three findings below were a misreading, corrected 2026-09-23 by
ADR-0042.** ADR-0013 decided that a milestone listing another milestone in
`blocked_by` *sequences* the two cuts, and that the member filter excludes it
deliberately -- so the fixture was never nesting, and the rollup reading zero was
that decision working. What was real is that the same edge meant two things with
nothing saying which. ADR-0042 adds `contains`, which makes nesting sayable, and
the two findings then hold of the shape they describe. The third was a genuine
defect throughout.

It does not hold, in three separate ways:

- **`APLAN021` fires on the intermediate cut.** ADR-0022 exempts a *release* from the
  empty-cut test, not a cut whose members are cuts, so every intermediate level of a
  legitimate four-level plan is reported as having no work in it.
- **The rollup reads zero above the sub-cut.** The cut reports `0/0 done, 0.0d effort`
  while holding five days of work one level down, because a cut's members are its direct
  `blocked_by` minus milestones. The release then reads one level through to the cut's
  members, which are empty, and reports `0/0` as well.
- **A release tagged `v1.0.0` is labelled `release`**, per decision 1.

None of these is fixed here: this ADR decides what a release carries, and they are code.
They are filed as `A1-332`, `A1-333` and `A1-334` so the next plan deeper than three
levels meets a task rather than a surprise. Until then, three levels is the supported
depth, which is now a measured statement rather than an untested assumption.

## Consequences

`A1-313` has a defined window in both shapes, and a demonstrated one in this repository.
The cost is a field on a release node, argued above against ADR-0008, and a documented
depth limit this project did not know it had.
