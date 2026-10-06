# ADR-0037: A generated artifact names the build that wrote it

Date: 2026-09-19
Status: Accepted
Tasks: `A1-325`
Extends the banner convention `A1-059` established for generated artifacts.
Uses the identity `A1-256` made available. Supersedes nothing.

## Context

Several artifacts here are generated, committed, and compared against their
generated form by a gate:

| artifact | gate |
|---|---|
| `ROADMAP.status.md` | `cargo xtask gen-roadmap --check` |
| the `AGENTS.md` block | `aplan export-agents-md --check`, and `APLAN041` |
| `schemas/v1/*.json` | `cargo xtask gen-schema --check` |
| `docs/rule-catalogue.md` | the corpus test |
| `facts.json` | its own drift test |
| `coverage.json` | `cargo xtask coverage --check` |
| `cli-surface.json` | `cargo xtask cli-surface --check` |

The last two are **ratchets rather than byte comparisons**: they compare numbers
this build measured against numbers some build committed. That makes them the
same hazard in a different shape -- a floor or a count is only comparable to the
committed one if the same `aplan` produced both -- so they name the build in
their refusal like the rest, and they carry no banner because neither file has
anywhere to put a comment that is not part of the data.

**Each comparison regenerates with the build that is running and diffs against
the committed file.** Where both sides are the same build, which is always on
the machine the check is run on, the check is green whatever the build. It is
only *not* the same build somewhere else -- in CI, or in an adopter's repository
where a pinned release regenerates a file a newer binary wrote.

The third pilot hit exactly that. A local check passed; CI, running a pinned
`0.2.0`, failed on a thirty-line diff because the generated shape had changed
between `0.2.0` and `0.3.0-dev`. Nothing in the diff said so. What caught it was
a note the pilot had added to their own `AGENTS.md` after an earlier near-miss,
not the tool (pre-public adoption evidence item 13).

This is the shape `A1-209` and `A1-210` name on other surfaces: a check that
cannot fail where it is run. A stale fingerprint at least produces a diagnostic;
a generator compared against itself produces silence.

## Decision

**The artifact carries the build, where a banner is free.** `ROADMAP.status.md`
and the `AGENTS.md` block gain one line:

    <!-- written by aplan 0.4.0-dev, surface e122c5b85f96 -->

Version *and* surface digest, because they answer different questions. The
version is what a human recognises, and every build from `main` carries the same
one -- so on its own it cannot distinguish two development builds, which is the
case `A1-256` exists for.

**The surface digest covers what this build writes, not only what it
advertises.** The first cut took the first twelve hex digits of `facts_sha256`,
reasoning that the facts document moves when the rules, commands, config keys,
formats or evidence kinds move, which is when generated shape moves. That
reasoning is wrong, and the audit measured it: rewording one heading in the
roadmap renderer changed the artifact and left the banner byte-identical. A
renderer is not in the facts document. Since every development build here
reports `0.4.0-dev`, two builds differing only in rendering were exactly as
indistinguishable as the pilot's two builds were -- this decision reintroducing,
inside its own remedy, the defect it was written to remove.

So the digest is taken over the facts document **plus this build's rendering of
a fixed canonical roadmap and agent contract**. The question the banner is asked
is "would another build have written this file differently", and the only
honest way to answer it is to render something and look.

`facts_sha256` is left alone. It answers a different question -- what a consumer
may rely on, not what a generator will emit -- and `attest` and
`facts --identity` publish it under that meaning.

The fixture is deliberately not minimal. An empty roadmap renders a heading and
a table head and nothing else, so every change to a *row* would hash the same:
the green-by-construction shape one layer down from where it was found. It
carries a cut with the weeks column live, a member row with evidence and a
blocker, a group heading, a cross-cutting row and a critical-path total, and a
test names each of those so trimming the fixture fails rather than silently
narrowing the digest. It lives in code rather than in a file on disk, because a
digest computed from a file would move when somebody edited the file and claim a
build had changed when it had not.

**Not `facts_sha256` alone** for a second reason, found first. `A1-325` assumed
it avoided version churn. It does not: the facts document contains the version,
so the digest changes on every bump as well. Measured rather than assumed, and
the assumption was wrong in the task that proposed it.

**Inside the `AGENTS.md` block, never in the marker.** `A1-294` made that marker
a literal an adopter copies by hand, and `contract.begin_marker_matched_by_prefix`
pins how it is found. Changing it would make every adopter rewrite a line.

**Every drift gate names the running build when it reports a stale artifact**,
whether or not that artifact carries a banner. This is what covers the schemas,
and it is the half that helps in CI, where the reader has the failure text in
front of them and not the file.

## What deliberately carries no banner

**`schemas/v1/*.json`.** `ROADMAP.md` commits them "as stable" with additive-only
evolution, and the risk register calls format churn "the cardinal sin". A
provenance comment carrying the surface digest would change those files whenever
*any* surface changed -- a new rule code, a new command -- none of which affects
a schema's own content. Churning the artifact whose stability is the promise, for
reasons unrelated to it, is worse than the inconsistency. Their gate names the
build instead.

**`docs/rule-catalogue.md`.** It is generated only by this repository's own test
suite, and never by a released binary, so a different build cannot have written
it. The defect this ADR is about cannot reach it.

**`facts.json`.** It already contains the version, so it self-identifies. Adding
a second identity would give it two that could disagree.

## Consequences

A regeneration by a different build is now a one-line diff that explains itself,
and the thirty lines beneath it have a stated cause.

**`APLAN041` now reports a build difference as well as a policy difference.** The
generated block contains the build, so an adopter whose `AGENTS.md` was written
by one build and who runs another is told it is stale. That is the defect being
surfaced rather than a new one, but it is a widening of what the rule means and
the upgrade note says so.

**It says which of the two it found.** A block that matches the policy and names
another author is reported as a build difference, in those words, with a fix
that names the pinned build rather than saying "regenerate" -- regenerating is
what the reader would have done anyway and leaves the two builds disagreeing on
the next run. Reporting it as a policy change would send somebody looking for a
change that did not happen, which is the same wasted hour the unexplained
thirty-line diff cost, one layer up. Where the policy differs *as well*, the
policy is what is reported: that is the half the reader has to act on.

`advisory: [APLAN041]` is the way out for a repository that runs mixed builds
deliberately, and the rule's own instruction says so, because a reader meeting
this for the first time meets it as a failure.

The comparison ignores the provenance line rather than parsing it. This makes no
claim about *which* build is newer, and cannot: a version string is not ordered
here and a surface digest is not ordered at all. Naming both sides and letting
the reader decide is the honest answer, and it is the one the pilot needed.

The banner is in command output, not only in a written file, because the file is
that output. Two byte-gated CLI fixtures elide the value with `[..]` rather than
pinning it, so a new rule does not churn a test fixture -- the line's presence is
asserted, its contents are not.
