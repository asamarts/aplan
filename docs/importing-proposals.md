# Importing proposals (RFC pipelines)

`aplan import` reads someone else's plan into `aplan` tasks. It is **lossy by
design, and says so** (ADR-0012): every field the task model cannot hold is
reported, never dropped in silence, because a lossy import that quietly discards
a field is precisely the drift this tool exists to detect.

This guide covers the RFC/proposal flavor, `--format kep`.

## Why proposal repositories

Every mature RFC process separates *design accepted* from *implemented*:

| Process | Accepted | Implemented |
|---|---|---|
| Kubernetes KEP | `implementable` | `implemented` |
| Oxide RFD | `published` | `committed` |
| Rust RFC | merged RFC | tracking issue closed |
| Matrix MSC | MSC merged | spec PR landed |

That gap is the claimed-versus-resolved gap `aplan` models, which makes these
repositories a natural source. Serving them needs no new status: **a `todo` task
already *is* accepted-but-not-implemented.**

## The tag convention

A proposal's stage is a **tag**, never a status (ADR-0012 clause 2):

| Source | Becomes |
|---|---|
| `status: implementable` | `status: todo` + tag `kep:implementable` |
| `status: implemented` | `status: todo` + tag `kep:implemented` |
| `status: rejected`, `withdrawn`, `replaced` | `status: dropped` + tag `kep:<status>` |
| `stage: beta` | tag `kep:stage:beta` |
| `owning-sig: sig-node` | tag `sig:node` |

So "accepted but not implemented" reads as a `todo` task carrying its own
pipeline vocabulary, and you can group or filter on the tag without `aplan`
pretending to validate a vocabulary it does not own.

## A round trip

Given `kep.yaml`:

```yaml
title: Graduate the widget API to stable
kep-number: 3063
authors:
  - "@alice"
owning-sig: sig-node
status: implementable
creation-date: 2024-01-15
approvers:
  - "@dave"
stage: beta
```

Then:

```console
$ aplan import kep.yaml --root plan
imported 1 task(s) from kep.yaml
  KEP-3063  todo  [kep:implementable, kep:stage:beta, sig:node]  Graduate the widget API to stable
    -> plan/tasks/KEP-3063.md
report:
  dropped: `approvers` is not held by the task model (kep.yaml)
  dropped: `creation-date` is not held by the task model (kep.yaml)
  placement: imported tasks belong to no cut; where `require_cut_membership` is set, `check` reports APLAN022 until `--cut <milestone>` or a `cross-cutting` tag places them (kep.yaml)
  placement: imported tasks carry no `effort_days`, because there is nothing to derive one from; `check` warns (APLAN009) until you estimate them (kep.yaml)
```

produces `plan/tasks/KEP-3063.md`:

```markdown
---
id: KEP-3063
title: Graduate the widget API to stable
phase: 1
status: todo
owner: alice
tags:
- kep:implementable
- kep:stage:beta
- sig:node
---

## Why

Imported from `kep.yaml` by `aplan import --format kep`.
...
```

which validates:

```console
$ aplan check --root plan
warning[APLAN009] KEP-3063: no `effort_days` estimate
  --> plan/tasks/KEP-3063.md
ok: 1 tasks, 1 warning(s), DAG acyclic
```

The `APLAN009` warning is the honest outcome: a proposal repository carries no
estimate, and `aplan` will not invent one. It is a warning, not an error, so the
gate still passes; add `effort_days` when you plan the work.

## An imported claim is not proof

The rule that matters most: **an importer never asserts a `done` it cannot
prove.** A KEP that says `implemented` still arrives as a `todo` tagged
`kep:implemented`, and the report names the fix:

```
provenance: KEP-1287 claims `implemented`; imported as `todo` (the claim is not
proof). Close it with `aplan close KEP-1287 --evidence commit:COMMIT_SHA`
```

That default is also what keeps import output valid everywhere: because nothing
becomes `done`, the result satisfies any configured evidence policy, including a
strict `require_min_strength` that excludes `note`.

If you accept a weaker record, opt in explicitly:

```console
$ aplan import shipped.yaml --implemented-as done --as-of 2026-07-31
```

This yields `done` backed only by a dated `note`, the weakest evidence kind, and
warns that the provenance is note-level. The date is required rather than
derived: `APLAN010` needs a real ISO-8601 `captured` date, and `aplan` does not
invent provenance (`aplan close` refuses to author note evidence for the same
reason). Under a strict policy this output *fails* `APLAN017`, deliberately: an
imported claim should not survive a strict gate until someone proves it.

## Placing imports in a release cut

Freshly imported tasks belong to no cut, so where `.aplan.yml` sets
`require_cut_membership`, `check` reports `APLAN022` until they are placed.
`--cut` does that in the same run:

```console
$ aplan import kep.yaml --cut M-v0.6
```

It appends the imported ids to that milestone's `blocked_by` (ADR-0008's
membership model), editing only that line and leaving the rest of the file
byte-for-byte alone. It refuses a cut that is already `done` or `dropped`,
because adding an unfinished task to a settled cut would break it (`APLAN007`),
and it validates the cut *before* writing anything, so a refusal leaves the plan
untouched.

## Look before you write

`--dry-run` prints the whole result, including the report and any placement, and
writes nothing:

```console
$ aplan import kep.yaml --dry-run --cut M-v0.6
```

Import never overwrites: a task id that already exists is a refusal naming the
file that holds it, not a silent replacement.

## Flags

| Flag | Meaning |
|---|---|
| `--format <shape>` | source shape: `kep`, `checklist`, or `roadmap-json` |
| `--dry-run` | print what would happen, write nothing |
| `--out <dir>` | where to write (default `<root>/tasks` when it exists) |
| `--phase <n>` | `phase` for imported tasks (default `1`) |
| `--implemented-as todo\|done` | how to read a source's "implemented" (default `todo`) |
| `--as-of <YYYY-MM-DD>` | the note's `captured` date; required with `done` |
| `--cut <milestone>` | place the imported ids in a release cut |
| `--id-prefix <prefix>` | id family for a source that has none (`checklist` only; default `T`) |

## A markdown checklist

`--format checklist` reads `- [ ]` / `- [x]` items under `##` headings, which is the
commonest planning shape there is and what the first pilot kept before it adopted `aplan`.
A wrapped item is joined into one title, the heading becomes a tag, and the prose under
that heading is carried into each task's body, because a list of one-liners loses its
context otherwise.

It carries less than a KEP does, so it says more:

- **A ticked box is a claim.** It imports as `todo` tagged `checklist:checked`, and the
  report names what would close it. There is nothing in a checkbox to resolve, so `done`
  would be `aplan` asserting what it cannot check. `--implemented-as done` opts into the
  weaker record, with `--as-of`, exactly as for a KEP.
- **A checklist has no ids**, and a task nobody can refer to is not a task, so the
  importer generates them (`T-001` onward, or `--id-prefix`) and *says* it did. Inventing
  is the mirror image of dropping a field, and clause 3's reasoning applies to both.
- **Nesting and order are real information the model does not hold.** A nested item
  becomes a task of its own, and the flattening is reported; order becomes nothing at
  all, because writing `blocked_by` from position would be a guess recorded as a fact.

```console
$ aplan import ROADMAP.md --format checklist --id-prefix AL --dry-run
would import 18 task(s) from ROADMAP.md
  AL-001  todo  [phase-0-de-risk-the-hard-20]  `p0000004-core`: tokio runtime + a minimal INDI client ...
```

That is the first pilot's real roadmap, the one it hand-authored thirty-three tasks from;
the import produces eighteen tasks that pass `check`.

## A spec-driven task list

`--format spec-kit` reads `specs/NNN-feature/tasks.md`, and `--format openspec` reads
`openspec/changes/<name>/tasks.md`. RESEARCH section 15 ranked this camp first: spec-kit
discussion #2726 asks for "a deterministic gate... no LLM, `exit 0/1`", which is what this
tool is.

Both documents are markdown checklists whose items carry **ids**, which is the difference
that matters:

```text
spec-kit   - [ ] T012 [P] [US1] Create Entity1 model in src/models/entity1.py
OpenSpec   - [x] 1.1 Create `src/core/diff.ts` with diff logic
```

So unlike `--format checklist`, nothing is invented: `T012` and `1.1` are kept as the task
ids, and the source document's own prose still resolves against the plan. Pass `--id-prefix`
when you import a second feature: a spec-kit repository writes `T001` in every feature's
`tasks.md`, and without a prefix the second import meets the first's ids and is refused.

**One parser, two names.** The two formats differ by two tokens -- the id shape, and
spec-kit's `[P]`/`[US1]` markers -- so a second implementation would be duplication with a
second set of bugs. The flavors are separate names because that is what an adopter types,
and each declines the other's document rather than reading it wrong.

What is reported rather than guessed:

- **The ordering.** spec-kit writes a `## Dependencies & Execution Order` section in
  English: "Models before services", "User Story 2 may integrate with US1". Twenty-one
  lines of it in the upstream template. Turning that into `blocked_by` would be a guess
  dressed as a dependency, so the import names the section, counts its lines, and points at
  `aplan dep add <id> --after <id>`.
- **`[P]`,** which claims an item shares no files with its neighbours. That is an assertion
  about the code, not the plan, and nothing here can check it: it becomes the tag
  `spec-kit:parallel`.
- **User stories** become tags too, with a notice: `aplan` has one modelled grouping level
  and it is the cut (ADR-0014), so if the stories are your cuts, `aplan mv --cut` is how
  they become one.
- **File paths inside descriptions.** spec-kit's format section says "include exact file
  paths in descriptions", which reads like `paths` waiting to be extracted. Measured before
  declining: zero of the 34 items in the upstream template carry a resolvable path -- they
  are placeholders like `src/models/[entity1].py` -- and the one real item in that
  repository writes "in hello.py" inside an English sentence. Guessing there would produce
  the field that makes `commit` evidence checkable, wrongly.
- **Prose between items**, counted and reported. What sits under a heading before its first
  item is kept in each task's body, as for a checklist.

```console
$ aplan import specs/003-search/tasks.md --format spec-kit --id-prefix S3
imported 28 task(s) from specs/003-search/tasks.md
  S3-T001  todo  [phase-1-setup-shared-infrastructure]  Create project structure per implementation plan
```

Measured against the real documents on 2026-08-17: spec-kit's `templates/tasks-template.md`
and its `tests/hooks/tasks.md`, and two of OpenSpec's own change directories. 58 tasks, all
of which load and pass `check`, with 58 warnings, one per task, for the `effort_days` no
document of this kind carries.

## A table is not a checklist

There is no table importer, and that is a decision rather than a gap (ADR-0019).

A checkbox has one meaning. `- [ ]` is a proposal, everywhere it appears, which is why
`--format checklist` can read a document it has never seen and be right about it. A table
row has no such meaning. The fifth pilot's programme plan is 1,887 lines with **45 tables
and 451 data rows**, describing streams, gates, risks, deliverables, metrics, budgets,
role assignments and comparisons. Perhaps two of those tables are work. Only the author
knows which.

So `aplan` says what it read and what it wanted:

```console
$ aplan import PLAN.md --format checklist --dry-run
aplan: PLAN.md: no importable proposal found: this reads `- [ ]` and `- [x]` rows, and the
document has none. It has 45 table(s) with 451 data row(s) and 81 heading(s). A table row
has no single meaning the way a checkbox does, so nothing here guesses which of them are
work: extract the rows you mean into a checklist and import that
$ echo $?
2
```

### The recipe

Extract the rows you mean, in your own repository, and import the result. The extraction
is four lines of `awk` per table, and it is *yours*: it encodes which table is work and
which column is the title, which is exactly the knowledge `aplan` does not have.

Three passes: take the section, take the first table in it, take the columns you want.

```console
$ awk '/^## 6\. Stream registry/,/^## 7\./' PLAN.md \
    | awk '/^\|/{t=1} t&&!/^\|/{exit} t' \
    | awk -F'|' 'NR>2 { gsub(/^ +| +$/,"",$2); gsub(/^ +| +$/,"",$5); print "- [ ] " $2 ": " $5 }' \
    > streams.md
$ aplan import streams.md --format checklist --id-prefix STR --dry-run
would import 62 task(s) from streams.md
  STR-001  todo  []  `V0`: Fixed scope, evidence register, weekly kill-criterion review
  STR-002  todo  []  `V1`: npm/PyPA/native differential domain prototype
```

That is the fifth pilot's real stream registry, and those are its real numbers. The middle
pass is what makes it a table rather than a section: section 6 of that document contains
several tables, and a range over the whole section would have swept up eighty rows from
three of them. This repository runs that exact pipeline in its own test suite, so the
commands above are ones that have been executed rather than ones that look plausible.

Three properties make this the supported path rather than a workaround:

- **The mapping is written down and reviewable**, in your repository, next to the document
  it reads. A mapping expressed as `aplan` configuration would be a second place where a
  plan's meaning is declared, and this tool exists to have one.
- **The import is still gated.** Whatever the extraction produces goes through the same
  checklist importer, so a ticked row is still a claim, generated ids are still reported,
  and `check` still judges the result.
- **It fails loudly.** An `awk` that matches nothing produces an empty checklist, and the
  importer refuses an empty file by name.

### What that costs, stated

You write the extraction, and it is one more thing to maintain if the source document is
restructured. That cost is real and it is smaller than the alternative: a mapping language
inside `.aplan.yml` that has to express "the third column of the table under this heading,
unless the row is a total", which is a worse `jq` embedded in a plan validator.

## A hand-maintained status contract

`--format roadmap-json` reads a JSON status-and-dependency projection: `phases`, and
`workstreams` whose `milestones` carry a `depends_on` graph.

- **`depends_on` becomes `blocked_by`**, which is the point: a reference to a milestone that
  does not exist is `APLAN003` and a cycle is `APLAN004`, and neither was checkable before,
  because a linter that proves identifiers resolve to files says nothing about the graph.
- **The status vocabulary is mapped in one table, printed in the report.** `not_started` ->
  `todo`, `in_progress` and `blocked` and `dropped` carried across, and `done` -> `todo` with
  the source's word kept as a tag, because a hand-maintained `done` is the claim this tool
  exists to doubt. A word the table does not know is kept as a tag and flagged, never guessed.
- **A workstream becomes a cut**: a `milestone`-tagged task whose `blocked_by` is its own
  milestones (ADR-0008), so `APLAN007` refuses to let it be `done` while a member is not. Its
  `artifact` becomes a `traces` entry, which `check` then resolves.
- **`SPEC-`/`ADR-` references in prose stay prose.** The file gives ids, not paths, and a
  `traces` entry has to resolve; inventing a path from a naming convention this importer
  cannot see would break the plan whenever the guess is wrong. Each reference is named in the
  report instead.
- **`adrs` and `specs` are not imported.** They are reference records with their own statuses,
  not work, and the report says so with their counts.

## Authoring a plan wholesale

`aplan add --from <file>` is the other bulk seam, and it is not an import: the file is a list of
tasks in this tool's own vocabulary rather than somebody else's shape. An entry carries what
`aplan add` takes as flags -- `title`, and any of `id`, `phase`, `cut`, `effort`, `owner`, `tags`,
`paths`, `traces`, `blocked_by` -- plus `body`, the Markdown written after the frontmatter.

**It is applied as one change**, and the rules run once on the plan the whole file would produce,
so nothing is written unless all of it can be.

**Order is for reading, not for resolving.** An entry may wait on, or join, any id the file
declares, whether that entry is above it or below it. That matters because membership has two
spellings and they used to pull in opposite directions: `blocked_by` on the milestone wanted its
members written first, `cut:` on each task wanted the milestone written first, and a file cannot
do both. Either now reads the way a roadmap does:

```yaml
- id: M-v0.1
  title: First cut
  tags: [milestone]
  blocked_by: [T-001, T-002]      # or drop this and put `cut: M-v0.1` on each task
- id: T-001
  title: Decide what done looks like
  effort: 1
- id: T-002
  title: Wire the gate into CI
  effort: 2
```

An id the file does not declare is minted from the convention the plan already uses, and one
minted for a later entry has no name to be referred to by. A cut the file declares is named by its
`id`: the label a cut reports itself under is derived from the milestone the plan holds, and one
the file has not written yet has nothing to derive it from.

**The plan is asked first.** A `cut:` the plan already resolves is the plan's cut, so a file cannot
shadow one -- and where the plan refuses the name rather than failing to find it, because two of
its cuts answer to that label, the refusal stands. Reading past it to a milestone the file happens
to declare would answer an ambiguity with a third cut.

## Writing it back out

`aplan export --format checklist` is the other half. It exists because of what
adoption actually costs: the day the plan becomes the source of truth, the
document you imported from starts going stale, and it is usually the document
your colleagues still read. ADR-0009 already settled that the answer to "a
hand-maintained file disagrees with the plan" is to *generate* it.

**Lossy in this direction too, and it says so.** ADR-0012 clause 3 made an import
name every field the task model could not hold; an export names every field the
*target shape* cannot. The report goes to standard error, so a redirected
document is only the document:

On a four-task plan -- a cut holding two, one of them closed, and one task in no
cut:

```console
$ aplan export --format checklist --out ROADMAP.md
report:
  dropped: `id` has no place in this shape: 3 row(s) are written without one, because a checklist row has no id field; importing this file back generates fresh ids (ROADMAP.md)
  dropped: `evidence` has no place in this shape: 1 task(s) carry it: the proof a closed task cites; a ticked box asserts the claim with nothing behind it (ROADMAP.md)
  dropped: `owner` has no place in this shape: 1 task(s) carry it: who holds it (ROADMAP.md)
  dropped: `effort_days` has no place in this shape: 3 task(s) carry it: the estimate the order is derived from (ROADMAP.md)
  placement: 1 task(s) are in no cut and are written before the first heading: a heading here is a cut, and inventing one would assert a grouping the plan does not have. `aplan mv T-003 --cut M-core` files one (ROADMAP.md)
  placement: rows are in id order, which carries no meaning: the plan's own order is derived from what blocks what, and writing that here would reorder the file every time an estimate changed (ROADMAP.md)
wrote ROADMAP.md
```

and `ROADMAP.md` is:

```markdown
- [ ] Write the docs

## Core validator

- [x] Build the core
- [ ] Build the renderer
```

The mapping, and what each choice refuses to guess:

| Plan | Document |
|---|---|
| a cut (a `milestone` task) | a `##` heading, its members as rows |
| a task in no cut | a row above the first heading, because inventing a heading asserts a grouping the plan does not have |
| `status: done` | `- [x]` |
| `todo`, `in_progress`, `blocked` | `- [ ]`, and the count is reported: a checkbox has two states and the plan has five |
| `status: dropped` | nothing, and the count is reported: an empty box reads as outstanding work |
| a release, or a cut whose members are all dropped | nothing: a heading with no rows under it is noise |
| a cancelled cut that still holds live work | no heading, and its members are written loose: `## A cancelled cut` over live rows reads as a live section |
| a title with a line break or a control character | one line, with the character replaced: a row is one line, and the file is opened by people who never ran this tool |

**`--check` is the gate**, the shape `roadmap --format markdown` and
`export-agents-md` already have: it never writes, exits `0` while the file agrees
with the plan, and `1` when it stops agreeing.

**A shape that cannot be written honestly is refused by name**, with the one
field a generated document would have had to invent:

```console
$ aplan export --format openspec
aplan: `openspec` is a shape this reads and does not write: a generated one could not carry `id`, because its `1.1` numbering is a section hierarchy the task model does not hold, so the numbers would be invented and would move whenever a task was added. Run `aplan export --format checklist`, which writes the same rows and invents no ids.
```

`aplan export-roadmap-json` and `aplan export-agents-md` stay separate commands,
and ADR-0032 records why: both *merge* into a file somebody else maintains, so
the target file is an input rather than a destination.

## Adding a flavor

Each source shape is a module in `crates/aplan-interop` implementing
`ProposalSource`, so a second flavor is an addition rather than a refactor, and
ADR-0012's policy has a single enforcement point. Unmodelled fields are collected
by `#[serde(flatten)]` rather than a hand-written list, so a field added upstream
after the code was written is still reported.

A shape that can also be *written* implements `PlanTarget` in the same module, so
the two halves of one format share a file and a change to the mapping is visible
to both. Which shapes go which way is one table, `aplan_interop::FLAVORS`: the
reader validates `--format` against it and the writer dispatches on it, so a
shape cannot be readable on one side and unknown to the other. A row that is
read-only carries the field that stops it being written, which is what keeps
"unsupported" from being the whole answer an adopter gets.
