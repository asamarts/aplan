# ADR-0029: One repository holds one plan, and an edge never leaves it

Date: 2026-09-08
Status: Accepted
Tasks: `A1-200`
Extends ADR-0014 (one modelled grouping level) with the level above it, and ADR-0028 (evidence
does not cross the repository boundary) with the plan's own boundary. Supersedes nothing.

## Context

`root` is one directory and `--root` takes one value, so a repository holds exactly one plan today.
`A1-200` asked whether that should change, and it was filed as a goal rather than from a friction
log: no pilot had reported it. It made its own first condition explicit -- measure a real
repository before designing anything -- because the alternative was designing for an imagined
adopter.

A second use case appeared and was withdrawn. `A1-227` had measured `check` as quadratic in a cut's
member count, which made splitting a large plan a performance answer; `A1-227` then found the cost
was a traversal asking for a node's blockers once per edge, and `check` is now `O(n^0.97)` to 50,000
tasks. A plan does not get slow by getting big, so sharding buys nothing a user would feel.

## What the data actually shows

**`p00011` is the repository that most resembled two plans, and it is one plan**
(pre-public adoption evidence, 2026-09-08). It has never adopted this tool. Its `plans/` tree holds 148
documents carrying `type: stream` and an `id`: 23 unfinished under `plans/streams`, 117 finished
under `plans/archive/completed`, 8 obsolete or superseded, all 148 ids distinct.

Nothing in it varies the way a second plan would have to justify. Across the 23 active streams
`owner` has one distinct value, `public_readable` has one, and `type` has one. There is no team
boundary and no per-stream policy. The fields that do vary already have homes: `status` is a status,
`crates` and `migrations` are `paths`, and `related_streams` is an edge.

**Its costs come from splitting a plan, not from being unable to hold two.** Pointed at
`plans/streams` alone, `check` reports 10 `APLAN003`. None had rotted: every one names a *completed*
stream in the archive directory, unresolvable only because finishing a stream there means moving its
file out of view. Loading all 148 as one plan resolves 5 of the 10 immediately, because in this model
a finished task keeps its id and stays in the plan with `status: done`.

The other 5 are real: `T4` names `A4`, `A5`, `A6`, `A7` and `VOH` names `G3`, and no document in the
repository declares any of them, though prose mentions them 43, 39, 19, 28 and 7 times. One `check`
found all five in a corpus that had never seen this tool.

**And its cross-plan edge is not a dependency.** Read as `blocked_by`, `related_streams` produces
`APLAN004 dependency cycle: A1 -> A2 -> A1` on the first run, because the field is undirected. A
*directed* field is already available and unused: `p00011`'s own linter parses `blocked_by`, its
convention document defines it, and a rule resolves it against the registry -- and across 148 stream
documents it is used zero times, against 252 `related_streams` entries on 81 of them. The ordering
the repository actually has lives in prose.

**Its own answer to many streams is one registry, not many plans.** `p00011` built a linter for its
plan corpus before this project was pointed at it: `src/rust/tools/plan-lint`, 17 rules over
`plans/**`, gated in CI, resolving every cross-stream reference against a single repository-wide
`plans/status/streams.yaml`. That is this decision, reached independently and implemented. The
failure it does have is a *second* namespace growing beside the first: the registry lists 113 ids
and the tree holds 148 documents, disagreeing by 13 in one direction and 48 in the other.

**No adopter has asked.** Fifteen repositories on the development host hold a plan, 2,441 loadable
tasks between them on 2026-09-08. Every one holds exactly one plan.

## Decision

**A repository holds one plan, and a dependency edge never leaves it.**

`blocked_by` names a task in this plan and nothing else. There is no qualified id, no second root,
and no cross-plan reference. A repository that wants two groups of work uses the grouping the model
already has: a cut (ADR-0008), whose members are tasks in the same plan and whose relationship to
another cut is a milestone-to-milestone edge (ADR-0013).

`related_streams` does not become a field. An undirected "see also" changes no answer any command
gives, which is the bar ADR-0025 set for a field and ADR-0027 applied to a task's product.

## Consequences

**`APLAN003` keeps its meaning**, which is the whole reason for the decision. "`blocked_by`
references a task that exists" is answerable because "exists" means "is in this plan". A qualified
id would make the rule resolve against a corpus outside the root it was given, so either `check`
stops being hermetic in one directory or it grows a second root to be hermetic over -- and the
`p00011` measurement says the references that break are the ones that crossed such a boundary, not
the ones that stayed inside.

**`APLAN022` keeps a countable question.** "Every task is a member of exactly one cut" is a count
over one plan. Across two, "exactly one" has to say one *per plan* or one *in total*, and the two
readings disagree for every task in a cut that spans them.

**Every rollup keeps one answer.** `summary`, `roadmap`, `critical-path` and `ready` each answer
about a plan. With one plan per repository they need no merge rule, no precedence between plans, and
no way to express "ready, in the other plan". `A1-075` already decided the composition point for
rollups is `aplan tasks` plus a shell loop, and that answer covers running the tool twice over two
repositories as well.

**A single-plan repository is unchanged**, because there is no feature to be ambiguous about.
`.aplan.yml` discovery by walking up stays unambiguous for the same reason: there is one plan to
find.

**An adopter that has split a plan is told to unsplit it**, and gets a rule that catches what the
split was hiding. That is the `p00011` recommendation and the pilot log carries it.

## What would reopen this

A repository holding two plans that **differ in something a plan carries**: a different policy in
`.aplan.yml`, a different owner or team, or a different set of rules one half is held to. Every
candidate measured so far differs in none of these, and `p00011` -- the closest -- differs in none
of them across 148 documents.

The question this could not answer is what *two teams* in one repository want, because no repository
available has two. Of the 32 git repositories on the development host, exactly one has a `CODEOWNERS`
file and it names a single owner for everything rather than teams; the 15 distinct git author
identities across all of them resolve to one person, five bots and a test fixture. That is a gap in
the evidence, and it is named here rather than resolved by assumption.
