# ADR-0076: A Work Task Integrates Its Decomposed Increments

- **Status:** Accepted
- **Date:** 2026-11-27
- **Task:** `A2-148`
- **Context:** Multi-increment work decomposed under an integration boundary while preserving one
  cut per task (`ADR-0014`), membership as an edge (`ADR-0030`), milestone-only containment
  (`ADR-0042`), and one source of status truth.

## Context

In adopting repositories, a multi-day task frequently delivers across several reviewed increments
while the plan exposes only one flat `in_progress` task. Progress then leaks into free prose
because `aplan` has no first-class work-task decomposition relation:

- `contains` (`ADR-0042`) is strictly milestone-to-milestone checkpoint borrowing; allowing a work
  task to declare `contains` would smuggle in a second grouping hierarchy (`ADR-0014`) and muddle
  pure-borrowing checkpoints with real evidence-gated work.
- `blocked_by` sequences work, so a parent can depend on subtasks, but `blocked_by` alone does not
  distinguish "prerequisite A happened before B" from "B is the integration boundary for bounded
  increments C1, C2, C3".
- Putting the whole effort estimate on both a parent and its children double-counts the cut and the
  plan; forcing a parent that owns no separate integration work to write `effort_days: 0` invents
  zero-effort fiction under `APLAN009`.

Any solution must keep every work task in exactly one cut (or explicitly `cross-cutting`), keep
`status` authoritative with zero duplicated or synthesized statuses, keep `blocked_by` as the sole
blocking-edge contract, and require zero migration for existing plans that do not opt in.

## Decision

### 1. Primitive and edge direction: parent-side `integrates: Vec<String>`

Add an optional top-level list field `integrates` on `Task` (`#[serde(default,
skip_serializing_if = "Vec::is_empty")]`). A non-milestone work task lists the IDs of the direct
work-task increments it integrates:

```yaml
id: A2-148
title: Define a first-class decomposition relation for multi-increment work
status: todo
effort_days: 0.5   # optional own integration effort; may be omitted while integrating children
blocked_by: [A2-148a, A2-148b]
integrates: [A2-148a, A2-148b]
```

The edge points downstream from the parent to its children, matching every other relation field in
the model (`blocked_by`, `prefer_after`, `ratified_by`, `contains`). Milestone `contains` remains
exclusively checkpoint-to-checkpoint borrowing (`ADR-0042`, `APLAN047`).

### 2. Parent role, status, readiness, and single-counted effort

- **Integration-boundary work task:** A task declaring `integrates` is ordinary evidence-gated
  work, not a synthesized rollup and not a milestone. It carries its own explicit `status` and
  closes through `aplan close` with its own proof.
- **Readiness via explicit `blocked_by`:** Every ID in `integrates` must also appear in the
  parent's `blocked_by` (mirroring `ADR-0042` / `APLAN047`). `blocked_by` remains the single
  blocking-edge contract across `ready`, `why`, `critical-path`, `APLAN004`, `APLAN007`, and
  `APLAN020`. A parent becomes `ready` only once every integrated child and every external blocker
  is `done`.
- **Single-counted effort without zero-effort fiction:** Every task's `effort_days` is strictly
  its **own** effort (for a parent, its own integration/assembly/verification effort). Cut,
  roadmap, critical-path, and summary rollups sum every live work task's own `effort_days`
  **exactly once**.
- **Narrow `APLAN009` exemption:** Under `require_effort`, a live non-milestone task whose
  `integrates` list names at least one known non-milestone, non-dropped child increment may omit
  `effort_days` cleanly without firing `APLAN009`. This covers both active decomposition and the
  moment all children finish and the parent sits ready at its integration boundary, eliminating
  zero-effort fiction without exempting ordinary leaf tasks.
- **Derived subtree rollups:** Read projections derive `parent`, `integrates`, `children_total`,
  `children_done`, `remaining_children`, `descendants_total`, `descendants_done`,
  `descendants_remaining`, `current_increment` (active `in_progress` increment, else first `ready`
  descendant in derived `ready` order), `integration_ready`, `children_effort_days`,
  `children_done_effort_days`, and `total_effort_days` (`own + children`). None of these derived
  numbers are persisted in task files.

### 3. Arbitrary acyclic rooted-forest nesting within one cut

- **Rooted forest:** A work task may integrate children that themselves integrate grandchildren to
  arbitrary finite depth, forming a forest of rooted trees. Each work task belongs to at most one
  decomposition parent (single-parent invariant), and cycles through `integrates` fail closed.
- **Explicit same-cut membership:** Every child increment still belongs to the exact same cut
  milestone as its parent (listed explicitly in that milestone's `blocked_by`), or both parent and
  child belong to no cut (`cross-cutting`). This preserves `ADR-0014`, `ADR-0030`, `ADR-0046`, and
  `APLAN022` literally without hidden inherited cut membership.

### 4. Fail-closed validation (`APLAN066`)

New unconditional error rule **`APLAN066`** (`IntegratesUnusable`) fails closed and names every
offending subject when any of the following hold:

1. A `milestone` declares `integrates` (milestones use `contains`).
2. A task lists itself in `integrates`.
3. A task repeats the same child ID in `integrates`.
4. An `integrates` entry names an unknown task ID.
5. An `integrates` entry names a `milestone` instead of a work task.
6. An `integrates` entry is not also listed in the parent's `blocked_by`.
7. More than one parent lists the same child in `integrates` (multi-parent conflict).
8. `integrates` edges form a cycle.
9. Parent and child have different owning cut membership (cross-cut split, including one in a cut
   and one in no cut).
10. A terminal parent violates child lifecycle: a `done` parent with any non-`done` child, or a
    `dropped` parent with any live (`todo`, `in_progress`, `blocked`) child.

A `done` parent whose children are all `done`, or a `dropped` parent whose children are all
terminal (`done` or `dropped`), remains valid historical record.

### 5. Projections, mutations, interop, and opt-in compatibility

- **Consistent projections:** `show`, `why`, `tree`, `ready`, `graph` (Mermaid, DOT, JSON),
  `roadmap` (human, JSON, Markdown), `summary` (human, JSON), `tasks` (human, JSON), MCP tools and
  resources, and the TUI inspector/authoring forms expose `parent`, `integrates`,
  `current_increment`, `remaining_children`, `integration_ready` (integration boundary), and
  own/children/total effort from one shared `Graph` derivation.
- **Mutations and interop:**
  - `add` (CLI `--integrates`, MCP `integrates`, TUI form), `add --from` manifests, and
    `blueprint` templates (`local:<key>` and `task:<id>` resolution) preserve and validate
    `integrates` through the post-state rule gate.
  - `rekey` rewrites `integrates` atomically across task files using a surgical sequence editor.
  - `mv --cut` succeeds when a subtree moves together and fails closed with `APLAN066` on any
    partial cross-cut split.
  - `drop` succeeds when a parent and its live children drop together (or via `drop --cut`) and
    fails closed with `APLAN066` / `APLAN020` when dropping only one side of a live relation.
  - `dep rm` fails closed with `APLAN066` if removing a `blocked_by` edge would orphan an
    `integrates` entry.
  - `supersede` leaves historical `done` parent/child records intact and starts a fresh standalone
    successor without copying `integrates`.
  - `close`, `amend`, `review`, `start`, `stop`, `park`, and `unpark` enforce child completion
    before parent close and preserve `integrates` untouched.
  - Lossy `export --format checklist` explicitly reports `integrates` in its conversion report
    rather than dropping it silently.
- **Zero-migration opt-in:** `integrates` defaults to empty and is omitted on serialization when
  empty; plans that do not use `integrates` require no edits and see unchanged totals and outputs.

## Rejected alternatives

1. **Grouping abuse (sub-cuts / overloading milestones):** Turning decomposed tasks into milestones
   would create a second grouping hierarchy (`ADR-0014`), strip work tasks of evidence/paths/own
   integration effort, and pollute cut rollups with task-level implementation splits.
2. **Overloading `contains` on work tasks:** `contains` (`ADR-0042`) means pure-borrowing
   checkpoint aggregation with no work of its own. Giving `contains` dual semantics on work tasks
   would conflate borrowing rollups with evidence-gated integration work and risk double-counting.
3. **Child-side `part_of` scalar pointer:** Putting the edge on children inverts the downstream
   direction of every other edge in `aplan` (`blocked_by`, `prefer_after`, `ratified_by`,
   `contains`), forces mutating the child file when composing a parent, and lets child files
   silently attach themselves to a terminal parent without touching the parent file.
4. **Arbitrary DAG hyperedges (multi-parent sharing):** Allowing one child to belong to multiple
   parents turns decomposition into an unstructured DAG where "remaining children", "current
   increment", and subtree effort rollups double-count shared nodes across parents.
5. **Markdown checklist parsing in task bodies:** Parsing `- [ ]` bullets as subtasks breaks the
   model's thesis that prose is never a second source of status truth, leaves increments without
   independent evidence, paths, or readiness, and reintroduces unverified status claims.
6. **Synthesized status duplication:** Automatically computing parent `status` from children would
   either fabricate `done` without integration evidence or maintain a second derived status beside
   authoritative task status.
7. **Unchecked tag + dependency convention:** Relying on naming or tags beside `blocked_by` leaves
   cuts, rollups, `show`, `why`, `tree`, `ready`, MCP, and TUI unable to distinguish integration
   boundaries from ordinary prerequisites without parsing conventions or prose.

## Consequences

- Multi-increment work can now be tracked cleanly as first-class, evidence-gated child tasks under
  one integration boundary in the same cut, with deterministic `current_increment`,
  `remaining_children`, and own/subtree effort visibility on every CLI, JSON, MCP, and TUI surface.
- Existing plans require no changes until they opt in, and `PlanStore` bumps its binary cache
  `FORMAT_EPOCH` (`5 -> 6`) to serialize `Task::integrates`.
- Downstream follow-up tasks (`A2-149`, `A2-150`) can build authoring workflows and UX affordances
  directly atop `integrates` and `APLAN066` without re-litigating the data model.
