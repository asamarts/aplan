# ADR-0017: Ordering is derived, and a priority field is deferred

Date: 2026-08-13
Status: Accepted; **superseded by ADR-0025**, which adds the discretionary-order field this
record deferred. The derivation below stands unchanged; the deferral it ends with does not.
Tasks: `A1-124`, `A1-125`, `A1-126`, `A1-127`
Extends ADR-0013's sequencing model. Supersedes nothing.

## Context

A plan of this one's size (133 tasks the day this was written) needs an answer to "what should I do next", and to "do this
before that". Asked for the first time in anger, the tool's answer is thin:

- The task model has **no priority or rank field**.
- `ready` returns tasks in **id order**, so `A1-020` outranks `A1-113` because
  twenty sorts before a hundred and thirteen. That is not an order, it is an
  accident of naming.
- `critical-path` exists and is explicitly advisory, never a gate.
- `graph` renders everything: 285 lines of Mermaid on a 105-task plan, with no
  `--cut`, `--tag` or depth option (pre-public adoption evidence, item 13). That
  finding dispositioned itself as not worth fixing alone, which was right then
  and is not right now.
- **`blocked_by` cannot be edited after `add`.** Checked across every write
  path: it is set at creation and never again. The model's only per-task
  ordering mechanism is unmaintainable through the tool, so changing it means
  hand-editing YAML. That is the unfinished half of pre-public adoption evidence
  item 6, whose other half `A1-077` closed with `drop` and `mv`.

Two things pull in opposite directions.

The request is for a **discretionary** ordering, which the project-management
literature separates cleanly from a mandatory one: mandatory logic must be
satisfied, discretionary logic is a preference that may be overridden. `aplan`
models mandatory logic only.

The tempting shortcut is to express preference as `blocked_by`. **This project
has already measured what that costs.** p00020 items 8 and 10 were both caused
by conflating a sequencing edge with a membership claim, and `A1-113` exists
because `mv` silently destroyed sequencing while `check` called the result
clean. A hard edge used for preference corrupts `ready` (it stops offering the
work), `critical-path` (it lengthens a chain that does not exist), and
`APLAN007` (a `done` task that depends on an unfinished one becomes an error
about nothing).

Pulling the other way, ADR-0013 decided cut sequencing with **"No new field"**,
reusing an edge that already existed. A model this small stays small on purpose.

## Decision

**1. Ordering is derived from the graph, not declared on a task.** `ready`
becomes an ordered answer to "what next", computed from what the plan already
records: cut sequence, how much a task unblocks, whether it lies on the critical
path, and effort. Every input is already checkable, so the order carries no claim
that cannot be verified. A rank field would be an opinion this tool cannot check,
and this project's thesis is that an unverifiable claim in a plan is the thing it
exists to distrust.

The precedence is `A1-125`'s to fix and record, and it is deliberately not fixed
here, because two of those inputs turned out to be empty on this very plan when
they were measured: **no cut in this repository is sequenced**, so ADR-0013's
ordering has nothing to order, and **unblocking power counts milestone
membership**, so 35 of 38 open tasks score identically. An ADR that named a
precedence over inputs that do not discriminate would be deciding on air. What
this ADR decides is that the order is *derived and checkable*; which derivation
is a measurement `A1-125` must make.

**2. The mandatory edges become editable** (`A1-124`). Where one task genuinely
must precede another, that is a `blocked_by` edge, and it must be possible to
say so through the tool rather than in a text editor. This is the largest of the
gaps above and it is not a new concept, only the missing half of an existing
one.

**3. A discretionary edge is deferred, not rejected** (Stage 3, unfiled). A
`prefer_after:` field that ranks without blocking is a coherent design and may
prove necessary. It is deferred because a derived order plus editable
dependencies may make it unnecessary, and the way to find out is use rather than
argument. **The decision point is the dogfooding period in the pre-public repository-boundary decision**: if at the
end of it there is a recorded instance of wanting "X before Y" that neither a
dependency nor the derived order could express, that instance is the evidence an
ADR for the field would need. If there is none, the field was never needed.

**4. Seeing a slice of the graph is part of this, not separate** (`A1-126`).
An order nobody can inspect is an assertion. `tree` and a scoped `graph` are how
a derived order is checked by the person relying on it, which is what makes
p00020 item 13 worth doing now when it was not worth doing alone.

## Consequences

- `ready`'s output order changes, which is a consumer-visible change to a
  command whose output people script against. It needs saying in
  `docs/upgrading.md` and a `--format json` consumer must still get a stable
  order.
- The derived order must be *explainable*, or it is a black box that people
  learn to distrust: `why` already explains blocking, and the same standard
  applies here.
- No task file changes shape, so no schema migration, and an adopter who never
  runs the new commands sees nothing new.
- If the deferred field is eventually added, this ADR is superseded rather than
  edited, and the superseding one carries the recorded instances that justified
  it.
