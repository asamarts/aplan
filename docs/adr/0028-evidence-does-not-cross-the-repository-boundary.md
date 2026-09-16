# ADR-0028: Evidence does not cross the repository boundary

Date: 2026-09-02
Status: Accepted
Tasks: `A1-249`
Completes ADR-0021 (a trace that leaves the repository is declared), which stopped the one
mechanism that crossed silently and left this one listed as a deferral. Supersedes nothing.

## Context

ADR-0021 gave `traces` a way out of the repository: a target under a root declared in
`traces_external` resolves, and one that leaves undeclared is `APLAN034`. Its own consequences
named what it was not settling: "`blocked_by`, `commit` and `test` evidence, cross-repo cuts and
portfolio rollup remain the documented deferral".

The ninth pilot met the gap in the case this project invented its pilot directory for: work whose
deliverable is a friction log in *this* repository (pre-public adoption evidence item 6).

```console
$ aplan close AM-001 --evidence file:../aplan/pre-public-adoption-evidence
error[APLAN015] AM-001: file `../aplan/pre-public-adoption-evidence` does not exist at HEAD
```

The asymmetry is real: a task may answer to a sibling's document and may not prove itself with the
sibling's file. The question is whether `evidence_external` is the missing half.

## Decision

**It is not. `file` evidence resolves against this repository's history and does not gain a
declared root. A deliverable that landed elsewhere is cited with `url`, or tracked in that
repository's own plan.**

**1. A `file` citation has one slot for a rev, and it is a rev of this repository.** Resolution is
`path_exists_at(rev, path)`: git is asked whether the path is a blob in a commit's tree, here.
Crossing the boundary needs a second repository *and* a second rev, and `path#section@rev` has
nowhere to put the second one. Resolving a sibling at whatever its working copy happens to be
would make the citation mean something different on every machine, which is the failure ADR-0021
exists to prevent, reintroduced one rung up.

**2. The relevance rung cannot follow.** `file` is not "the file exists"; `APLAN031` also asks
whether the cited file lies inside the task's `paths`, which are globs over this repository. A
sibling file is outside every one of them, so a declared root would have to skip the check --
producing a rung called `file` that proves strictly less than `file` does, under the same name and
the same `require_min_strength`. A weaker check wearing a stronger name is worse than the gap.

**3. The comparison with `traces_external` does not carry.** That declaration widens a
*filesystem read* of a document. This one would open another repository's object database and
trust its history. ADR-0021's security argument was that an unconstrained target is a path this
tool will read, and a task file can arrive in a fork's pull request; the answer was a declaration
in a file the repository controls. Here there is nothing to defend, because nothing is read: an
absolute or climbing path is not a path in any tree, so today's behaviour is already safe. The
declaration would create the exposure rather than contain it.

**4. `url` is the mechanism, and what makes it shallower is not what it looks like.**
`require_min_strength` is not a ranking, whatever its name suggests: it is a set of accepted kinds,
and a `done` task must carry one of them with a real reference. So a repository listing
`[commit, file, test]` has not rated `url` lower, it has left it out.

What is genuinely shallower is the ladder. A `file` citation is checked twice -- the blob is in a
tree here, and the file lies inside the task's `paths` -- while a `url` is checked once, that the
adopter's configured checker got an answer, because there is nothing about a URL to judge against
`paths`. That difference is real and it is the honest reason to prefer `file` *for work that landed
here*. It is not a reason to want a `file` citation that crossed the boundary, because such a
citation would have lost the second check too (point 2), leaving both kinds checked once and only
one of them naming an immutable object: a forge blob at a commit
(`https://forge/o/r/blob/<sha>/path`) is more than a working copy at `../sibling` is.

**5. The other half of the case is a modelling answer, not a tooling one.** `p00002`'s log is a
deliverable of *this* repository, and this repository has a task for it (`A1-247`). Two plans, two
tasks, each citing its own history, is the shape that already works and needs nothing.

## What changed

The refusal was right and its sentence was not. A sibling path was reported as `does not exist at
HEAD`, which the author can see is false -- the file is on their disk -- and the remedy offered was
to correct a path that is already correct. `APLAN015` now distinguishes the two: a path that leaves
the repository is reported as outside it, with the remedy that applies. Same rule code, because the
fault is still that the citation does not resolve; a different sentence, because the reason and the
remedy are different.

## Consequences

- No new config key, no new evidence kind, and nothing that opens a second repository.
- A repository whose deliverables land upstream cites `url`, and if its policy admits no `url` it
  is choosing that, having been told why at the moment it hits it.
- The multi-repo edge stays where ADR-0021 left it. `blocked_by` and `commit` across repositories,
  cross-repo cuts and portfolio rollup are still deferred, and this decision narrows that list by
  one rather than reopening it.
- **What would overturn this.** A repository where the deliverable's forge is unreachable from CI
  and the sibling is checked out at a pinned commit, so a URL cannot be checked and a path can.
  That is a real shape and no pilot has been it; the evidence to revisit this is a log showing it,
  not an argument that it could exist.
