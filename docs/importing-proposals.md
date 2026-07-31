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
proof). Close it with `aplan close KEP-1287 --evidence commit:<sha>`
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
| `--format <shape>` | source shape; `kep` today |
| `--dry-run` | print what would happen, write nothing |
| `--out <dir>` | where to write (default `<root>/tasks` when it exists) |
| `--phase <n>` | `phase` for imported tasks (default `1`) |
| `--implemented-as todo\|done` | how to read a source's "implemented" (default `todo`) |
| `--as-of <YYYY-MM-DD>` | the note's `captured` date; required with `done` |
| `--cut <milestone>` | place the imported ids in a release cut |

## Adding a flavor

Each source shape is a module in `crates/aplan-interop` implementing
`ProposalSource`, so a second flavor is an addition rather than a refactor, and
ADR-0012's policy has a single enforcement point. Unmodelled fields are collected
by `#[serde(flatten)]` rather than a hand-written list, so a field added upstream
after the code was written is still reported.
