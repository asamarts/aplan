# ADR-0034: A task declares the change; the scheme decides the number

Date: 2026-09-14
Status: Accepted
Tasks: `A1-311`
Amended 2026-09-15 by `A1-311`'s own audit: two decisions the ADR had made but not
recorded, and one it had not noticed it was assuming. See *What the audit found*.
No decision changed; two were written down and one was handed to a task.
Extends ADR-0022 (a release closes cuts; a cut holds work) to the version those
deliveries carry. Supersedes nothing.

## Context

A plan that knows a task is `done` does not know whether shipping it obliges a
new version, and nothing connects the two. The release number is chosen by hand
at tag time, from memory, by whoever is tagging -- which is the same take-it-on-
trust this tool refuses everywhere else.

**The drift is measurable in this repository's own plan.** Eight cuts are `done`
and named `M-v0.1` through `M-v0.8`. Two tags exist, `v0.1.0` and `v0.2.0`. The
workspace version is `0.3.0-dev`. The plan's milestone names promised nine
versions; the artifact delivered two. No task carries the `release` tag at all,
so the plan has never modelled a release -- only the cuts it hoped would become
one.

**The prior art divides and neither half does this.** Agile tooling has the
hierarchy and no version semantics: Jira, Linear, SAFe release trains. Release
tooling has version semantics and no hierarchy: `changesets` asks for a
`major | minor | patch` file per change, `semantic-release` infers the same from
commit messages, `release-please` edits version strings wherever they live.
`changesets` is the closest, and its insight is that the argument about whether a
change deserves a major bump belongs at review time rather than at release time.
**This tool already has the file `changesets` had to invent: it is the task.**

The verifying half exists too, for at least one ecosystem: `cargo-semver-checks`
compares a crate's public API against its last published release and fails when a
breaking change ships without the bump semver requires, with a required bump per
lint.

### What the plan already models

ADR-0022 gives three levels and this decision needs no fourth: a **work task**, a
**cut** that holds work, and a **release** that closes cuts. Milestones nest
through `blocked_by` -- `M-v0.9` waits on `M-v0.8` -- so a plan can be deeper
than three, and a rule that keys on *kind* rather than on depth keeps working
when it is.

### What the tasks actually are

Measured on 2026-09-14, across the 317 work tasks the plan held that day, using
`CHANGELOG.md` sections and `docs/upgrading.md` rows as the record of what each
change asked of a consumer. The plan grows, so the counts date rather than
describe; the shape is what the decision rests on:

| | tasks | share |
|---|---|---|
| feature (*Added*, *Deprecated*) | 160 | 50% |
| named in nothing consumer-facing | 114 | 36% |
| breaking (*Changed*, *Removed*) | 27 | 9% |
| fix (*Fixed* only) | 22 | 7% |

The first reading of that table was that a `patch` default is "wrong for 59% of
tasks". **That reading is a category error and is recorded here because it nearly
decided the design.** The table answers a semantic question -- what kind of change
was this -- while a level default answers a cadence question: how far does the
version move when this completes. Under a hierarchy the two are not the same,
because a cut's floor absorbs the features beneath it. The share that genuinely
*must* be declared, because no floor can supply it and getting it wrong ships a
break in a compatible release, is the breaking 9%.

## Decision

**A task declares the change it made. The scheme decides the number.**

### One ladder, describing the change

`none < fix < feature < breaking`, and not `patch < minor < major`.

The deciding argument is in this repository today. It is `0.3.0-dev`, and cargo
formalises `0.x.y` as `0.INCOMPATIBLE.COMPATIBLE`: **a breaking change here bumps
the minor.** Were the vocabulary semver's arithmetic, an author would have to
write `minor` for a breaking change and know the 0.x rule to do it -- so the word
in the file would not mean what it says, and the file is the thing a reviewer
reads. `breaking` stays true at every version, and the arithmetic is the tool's
job:

| ladder | semver >= 1.0 | semver 0.x | CalVer |
|---|---|---|---|
| `breaking` | major | minor | date, and the incompatibility is declared |
| `feature` | minor | patch | date |
| `fix` | patch | patch | date |
| `none` | nothing | nothing | nothing |

CalVer keeps only the compatibility claim, because its number comes from the
calendar and no ladder can move it. That is the honest answer for a scheme with
no ladder, and it is why one implementation serves all three.

**The semver words are not accepted as aliases.** Teaching that `major` means
`breaking` is teaching something false below 1.0, which is where this project and
most new adopters are.

### A level supplies a floor, a task supplies a claim

The default map, which an adopter never has to write:

| level | floor |
|---|---|
| work task | `fix` |
| cut | `feature` |
| release | none of its own; it takes the strongest of the cuts it closes |

A **floor**, not an assertion: only a floor composes. A `breaking` task inside a
cut raises the cut, and the cut raises the release that closes it. A ceiling
would let a break ship inside a compatible release, which is the failure this
exists to prevent.

**A release carries no floor of its own, and that is deliberate.** "Every release
is a major" is version by ceremony rather than by compatibility, and it is wrong
for any project past 1.0. A team that versions by ceremony sets the release floor
to `breaking` and gets it; the shipped default reserves `major` for
incompatibility, which is what the number is for. A team that does not want a cut
to imply a minor sets that floor to `none` -- **and this repository will, because
eight of its done cuts produced two releases.** The default map being wrong for
the plan that ships it is exactly why the map is configuration.

### Off unless asked for

The requirement is off by default, as `require_effort` is. A plan that never asks
sees nothing: no field, no rule, no finding. With it on and nothing else
configured, every task takes its level's floor and a plan that declares no
impacts at all still judges clean. The only thing that must be written by hand is
the claim no floor can supply -- `breaking`, and `none` with the reason it is
none.

### Which rung each half sits on

- **`check`, hermetic**: the field is one of the ladder's words; a `none` carries
  a reason; a task claims no *less* than its level's floor. No repository, no
  command, no network.
- **`verify`**: the artifact's version, read by a **declared command** rather than
  parsed -- this tool does not know what a `pyproject.toml` means and should not
  pretend to -- compared against what the closed tasks require.
- **`verify --execute`**: a declared checker run against the built artifact, so a
  claim of `fix` on a task that removed a public function is contradicted by
  something other than the author. `cargo-semver-checks` is the reference. It runs
  under the caps ADR-0033 added, because an API differ compiles a workspace twice.

### One artifact first

A task does not name which artifact it moves. This repository is one workspace
with one version, and so is the common adopter. The extension point is named
rather than built: a task would carry an artifact the way a runner carries
`covers`, and every rule, the aggregation and the release gate each grow a
dimension. That is a second decision, taken when a monorepo actually asks.

## What the audit found

**It never said this tool does not write the version.** That was decided and is
load-bearing: reading a version goes through a declared command because this tool
does not know what a `pyproject.toml` means, and writing one is the same problem
and harder -- `release-please` needs a format-aware updater per file type and
inline annotations for arbitrary text to do it. So the rule is explicit now: **it
computes the number and hands it over; something else writes it.** The ecosystem
is arranged for exactly that. `cargo-dist`, which HLD section 17 commits this
project to, does not own the version at all and reacts only to a tag;
`cargo-release` takes the version as an argument; `release-please` takes a
`Release-As: X.Y.Z` commit footer. What none of them has is a reason to pick the
number, which is the part this tool supplies. `A1-319` proves the hand-off against
a real tool rather than leaving it reasoned about.

**It assumed a window it does not have.** "Derived from the tasks a release
closes" reads as though the set is obvious. It is not: this plan closes **no**
releases -- the `release` tag ADR-0022 defines is carried by no task, against
eight done cuts and two tags. So the arithmetic has no window in the repository
that ships it. That is not a flaw in the ladder above and does not change it, but
it is a hole this ADR did not admit, and `A1-317` now owns it: what a release node
carries, what closes it, and what the window is when a plan has modelled none.

**And the counts were written in the present tense.** They were measured on one
day against 317 work tasks; the plan grows. They are dated now. Recounted at the
audit the shape was unchanged -- 160 features, 27 breaking, 22 fixes -- which is
the part the decision rests on.

The open parts are staged in the `M-versioning` cut rather than left in prose
here: `A1-316` decides whether closing a task may cause anything at all, `A1-317`
the release model, `A1-318` whether version-string consistency belongs here or in
`alint`, `A1-319` the hand-off, and `A1-320` turns those into a design and files
the work.

## Consequences

**The migration is mechanical here and may not be elsewhere.** 27 tasks need
`breaking`, and both `CHANGELOG.md` and `docs/upgrading.md` already record which
-- so this plan's backfill is derivable rather than remembered. A plan without
that record has to judge 300 tasks by hand, and `A1-315` must report which of the
two it turned out to be rather than assume the easy one.

**The cut floor will be turned off in this repository.** Eight done cuts against
two tags says a cut here is a planning bucket, not a delivery. The feature's own
default therefore does not fit the project that ships it, which is worth stating
plainly rather than discovering: it is evidence for configurability, not against
the default.

**A `fix` default can still under-claim.** A task that broke something and said
nothing gets `fix`. Three things stand between that and a shipped break: the
author, the reviewer reading a one-word claim in the diff, and the L3 checker
where one exists. Below 1.0 the blast radius is smaller -- a break lands in a
minor, which cargo already treats as incompatible. The cost is stated rather than
engineered away, because the alternative considered and rejected was requiring a
declaration on every task, which is unadoptable at that scale and produces the lazy
declarations `semantic-release` is criticised for.

**This depends on two open questions.** The level of a task is read from the
`milestone` and `release` tags, which `A1-212` reports are an undeclared
vocabulary that `APLAN026` never checks -- so a task tagged `releaes` silently
becomes a work task, and under this decision that silently demotes a major to a
patch. And the hierarchy is walked over `blocked_by`, which `A1-306` reports is
one edge doing two jobs. Neither blocks the declaration rung; both must be
settled before the aggregation in `A1-313` is trusted, and `A1-313` should say so
in its own body.
